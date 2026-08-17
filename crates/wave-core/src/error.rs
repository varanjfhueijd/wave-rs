//! Hiérarchie d'erreurs du SDK.
//!
//! Toutes les APIs publiques retournent `Result<T, WaveError>` —
//! jamais de `Box<dyn Error>` en surface publique.

/// Erreur unifiée du SDK `wave-rs`.
#[derive(Debug, thiserror::Error)]
pub enum WaveError {
    /// Échec de transport HTTP (DNS, TLS, connexion coupée, ...).
    #[error("Erreur réseau : {0}")]
    Network(#[from] reqwest::Error),

    /// Credentials refusés par l'opérateur (clé API invalide, token expiré, ...).
    #[error("Authentification refusée par {provider}")]
    AuthFailed { provider: String },

    /// Le compte émetteur n'a pas le solde nécessaire.
    #[error("Solde insuffisant")]
    InsufficientFunds,

    /// Le numéro fourni n'est pas un numéro E.164 valide.
    #[error("Numéro de téléphone invalide : {number}")]
    InvalidPhoneNumber { number: String },

    /// Délai dépassé — fréquent : l'utilisateur n'a pas confirmé sur son
    /// téléphone dans le temps imparti (jusqu'à 90s selon l'opérateur).
    #[error("Timeout après {seconds}s")]
    Timeout { seconds: u64 },

    /// Trop de requêtes — respecter `retry_after_secs` avant de réessayer.
    #[error("Rate limit atteint, réessayer après {retry_after_secs}s")]
    RateLimited { retry_after_secs: u64 },

    /// Erreur métier renvoyée par l'API de l'opérateur.
    #[error("Erreur API {provider} [{code}] : {message}")]
    ApiError {
        provider: String,
        code: String,
        message: String,
    },

    /// Corps de requête/réponse JSON malformé.
    #[error("Erreur de sérialisation : {0}")]
    Serialization(#[from] serde_json::Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Transaction;

    #[test]
    fn test_core_error_auth_failed_message() {
        let err = WaveError::AuthFailed {
            provider: "orange".to_string(),
        };
        assert_eq!(err.to_string(), "Authentification refusée par orange");
    }

    #[test]
    fn test_core_error_insufficient_funds_message() {
        assert_eq!(
            WaveError::InsufficientFunds.to_string(),
            "Solde insuffisant"
        );
    }

    #[test]
    fn test_core_error_invalid_phone_number_message() {
        let err = WaveError::InvalidPhoneNumber {
            number: "+225abc".to_string(),
        };
        assert_eq!(err.to_string(), "Numéro de téléphone invalide : +225abc");
    }

    #[test]
    fn test_core_error_timeout_message() {
        let err = WaveError::Timeout { seconds: 90 };
        assert_eq!(err.to_string(), "Timeout après 90s");
    }

    #[test]
    fn test_core_error_rate_limited_message() {
        let err = WaveError::RateLimited {
            retry_after_secs: 30,
        };
        assert_eq!(err.to_string(), "Rate limit atteint, réessayer après 30s");
    }

    #[test]
    fn test_core_error_api_error_message() {
        let err = WaveError::ApiError {
            provider: "mtn".to_string(),
            code: "NOT_ENOUGH_FUNDS".to_string(),
            message: "Solde insuffisant".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "Erreur API mtn [NOT_ENOUGH_FUNDS] : Solde insuffisant"
        );
    }

    /// Le `#[from]` doit convertir une erreur serde_json en
    /// `WaveError::Serialization` — c'est ce qui permet le `?` dans les
    /// providers sur un body malformé.
    #[test]
    fn test_core_error_from_serde_json() {
        let json_err = serde_json::from_str::<Transaction>("{ pas du json }").unwrap_err();
        let err: WaveError = json_err.into();
        assert!(matches!(err, WaveError::Serialization(_)));
        assert!(err.to_string().starts_with("Erreur de sérialisation :"));
    }
}
