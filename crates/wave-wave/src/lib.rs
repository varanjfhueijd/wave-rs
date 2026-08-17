//! Implémentation Wave du trait [`wave_core::Provider`].
//!
//! # Configuration
//!
//! Via [`WaveConfig`] directement, ou depuis l'environnement avec
//! [`WaveProvider::from_env`] :
//!
//! - `WAVE_API_KEY` (requis) — clé API Bearer
//! - `WAVE_MERCHANT_ID` (requis) — identifiant marchand
//! - `WAVE_SANDBOX` (optionnel, défaut `true`) — cible l'environnement
//!   sandbox tant qu'il n'est pas explicitement mis à `false`
//!
//! # Contexte métier
//!
//! Les paiements sont asynchrones : [`Provider::initiate_payment`] retourne
//! généralement un statut `pending`, le statut final s'obtient par polling
//! via [`Provider::get_transaction`]. Le timeout HTTP par défaut est de 90s
//! car l'API attend la confirmation de l'utilisateur sur son téléphone.

use std::fmt;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::StatusCode;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use wave_core::{
    Currency, ListOptions, Money, PaymentRequest, PaymentResponse, PhoneNumber, Provider,
    Transaction, TransactionId, TransactionStatus, WaveError,
};

/// URL de base de l'API Wave en production.
pub const DEFAULT_BASE_URL: &str = "https://api.wave.com";
/// URL de base de l'environnement sandbox.
pub const SANDBOX_BASE_URL: &str = "https://api.sandbox.wave.com";

const PROVIDER_NAME: &str = "wave";
const DEFAULT_TIMEOUT_SECS: u64 = 90;

/// Configuration du provider Wave.
#[derive(Clone)]
pub struct WaveConfig {
    pub api_key: String,
    pub merchant_id: String,
    /// URL de base — surchargée en sandbox et dans les tests (wiremock).
    pub base_url: String,
    /// Timeout HTTP global (défaut 90s : confirmation utilisateur lente).
    pub timeout: Duration,
}

impl WaveConfig {
    pub fn new(api_key: impl Into<String>, merchant_id: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            merchant_id: merchant_id.into(),
            base_url: DEFAULT_BASE_URL.to_string(),
            timeout: Duration::from_secs(DEFAULT_TIMEOUT_SECS),
        }
    }

    /// Lit `WAVE_API_KEY`, `WAVE_MERCHANT_ID`, `WAVE_SANDBOX` et
    /// `WAVE_BASE_URL`.
    ///
    /// Tant que `WAVE_SANDBOX` ne vaut pas explicitement `false` (ou `0`),
    /// la sandbox est ciblée — on ne touche jamais la production par accident.
    /// `WAVE_BASE_URL` (optionnel) surcharge l'URL quelle que soit la valeur
    /// de `WAVE_SANDBOX` — pour les mocks locaux et environnements de test.
    pub fn from_env() -> Result<Self, WaveError> {
        let api_key = require_env("WAVE_API_KEY")?;
        let merchant_id = require_env("WAVE_MERCHANT_ID")?;
        let sandbox = std::env::var("WAVE_SANDBOX")
            .map(|v| !matches!(v.trim(), "false" | "0"))
            .unwrap_or(true);
        let mut config = Self::new(api_key, merchant_id);
        if sandbox {
            config.base_url = SANDBOX_BASE_URL.to_string();
        }
        if let Ok(base_url) = std::env::var("WAVE_BASE_URL") {
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

// Debug manuel : la clé API ne doit jamais apparaître dans les logs.
impl fmt::Debug for WaveConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WaveConfig")
            .field("api_key", &"***")
            .field("merchant_id", &self.merchant_id)
            .field("base_url", &self.base_url)
            .field("timeout", &self.timeout)
            .finish()
    }
}

fn require_env(name: &str) -> Result<String, WaveError> {
    // Pas de variante `Config` dans WaveError (hiérarchie figée par la spec) :
    // on signale une config manquante comme une erreur API locale explicite.
    std::env::var(name)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| WaveError::ApiError {
            provider: PROVIDER_NAME.to_string(),
            code: "config".to_string(),
            message: format!("variable d'environnement {name} manquante"),
        })
}

/// Client Wave, implémentation concrète de [`Provider`].
#[derive(Debug)]
pub struct WaveProvider {
    config: WaveConfig,
    http: reqwest::Client,
}

impl WaveProvider {
    pub fn new(config: WaveConfig) -> Result<Self, WaveError> {
        let http = reqwest::Client::builder().timeout(config.timeout).build()?;
        Ok(Self { config, http })
    }

    /// Construit le provider depuis les variables d'environnement.
    pub fn from_env() -> Result<Self, WaveError> {
        Self::new(WaveConfig::from_env()?)
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

    /// Envoie la requête, gère les status non-2xx et désérialise la réponse.
    async fn execute<T: DeserializeOwned>(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<T, WaveError> {
        let response = request
            .bearer_auth(&self.config.api_key)
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
        tracing::trace!(body = %body, "réponse brute Wave");
        Ok(serde_json::from_str(&body)?)
    }
}

/// Traduit une réponse d'erreur HTTP Wave en [`WaveError`].
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
    tracing::trace!(status = %status, body = %body, "réponse d'erreur Wave");

    match serde_json::from_str::<ErrorBody>(&body) {
        Ok(parsed) if parsed.error.code == "insufficient-funds" => WaveError::InsufficientFunds,
        Ok(parsed) => WaveError::ApiError {
            provider: PROVIDER_NAME.to_string(),
            code: parsed.error.code,
            message: parsed.error.message,
        },
        Err(_) => WaveError::ApiError {
            provider: PROVIDER_NAME.to_string(),
            code: status.as_u16().to_string(),
            message: "réponse d'erreur non JSON".to_string(),
        },
    }
}

// ---------------------------------------------------------------------------
// DTOs wire (format JSON de l'API Wave)
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct PayoutBody<'a> {
    merchant_id: &'a str,
    recipient: &'a str,
    amount: u64,
    currency: Currency,
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    client_reference: Option<&'a str>,
}

#[derive(Debug, Deserialize)]
struct PayoutReply {
    id: String,
    status: TransactionStatus,
}

#[derive(Debug, Deserialize)]
struct BalanceReply {
    amount: u64,
    currency: Currency,
}

#[derive(Debug, Deserialize)]
struct TransactionReply {
    id: String,
    status: TransactionStatus,
    amount: u64,
    currency: Currency,
    counterparty: Option<String>,
    note: Option<String>,
    created_at: String,
}

impl TransactionReply {
    fn into_transaction(self) -> Result<Transaction, WaveError> {
        let counterparty = match self.counterparty {
            Some(raw) => Some(PhoneNumber::parse(&raw)?),
            None => None,
        };
        Ok(Transaction {
            id: TransactionId::from(self.id),
            provider: PROVIDER_NAME.to_string(),
            status: self.status,
            amount: Money::new(self.amount, self.currency),
            counterparty,
            note: self.note,
            created_at: self.created_at,
        })
    }
}

#[derive(Debug, Deserialize)]
struct ListReply {
    items: Vec<TransactionReply>,
    #[allow(dead_code)]
    next_cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ErrorBody {
    error: ErrorDetail,
}

#[derive(Debug, Deserialize)]
struct ErrorDetail {
    code: String,
    message: String,
}

// ---------------------------------------------------------------------------
// Implémentation du trait Provider
// ---------------------------------------------------------------------------

#[async_trait]
impl Provider for WaveProvider {
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
        let url = self.url("/v1/payouts");
        tracing::debug!(%url, amount = request.amount.amount, "POST paiement Wave");

        let body = PayoutBody {
            merchant_id: &self.config.merchant_id,
            recipient: request.to.as_str(),
            amount: request.amount.amount,
            currency: request.amount.currency,
            note: request.note.as_deref(),
            client_reference: request.reference.as_deref(),
        };

        let mut http_request = self.http.post(&url).json(&body);
        if let Some(reference) = request.reference.as_deref() {
            http_request = http_request.header("Idempotency-Key", reference);
        }

        let reply: PayoutReply = self.execute(http_request).await?;
        Ok(PaymentResponse {
            transaction_id: TransactionId::from(reply.id),
            status: reply.status,
            provider: PROVIDER_NAME.to_string(),
        })
    }

    async fn check_balance(&self, account: &PhoneNumber) -> Result<Money, WaveError> {
        let url = self.url("/v1/balance");
        tracing::debug!(%url, "GET solde Wave");

        let request = self.http.get(&url).query(&[("account", account.as_str())]);
        let reply: BalanceReply = self.execute(request).await?;
        Ok(Money::new(reply.amount, reply.currency))
    }

    async fn get_transaction(&self, id: &TransactionId) -> Result<Transaction, WaveError> {
        let url = self.url(&format!("/v1/transactions/{id}"));
        tracing::debug!(%url, "GET transaction Wave");

        let reply: TransactionReply = self.execute(self.http.get(&url)).await?;
        reply.into_transaction()
    }

    async fn list_transactions(
        &self,
        account: &PhoneNumber,
        opts: ListOptions,
    ) -> Result<Vec<Transaction>, WaveError> {
        let url = self.url("/v1/transactions");
        tracing::debug!(%url, "GET liste transactions Wave");

        let mut query: Vec<(&str, String)> = vec![("account", account.as_str().to_string())];
        if let Some(limit) = opts.limit {
            query.push(("limit", limit.to_string()));
        }
        if let Some(cursor) = opts.cursor {
            query.push(("cursor", cursor));
        }

        let request = self.http.get(&url).query(&query);
        let reply: ListReply = self.execute(request).await?;
        reply
            .items
            .into_iter()
            .map(TransactionReply::into_transaction)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wave_config_debug_redacts_api_key() {
        let config = WaveConfig::new("clef-tres-secrete", "m-001");
        let output = format!("{config:?}");
        assert!(!output.contains("clef-tres-secrete"));
        assert!(output.contains("***"));
    }
}
