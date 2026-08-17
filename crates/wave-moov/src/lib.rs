//! Implémentation Moov Africa du trait [`wave_core::Provider`].
//!
//! Le plus simple des quatre opérateurs : authentification par clé API
//! statique (header `X-API-Key`), pas de token à renouveler.
//!
//! # Configuration
//!
//! Via [`MoovConfig`] directement, ou depuis l'environnement avec
//! [`MoovProvider::from_env`] :
//!
//! - `MOOV_API_KEY` (requis)
//! - `MOOV_SANDBOX` (optionnel, défaut `true`)
//! - `MOOV_BASE_URL` (optionnel) — surcharge l'URL, pour mocks et tests
//!
//! # Dialecte wire
//!
//! Statuts en minuscules avec `completed` pour un paiement abouti,
//! erreurs `{error_code, error_message}` (`INSUFFICIENT_BALANCE` pour un
//! solde insuffisant) — tout est traduit vers les types unifiés.

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

/// URL de base de l'API Moov Africa en production.
pub const DEFAULT_BASE_URL: &str = "https://api.moov-africa.com";
/// URL de base de l'environnement sandbox.
pub const SANDBOX_BASE_URL: &str = "https://api.sandbox.moov-africa.com";

const PROVIDER_NAME: &str = "moov";
const DEFAULT_TIMEOUT_SECS: u64 = 90;
/// Code d'erreur métier Moov pour un solde insuffisant.
const CODE_INSUFFICIENT_FUNDS: &str = "INSUFFICIENT_BALANCE";

/// Configuration du provider Moov Africa.
#[derive(Clone)]
pub struct MoovConfig {
    pub api_key: String,
    pub base_url: String,
    pub timeout: Duration,
}

impl MoovConfig {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: DEFAULT_BASE_URL.to_string(),
            timeout: Duration::from_secs(DEFAULT_TIMEOUT_SECS),
        }
    }

    /// Lit `MOOV_API_KEY`, `MOOV_SANDBOX` et `MOOV_BASE_URL` (mêmes
    /// conventions que les autres providers : sandbox par défaut,
    /// `MOOV_BASE_URL` prioritaire sur tout).
    pub fn from_env() -> Result<Self, WaveError> {
        let api_key = require_env("MOOV_API_KEY")?;
        let sandbox = std::env::var("MOOV_SANDBOX")
            .map(|v| !matches!(v.trim(), "false" | "0"))
            .unwrap_or(true);
        let mut config = Self::new(api_key);
        if sandbox {
            config.base_url = SANDBOX_BASE_URL.to_string();
        }
        if let Ok(base_url) = std::env::var("MOOV_BASE_URL") {
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
impl fmt::Debug for MoovConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MoovConfig")
            .field("api_key", &"***")
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

/// Client Moov Africa, implémentation concrète de [`Provider`].
#[derive(Debug)]
pub struct MoovProvider {
    config: MoovConfig,
    http: reqwest::Client,
}

impl MoovProvider {
    pub fn new(config: MoovConfig) -> Result<Self, WaveError> {
        let http = reqwest::Client::builder().timeout(config.timeout).build()?;
        Ok(Self { config, http })
    }

    /// Construit le provider depuis les variables d'environnement.
    pub fn from_env() -> Result<Self, WaveError> {
        Self::new(MoovConfig::from_env()?)
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

    /// Envoie la requête authentifiée (`X-API-Key`), gère les status
    /// non-2xx et désérialise la réponse.
    async fn execute<T: DeserializeOwned>(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<T, WaveError> {
        let response = request
            .header("X-API-Key", &self.config.api_key)
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
        tracing::trace!(body = %body, "réponse brute Moov");
        Ok(serde_json::from_str(&body)?)
    }
}

/// Traduit une réponse d'erreur HTTP Moov en [`WaveError`].
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
    tracing::trace!(status = %status, body = %body, "réponse d'erreur Moov");

    match serde_json::from_str::<ErrorBody>(&body) {
        Ok(parsed) if parsed.error_code == CODE_INSUFFICIENT_FUNDS => WaveError::InsufficientFunds,
        Ok(parsed) => WaveError::ApiError {
            provider: PROVIDER_NAME.to_string(),
            code: parsed.error_code,
            message: parsed.error_message,
        },
        Err(_) => WaveError::ApiError {
            provider: PROVIDER_NAME.to_string(),
            code: status.as_u16().to_string(),
            message: "réponse d'erreur non JSON".to_string(),
        },
    }
}

// ---------------------------------------------------------------------------
// DTOs wire (format JSON Moov — statuts minuscules, `completed` = succès,
// erreurs `{error_code, error_message}`)
// ---------------------------------------------------------------------------

/// Statut au format Moov, converti vers le [`TransactionStatus`] unifié.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum MoovStatus {
    Pending,
    Completed,
    Failed,
    Cancelled,
    Expired,
}

impl From<MoovStatus> for TransactionStatus {
    fn from(status: MoovStatus) -> Self {
        match status {
            MoovStatus::Pending => TransactionStatus::Pending,
            MoovStatus::Completed => TransactionStatus::Successful,
            MoovStatus::Failed => TransactionStatus::Failed,
            MoovStatus::Cancelled => TransactionStatus::Cancelled,
            MoovStatus::Expired => TransactionStatus::Expired,
        }
    }
}

#[derive(Debug, Serialize)]
struct PaymentBody<'a> {
    destination: &'a str,
    amount: u64,
    currency: Currency,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    external_reference: Option<&'a str>,
}

#[derive(Debug, Deserialize)]
struct PaymentReply {
    id: String,
    status: MoovStatus,
}

#[derive(Debug, Deserialize)]
struct BalanceReply {
    available: u64,
    currency: Currency,
}

#[derive(Debug, Deserialize)]
struct TransactionReply {
    id: String,
    status: MoovStatus,
    amount: u64,
    currency: Currency,
    destination: Option<String>,
    message: Option<String>,
    created_at: String,
}

impl TransactionReply {
    fn into_transaction(self) -> Result<Transaction, WaveError> {
        let counterparty = match self.destination {
            Some(raw) => Some(PhoneNumber::parse(&raw)?),
            None => None,
        };
        Ok(Transaction {
            id: TransactionId::from(self.id),
            provider: PROVIDER_NAME.to_string(),
            status: self.status.into(),
            amount: Money::new(self.amount, self.currency),
            counterparty,
            note: self.message,
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
    error_code: String,
    error_message: String,
}

// ---------------------------------------------------------------------------
// Implémentation du trait Provider
// ---------------------------------------------------------------------------

#[async_trait]
impl Provider for MoovProvider {
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
        let url = self.url("/api/v1/payments");
        tracing::debug!(%url, amount = request.amount.amount, "POST paiement Moov");

        let body = PaymentBody {
            destination: request.to.as_str(),
            amount: request.amount.amount,
            currency: request.amount.currency,
            message: request.note.as_deref(),
            external_reference: request.reference.as_deref(),
        };

        let mut http_request = self.http.post(&url).json(&body);
        if let Some(reference) = request.reference.as_deref() {
            http_request = http_request.header("Idempotency-Key", reference);
        }

        let reply: PaymentReply = self.execute(http_request).await?;
        Ok(PaymentResponse {
            transaction_id: TransactionId::from(reply.id),
            status: reply.status.into(),
            provider: PROVIDER_NAME.to_string(),
        })
    }

    async fn check_balance(&self, account: &PhoneNumber) -> Result<Money, WaveError> {
        let url = self.url("/api/v1/balance");
        tracing::debug!(%url, "GET solde Moov");

        let request = self.http.get(&url).query(&[("account", account.as_str())]);
        let reply: BalanceReply = self.execute(request).await?;
        Ok(Money::new(reply.available, reply.currency))
    }

    async fn get_transaction(&self, id: &TransactionId) -> Result<Transaction, WaveError> {
        let url = self.url(&format!("/api/v1/transactions/{id}"));
        tracing::debug!(%url, "GET transaction Moov");

        let reply: TransactionReply = self.execute(self.http.get(&url)).await?;
        reply.into_transaction()
    }

    async fn list_transactions(
        &self,
        account: &PhoneNumber,
        opts: ListOptions,
    ) -> Result<Vec<Transaction>, WaveError> {
        let url = self.url("/api/v1/transactions");
        tracing::debug!(%url, "GET liste transactions Moov");

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
    fn test_moov_config_debug_redacts_api_key() {
        let config = MoovConfig::new("cle-moov-secrete");
        let output = format!("{config:?}");
        assert!(!output.contains("cle-moov-secrete"));
        assert!(output.contains("***"));
    }

    #[test]
    fn test_moov_status_completed_maps_to_successful() {
        assert_eq!(
            TransactionStatus::from(MoovStatus::Completed),
            TransactionStatus::Successful
        );
    }
}
