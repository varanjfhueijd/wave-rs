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
