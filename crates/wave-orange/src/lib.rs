//! Implémentation Orange Money CI du trait [`wave_core::Provider`].
//!
//! # Authentification
//!
//! Orange utilise OAuth2 *client credentials* : un `POST /oauth/v3/token`
//! (Basic auth `client_id:client_secret`) retourne un token Bearer à durée
//! de vie limitée. Le token est mis en cache et réutilisé jusqu'à expiration
//! (avec une marge de sécurité) ; un `401` sur un appel API invalide le
//! cache pour forcer une ré-authentification au prochain appel.
//!
//! # Configuration
//!
//! Via [`OrangeConfig`] directement, ou depuis l'environnement avec
//! [`OrangeProvider::from_env`] :
//!
//! - `ORANGE_CLIENT_ID` (requis)
//! - `ORANGE_CLIENT_SECRET` (requis)
//! - `ORANGE_SANDBOX` (optionnel, défaut `true`)
//! - `ORANGE_BASE_URL` (optionnel) — surcharge l'URL, pour mocks et tests

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

/// URL de base de l'API Orange en production.
pub const DEFAULT_BASE_URL: &str = "https://api.orange.com";
/// URL de base de l'environnement sandbox.
pub const SANDBOX_BASE_URL: &str = "https://api.sandbox.orange.com";

const PROVIDER_NAME: &str = "orange";
const DEFAULT_TIMEOUT_SECS: u64 = 90;
/// Marge avant expiration réelle : on renouvelle le token 60s avant.
const TOKEN_EXPIRY_MARGIN_SECS: u64 = 60;
/// Code d'erreur métier Orange pour un solde insuffisant.
const CODE_INSUFFICIENT_FUNDS: &str = "60019";

/// Configuration du provider Orange Money.
#[derive(Clone)]
pub struct OrangeConfig {
    pub client_id: String,
    pub client_secret: String,
    pub base_url: String,
    pub timeout: Duration,
}

impl OrangeConfig {
    pub fn new(client_id: impl Into<String>, client_secret: impl Into<String>) -> Self {
        Self {
            client_id: client_id.into(),
            client_secret: client_secret.into(),
            base_url: DEFAULT_BASE_URL.to_string(),
            timeout: Duration::from_secs(DEFAULT_TIMEOUT_SECS),
        }
    }

    /// Lit `ORANGE_CLIENT_ID`, `ORANGE_CLIENT_SECRET`, `ORANGE_SANDBOX`
    /// et `ORANGE_BASE_URL` (mêmes conventions que le provider Wave :
    /// sandbox par défaut, `ORANGE_BASE_URL` prioritaire sur tout).
    pub fn from_env() -> Result<Self, WaveError> {
        let client_id = require_env("ORANGE_CLIENT_ID")?;
        let client_secret = require_env("ORANGE_CLIENT_SECRET")?;
        let sandbox = std::env::var("ORANGE_SANDBOX")
            .map(|v| !matches!(v.trim(), "false" | "0"))
            .unwrap_or(true);
        let mut config = Self::new(client_id, client_secret);
        if sandbox {
            config.base_url = SANDBOX_BASE_URL.to_string();
        }
        if let Ok(base_url) = std::env::var("ORANGE_BASE_URL") {
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

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

// Debug manuel : le secret ne doit jamais apparaître dans les logs.
impl fmt::Debug for OrangeConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OrangeConfig")
            .field("client_id", &self.client_id)
            .field("client_secret", &"***")
            .field("base_url", &self.base_url)
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

/// Token OAuth2 mis en cache, avec son instant d'expiration effective
/// (marge de sécurité déjà déduite).
struct CachedToken {
    access_token: String,
    expires_at: Instant,
}

/// Client Orange Money, implémentation concrète de [`Provider`].
pub struct OrangeProvider {
    config: OrangeConfig,
    http: reqwest::Client,
    token: Mutex<Option<CachedToken>>,
}

impl fmt::Debug for OrangeProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OrangeProvider")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl OrangeProvider {
    pub fn new(config: OrangeConfig) -> Result<Self, WaveError> {
        let http = reqwest::Client::builder().timeout(config.timeout).build()?;
        Ok(Self {
            config,
            http,
            token: Mutex::new(None),
        })
    }

    /// Construit le provider depuis les variables d'environnement.
    pub fn from_env() -> Result<Self, WaveError> {
        Self::new(OrangeConfig::from_env()?)
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

    /// Retourne un token valide, en le récupérant auprès d'Orange si le
    /// cache est vide ou expiré.
    async fn access_token(&self) -> Result<String, WaveError> {
        let mut guard = self.token.lock().await;
        if let Some(cached) = guard.as_ref() {
            if cached.expires_at > Instant::now() {
                return Ok(cached.access_token.clone());
            }
        }

        let url = self.url("/oauth/v3/token");
        tracing::debug!(%url, "récupération d'un token OAuth2 Orange");
        let response = self
            .http
            .post(&url)
            .basic_auth(&self.config.client_id, Some(&self.config.client_secret))
            .form(&[("grant_type", "client_credentials")])
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

    /// Envoie la requête authentifiée, gère les status non-2xx et
    /// désérialise la réponse. Un `401` invalide le token en cache.
    async fn execute<T: DeserializeOwned>(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<T, WaveError> {
        let token = self.access_token().await?;
        let response = request
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| self.map_transport_error(e))?;

        if response.status() == StatusCode::UNAUTHORIZED {
            // Token révoqué côté Orange : on purge le cache pour que le
            // prochain appel se ré-authentifie proprement.
            *self.token.lock().await = None;
        }

        if !response.status().is_success() {
            return Err(error_from_response(response).await);
        }

        let body = response
            .text()
            .await
            .map_err(|e| self.map_transport_error(e))?;
        tracing::trace!(body = %body, "réponse brute Orange");
        Ok(serde_json::from_str(&body)?)
    }
}

/// Traduit une réponse d'erreur HTTP Orange en [`WaveError`].
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
    tracing::trace!(status = %status, body = %body, "réponse d'erreur Orange");

    match serde_json::from_str::<ErrorBody>(&body) {
        Ok(parsed) if parsed.code == CODE_INSUFFICIENT_FUNDS => WaveError::InsufficientFunds,
        Ok(parsed) => WaveError::ApiError {
            provider: PROVIDER_NAME.to_string(),
            code: parsed.code,
            message: parsed.description,
        },
        Err(_) => WaveError::ApiError {
            provider: PROVIDER_NAME.to_string(),
            code: status.as_u16().to_string(),
            message: "réponse d'erreur non JSON".to_string(),
        },
    }
}

// ---------------------------------------------------------------------------
// DTOs wire (format JSON de l'API Orange — statuts en MAJUSCULES,
// erreurs `{code, description}` avec codes numériques)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct TokenReply {
    access_token: String,
    expires_in: u64,
}

/// Statut au format Orange, converti vers le [`TransactionStatus`] unifié.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
enum OrangeStatus {
    Pending,
    Success,
    Failed,
    Cancelled,
    Expired,
}

impl From<OrangeStatus> for TransactionStatus {
    fn from(status: OrangeStatus) -> Self {
        match status {
            OrangeStatus::Pending => TransactionStatus::Pending,
            OrangeStatus::Success => TransactionStatus::Successful,
            OrangeStatus::Failed => TransactionStatus::Failed,
            OrangeStatus::Cancelled => TransactionStatus::Cancelled,
            OrangeStatus::Expired => TransactionStatus::Expired,
        }
    }
}

#[derive(Debug, Serialize)]
struct PaymentBody<'a> {
    receiver_msisdn: &'a str,
    amount: u64,
    currency: Currency,
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reference: Option<&'a str>,
}

#[derive(Debug, Deserialize)]
struct PaymentReply {
    transaction_id: String,
    status: OrangeStatus,
}

#[derive(Debug, Deserialize)]
struct BalanceReply {
    balance: u64,
    currency: Currency,
}

#[derive(Debug, Deserialize)]
struct TransactionReply {
    transaction_id: String,
    status: OrangeStatus,
    amount: u64,
    currency: Currency,
    peer_msisdn: Option<String>,
    note: Option<String>,
    created_at: String,
}

impl TransactionReply {
    fn into_transaction(self) -> Result<Transaction, WaveError> {
        let counterparty = match self.peer_msisdn {
            Some(raw) => Some(PhoneNumber::parse(&raw)?),
            None => None,
        };
        Ok(Transaction {
            id: TransactionId::from(self.transaction_id),
            provider: PROVIDER_NAME.to_string(),
            status: self.status.into(),
            amount: Money::new(self.amount, self.currency),
            counterparty,
            note: self.note,
            created_at: self.created_at,
        })
    }
}

#[derive(Debug, Deserialize)]
struct ListReply {
    transactions: Vec<TransactionReply>,
    #[allow(dead_code)]
    next_cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ErrorBody {
    code: String,
    description: String,
}

// ---------------------------------------------------------------------------
// Implémentation du trait Provider
// ---------------------------------------------------------------------------

#[async_trait]
impl Provider for OrangeProvider {
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
        let url = self.url("/omoney/v1/payments");
        tracing::debug!(%url, amount = request.amount.amount, "POST paiement Orange");

        let body = PaymentBody {
            receiver_msisdn: request.to.as_str(),
            amount: request.amount.amount,
            currency: request.amount.currency,
            note: request.note.as_deref(),
            reference: request.reference.as_deref(),
        };

        let mut http_request = self.http.post(&url).json(&body);
        if let Some(reference) = request.reference.as_deref() {
            http_request = http_request.header("X-Reference-Id", reference);
        }

        let reply: PaymentReply = self.execute(http_request).await?;
        Ok(PaymentResponse {
            transaction_id: TransactionId::from(reply.transaction_id),
            status: reply.status.into(),
            provider: PROVIDER_NAME.to_string(),
        })
    }

    async fn check_balance(&self, account: &PhoneNumber) -> Result<Money, WaveError> {
        let url = self.url("/omoney/v1/balance");
        tracing::debug!(%url, "GET solde Orange");

        let request = self.http.get(&url).query(&[("msisdn", account.as_str())]);
        let reply: BalanceReply = self.execute(request).await?;
        Ok(Money::new(reply.balance, reply.currency))
    }

    async fn get_transaction(&self, id: &TransactionId) -> Result<Transaction, WaveError> {
        let url = self.url(&format!("/omoney/v1/transactions/{id}"));
        tracing::debug!(%url, "GET transaction Orange");

        let reply: TransactionReply = self.execute(self.http.get(&url)).await?;
        reply.into_transaction()
    }

    async fn list_transactions(
        &self,
        account: &PhoneNumber,
        opts: ListOptions,
    ) -> Result<Vec<Transaction>, WaveError> {
        let url = self.url("/omoney/v1/transactions");
        tracing::debug!(%url, "GET liste transactions Orange");

        let mut query: Vec<(&str, String)> = vec![("msisdn", account.as_str().to_string())];
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
            .map(TransactionReply::into_transaction)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_orange_config_debug_redacts_client_secret() {
        let config = OrangeConfig::new("client-001", "secret-tres-confidentiel");
        let output = format!("{config:?}");
        assert!(!output.contains("secret-tres-confidentiel"));
        assert!(output.contains("***"));
    }

    #[test]
    fn test_orange_status_mapping() {
        assert_eq!(
            TransactionStatus::from(OrangeStatus::Success),
            TransactionStatus::Successful
        );
        assert_eq!(
            TransactionStatus::from(OrangeStatus::Pending),
            TransactionStatus::Pending
        );
    }
}
