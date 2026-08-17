//! # wave-core
//!
//! Fondations du SDK `wave-rs` : types partagés, hiérarchie d'erreurs
//! et le trait [`Provider`], contrat commun à tous les opérateurs de
//! paiement mobile (Wave, Orange Money, MTN MoMo, Moov Africa).
//!
//! Cette crate ne fait aucun appel réseau elle-même : les implémentations
//! concrètes vivent dans `wave-wave`, `wave-orange`, `wave-mtn` et `wave-moov`.

pub mod error;
pub mod provider;
pub mod types;

pub use error::WaveError;
pub use provider::Provider;
pub use types::{
    Currency, ListOptions, Money, PaymentRequest, PaymentResponse, PhoneNumber, Transaction,
    TransactionId, TransactionStatus,
};
