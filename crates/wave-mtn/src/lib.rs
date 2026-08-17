//! Implémentation MTN Mobile Money du trait [`wave_core::Provider`].
//!
//! # Particularités de l'API MoMo
//!
//! - Chaque appel porte la clé de souscription (`Ocp-Apim-Subscription-Key`)
//!   et l'environnement cible (`X-Target-Environment`), en plus du token
//!   Bearer obtenu par Basic auth `api_user:api_key`.
//! - Le paiement est **en deux temps** : le client génère lui-même un
//!   identifiant de référence (UUID v4) envoyé en `X-Reference-Id`, l'API
//!   répond `202 Accepted` **sans corps**, et le statut se consulte ensuite
//!   par polling sur cette référence — c'est elle qui devient le
//!   `transaction_id` retourné.
//! - Les montants sont des **chaînes** (`"7500"`) et les numéros sont au
//!   format MSISDN **sans** `+` (`2250700000000`) : tout est normalisé vers
//!   les types unifiés de `wave-core`.
//!
//! # Configuration
//!
//! - `MTN_SUBSCRIPTION_KEY` (requis)
//! - `MTN_API_USER` / `MTN_API_KEY` (requis)
//! - `MTN_SANDBOX` (optionnel, défaut `true`)
//! - `MTN_BASE_URL` (optionnel) — surcharge l'URL, pour mocks et tests

use std::fmt;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use reqwest::StatusCode;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use wave_core::{
    Currency, ListOptions, Money, PaymentRequest, PaymentResponse, PhoneNumber, Provider,
    Transaction, TransactionId, TransactionStatus, WaveError,
};

/// URL de base de l'API MoMo en production.
pub const DEFAULT_BASE_URL: &str = "https://proxy.momoapi.mtn.com";
/// URL de base du sandbox public MTN MoMo.
pub const SANDBOX_BASE_URL: &str = "https://sandbox.momodeveloper.mtn.com";

const PROVIDER_NAME: &str = "mtn";
const DEFAULT_TIMEOUT_SECS: u64 = 90;
const TOKEN_EXPIRY_MARGIN_SECS: u64 = 60;
/// Code d'erreur métier MoMo pour un solde insuffisant.
const CODE_INSUFFICIENT_FUNDS: &str = "NOT_ENOUGH_FUNDS";

/// Configuration du provider MTN MoMo.
#[derive(Clone)]
pub struct MtnConfig {
    pub subscription_key: String,
    pub api_user: String,
    pub api_key: String,
    pub base_url: String,
    /// Valeur du header `X-Target-Environment` (`sandbox` ou nom du
    /// marché en production, ex. `mtncotedivoire`).
    pub target_environment: String,
    pub timeout: Duration,
}

impl MtnConfig {
    pub fn new(
        subscription_key: impl Into<String>,
        api_user: impl Into<String>,
        api_key: impl Into<String>,
    ) -> Self {
        Self {
            subscription_key: subscription_key.into(),
            api_user: api_user.into(),
            api_key: api_key.into(),
            base_url: DEFAULT_BASE_URL.to_string(),
            target_environment: "mtncotedivoire".to_string(),
            timeout: Duration::from_secs(DEFAULT_TIMEOUT_SECS),
        }
    }

    /// Lit `MTN_SUBSCRIPTION_KEY`, `MTN_API_USER`, `MTN_API_KEY`,
    /// `MTN_SANDBOX` et `MTN_BASE_URL` (mêmes conventions que les autres
    /// providers : sandbox par défaut, `MTN_BASE_URL` prioritaire sur tout).
    pub fn from_env() -> Result<Self, WaveError> {
        let subscription_key = require_env("MTN_SUBSCRIPTION_KEY")?;
        let api_user = require_env("MTN_API_USER")?;
        let api_key = require_env("MTN_API_KEY")?;
        let sandbox = std::env::var("MTN_SANDBOX")
            .map(|v| !matches!(v.trim(), "false" | "0"))
            .unwrap_or(true);
        let mut config = Self::new(subscription_key, api_user, api_key);
        if sandbox {
            config.base_url = SANDBOX_BASE_URL.to_string();
            config.target_environment = "sandbox".to_string();
        }
        if let Ok(base_url) = std::env::var("MTN_BASE_URL") {
            if !base_url.trim().is_empty() {
                config.base_url = base_url.trim().trim_end_matches('/').to_string();
            }
        }
        Ok(config)
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    pub fn with_target_environment(mut self, environment: impl Into<String>) -> Self {
        self.target_environment = environment.into();
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

// Debug manuel : ni la clé API ni la clé de souscription dans les logs.
impl fmt::Debug for MtnConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MtnConfig")
            .field("subscription_key", &"***")
            .field("api_user", &self.api_user)
            .field("api_key", &"***")
            .field("base_url", &self.base_url)
            .field("target_environment", &self.target_environment)
            .field("timeout", &self.timeout)
            .finish()
    }
}

fn require_env(name: &str) -> Result<String, WaveError> {
    std::env::var(name)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| WaveError::ApiError {
            provider: PROVIDER_NAME.to_string(),
            code: "config".to_string(),
            message: format!("variable d'environnement {name} manquante"),
        })
}

/// Génère un UUID v4 (format exigé par MoMo pour `X-Reference-Id`) à partir
/// de l'entropie du système via `RandomState` — sans crate externe.
///
/// Suffisant pour des références de transaction uniques ; si un besoin
/// cryptographique apparaît, proposer le crate `uuid` au mainteneur.
fn generate_reference_id() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    use std::time::{SystemTime, UNIX_EPOCH};

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);

    let mut bytes = [0u8; 16];
    for (i, chunk) in bytes.chunks_mut(8).enumerate() {
        let mut hasher = RandomState::new().build_hasher();
        hasher.write_u128(nanos);
        hasher.write_usize(i);
        chunk.copy_from_slice(&hasher.finish().to_be_bytes());
    }
    // Bits de version (4) et de variante (RFC 4122).
    bytes[6] = (bytes[6] & 0x0F) | 0x40;
    bytes[8] = (bytes[8] & 0x3F) | 0x80;

    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

/// Convertit un montant MoMo (chaîne) vers un entier de francs.
fn parse_amount(raw: &str) -> Result<u64, WaveError> {
    raw.trim().parse().map_err(|_| WaveError::ApiError {
        provider: PROVIDER_NAME.to_string(),
        code: "invalid-amount".to_string(),
        message: format!("montant non numérique retourné par l'API : '{raw}'"),
    })
}

/// Normalise un MSISDN MoMo (sans `+`) vers un [`PhoneNumber`] E.164.
fn parse_msisdn(raw: &str) -> Result<PhoneNumber, WaveError> {
    let normalized = if raw.starts_with('+') {
        raw.to_string()
    } else {
        format!("+{raw}")
    };
    PhoneNumber::parse(&normalized)
}

struct CachedToken {
    access_token: String,
    expires_at: Instant,
}

/// Client MTN MoMo, implémentation concrète de [`Provider`].
pub struct MtnProvider {
    config: MtnConfig,
    http: reqwest::Client,
    token: Mutex<Option<CachedToken>>,
}

impl fmt::Debug for MtnProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MtnProvider")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl MtnProvider {
    pub fn new(config: MtnConfig) -> Result<Self, WaveError> {
        let http = reqwest::Client::builder().timeout(config.timeout).build()?;
        Ok(Self {
            config,
            http,
            token: Mutex::new(None),
        })
    }

    /// Construit le provider depuis les variables d'environnement.
    pub fn from_env() -> Result<Self, WaveError> {
        Self::new(MtnConfig::from_env()?)
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.config.base_url)
    }

    fn map_transport_error(&self, err: reqwest::Error) -> WaveError {
        if err.is_timeout() {
            WaveError::Timeout {
                seconds: self.config.timeout.as_secs(),
            }
        } else {
            WaveError::Network(err)
        }
    }

    /// Retourne un token valide, en le récupérant si le cache est vide ou
    /// expiré (mêmes règles que le provider Orange).
    async fn access_token(&self) -> Result<String, WaveError> {
        let mut guard = self.token.lock().await;
        if let Some(cached) = guard.as_ref() {
            if cached.expires_at > Instant::now() {
                return Ok(cached.access_token.clone());
            }
        }

        let url = self.url("/disbursement/token/");
        tracing::debug!(%url, "récupération d'un token MoMo");
        let response = self
            .http
            .post(&url)
            .basic_auth(&self.config.api_user, Some(&self.config.api_key))
            .header("Ocp-Apim-Subscription-Key", &self.config.subscription_key)
            .send()
            .await
            .map_err(|e| self.map_transport_error(e))?;

        if !response.status().is_success() {
            return Err(error_from_response(response).await);
        }

        let body = response
            .text()
            .await
            .map_err(|e| self.map_transport_error(e))?;
        let reply: TokenReply = serde_json::from_str(&body)?;

        let lifetime = reply.expires_in.saturating_sub(TOKEN_EXPIRY_MARGIN_SECS);
        let access_token = reply.access_token;
        *guard = Some(CachedToken {
            access_token: access_token.clone(),
            expires_at: Instant::now() + Duration::from_secs(lifetime),
        });
        Ok(access_token)
    }

    /// Ajoute l'authentification complète MoMo (Bearer + souscription +
    /// environnement cible) puis envoie la requête.
    async fn send_authenticated(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<reqwest::Response, WaveError> {
        let token = self.access_token().await?;
        let response = request
            .bearer_auth(&token)
            .header("Ocp-Apim-Subscription-Key", &self.config.subscription_key)
            .header("X-Target-Environment", &self.config.target_environment)
            .send()
            .await
            .map_err(|e| self.map_transport_error(e))?;

        if response.status() == StatusCode::UNAUTHORIZED {
            *self.token.lock().await = None;
        }

        Ok(response)
    }

    /// Envoie et désérialise une réponse JSON.
    async fn execute<T: DeserializeOwned>(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<T, WaveError> {
        let response = self.send_authenticated(request).await?;
        if !response.status().is_success() {
            return Err(error_from_response(response).await);
        }
        let body = response
            .text()
            .await
            .map_err(|e| self.map_transport_error(e))?;
        tracing::trace!(body = %body, "réponse brute MoMo");
        Ok(serde_json::from_str(&body)?)
    }

    /// Envoie une requête dont le succès est un statut sans corps
    /// (`202 Accepted` du transfert).
    async fn execute_no_content(&self, request: reqwest::RequestBuilder) -> Result<(), WaveError> {
        let response = self.send_authenticated(request).await?;
        if !response.status().is_success() {
            return Err(error_from_response(response).await);
        }
        Ok(())
    }
}

/// Traduit une réponse d'erreur HTTP MoMo en [`WaveError`].
async fn error_from_response(response: reqwest::Response) -> WaveError {
    let status = response.status();

    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        return WaveError::AuthFailed {
            provider: PROVIDER_NAME.to_string(),
        };
    }

    if status == StatusCode::TOO_MANY_REQUESTS {
        let retry_after_secs = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse().ok())
            .unwrap_or(60);
        return WaveError::RateLimited { retry_after_secs };
    }

    let body = response.text().await.unwrap_or_default();
    tracing::trace!(status = %status, body = %body, "réponse d'erreur MoMo");

    match serde_json::from_str::<ErrorBody>(&body) {
        Ok(parsed) if parsed.code == CODE_INSUFFICIENT_FUNDS => WaveError::InsufficientFunds,
        Ok(parsed) => WaveError::ApiError {
            provider: PROVIDER_NAME.to_string(),
            code: parsed.code,
            message: parsed.message,
        },
        Err(_) => WaveError::ApiError {
            provider: PROVIDER_NAME.to_string(),
            code: status.as_u16().to_string(),
            message: "réponse d'erreur non JSON".to_string(),
        },
    }
}

// ---------------------------------------------------------------------------
// DTOs wire (format JSON MoMo — camelCase, montants en chaînes,
// MSISDN sans `+`, statuts MAJUSCULES)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct TokenReply {
    access_token: String,
    expires_in: u64,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
enum MomoStatus {
    Pending,
    Successful,
    Failed,
}

impl From<MomoStatus> for TransactionStatus {
    fn from(status: MomoStatus) -> Self {
        match status {
            MomoStatus::Pending => TransactionStatus::Pending,
            MomoStatus::Successful => TransactionStatus::Successful,
            MomoStatus::Failed => TransactionStatus::Failed,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Party<'a> {
    party_id_type: &'static str,
    party_id: &'a str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TransferBody<'a> {
    amount: String,
    currency: Currency,
    #[serde(skip_serializing_if = "Option::is_none")]
    external_id: Option<&'a str>,
    payee: Party<'a>,
    #[serde(skip_serializing_if = "Option::is_none")]
    payee_note: Option<&'a str>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BalanceReply {
    available_balance: String,
    currency: Currency,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PartyReply {
    party_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TransferReply {
    amount: String,
    currency: Currency,
    status: MomoStatus,
    payee: Option<PartyReply>,
    payee_note: Option<String>,
    #[serde(default)]
    created_at: Option<String>,
}

impl TransferReply {
    fn into_transaction(self, id: TransactionId) -> Result<Transaction, WaveError> {
        let counterparty = match self.payee {
            Some(party) => Some(parse_msisdn(&party.party_id)?),
            None => None,
        };
        Ok(Transaction {
            id,
            provider: PROVIDER_NAME.to_string(),
            status: self.status.into(),
            amount: Money::new(parse_amount(&self.amount)?, self.currency),
            counterparty,
            note: self.payee_note,
            created_at: self.created_at.unwrap_or_default(),
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListItemReply {
    reference_id: String,
    #[serde(flatten)]
    transfer: TransferReply,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListReply {
    transactions: Vec<ListItemReply>,
}

#[derive(Debug, Deserialize)]
struct ErrorBody {
    code: String,
    message: String,
}

// ---------------------------------------------------------------------------
// Implémentation du trait Provider
// ---------------------------------------------------------------------------

#[async_trait]
impl Provider for MtnProvider {
    fn name(&self) -> &'static str {
        PROVIDER_NAME
    }

    fn currency(&self) -> Currency {
        Currency::XOF
    }

    async fn initiate_payment(
        &self,
        request: PaymentRequest,
    ) -> Result<PaymentResponse, WaveError> {
        let reference_id = generate_reference_id();
        let url = self.url("/disbursement/v1_0/transfer");
        tracing::debug!(%url, %reference_id, amount = request.amount.amount, "POST transfert MoMo");

        let msisdn = request.to.as_str().trim_start_matches('+');
        let body = TransferBody {
            amount: request.amount.amount.to_string(),
            currency: request.amount.currency,
            external_id: request.reference.as_deref(),
            payee: Party {
                party_id_type: "MSISDN",
                party_id: msisdn,
            },
            payee_note: request.note.as_deref(),
        };

        let http_request = self
            .http
            .post(&url)
            .header("X-Reference-Id", &reference_id)
            .json(&body);

        // 202 Accepted sans corps : la référence générée côté client EST
        // l'identifiant de suivi de la transaction.
        self.execute_no_content(http_request).await?;
        Ok(PaymentResponse {
            transaction_id: TransactionId::from(reference_id),
            status: TransactionStatus::Pending,
            provider: PROVIDER_NAME.to_string(),
        })
    }

    async fn check_balance(&self, account: &PhoneNumber) -> Result<Money, WaveError> {
        let url = self.url("/disbursement/v1_0/account/balance");
        tracing::debug!(%url, "GET solde MoMo");

        let msisdn = account.as_str().trim_start_matches('+').to_string();
        let request = self.http.get(&url).query(&[("msisdn", msisdn)]);
        let reply: BalanceReply = self.execute(request).await?;
        Ok(Money::new(
            parse_amount(&reply.available_balance)?,
            reply.currency,
        ))
    }

    async fn get_transaction(&self, id: &TransactionId) -> Result<Transaction, WaveError> {
        let url = self.url(&format!("/disbursement/v1_0/transfer/{id}"));
        tracing::debug!(%url, "GET transfert MoMo");

        let reply: TransferReply = self.execute(self.http.get(&url)).await?;
        reply.into_transaction(id.clone())
    }

    async fn list_transactions(
        &self,
        account: &PhoneNumber,
        opts: ListOptions,
    ) -> Result<Vec<Transaction>, WaveError> {
        let url = self.url("/disbursement/v1_0/transactions");
        tracing::debug!(%url, "GET liste transferts MoMo");

        let msisdn = account.as_str().trim_start_matches('+').to_string();
        let mut query: Vec<(&str, String)> = vec![("msisdn", msisdn)];
        if let Some(limit) = opts.limit {
            query.push(("limit", limit.to_string()));
        }
        if let Some(cursor) = opts.cursor {
            query.push(("cursor", cursor));
        }

        let request = self.http.get(&url).query(&query);
        let reply: ListReply = self.execute(request).await?;
        reply
            .transactions
            .into_iter()
            .map(|item| {
                item.transfer
                    .into_transaction(TransactionId::from(item.reference_id))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mtn_config_debug_redacts_secrets() {
        let config = MtnConfig::new("sub-secrete", "user-001", "cle-secrete");
        let output = format!("{config:?}");
        assert!(!output.contains("sub-secrete"));
        assert!(!output.contains("cle-secrete"));
        assert!(output.contains("user-001"));
    }

    #[test]
    fn test_mtn_reference_id_is_uuid_v4_shaped() {
        let id = generate_reference_id();
        assert_eq!(id.len(), 36);
        let dashes: Vec<usize> = id
            .char_indices()
            .filter(|(_, c)| *c == '-')
            .map(|(i, _)| i)
            .collect();
        assert_eq!(dashes, vec![8, 13, 18, 23]);
        assert_eq!(&id[14..15], "4", "bits de version v4");
        assert!(
            matches!(&id[19..20], "8" | "9" | "a" | "b"),
            "bits de variante RFC 4122"
        );
    }

    #[test]
    fn test_mtn_reference_ids_are_unique() {
        let a = generate_reference_id();
        let b = generate_reference_id();
        assert_ne!(a, b);
    }

    #[test]
    fn test_mtn_parse_msisdn_adds_prefix() {
        let phone = parse_msisdn("2250707070707").unwrap();
        assert_eq!(phone.as_str(), "+2250707070707");
    }

    #[test]
    fn test_mtn_parse_amount_rejects_garbage() {
        assert!(parse_amount("7500").is_ok());
        assert!(matches!(
            parse_amount("75.50"),
            Err(WaveError::ApiError { .. })
        ));
    }
}
