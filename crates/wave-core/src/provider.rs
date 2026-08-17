//! Le trait [`Provider`] — contrat central du SDK.
//!
//! Toute implémentation d'opérateur (Wave, Orange Money, MTN MoMo,
//! Moov Africa) DOIT implémenter ce trait exactement. Sa signature ne
//! change jamais sans validation explicite du mainteneur.

use async_trait::async_trait;

use crate::error::WaveError;
use crate::types::{
    Currency, ListOptions, Money, PaymentRequest, PaymentResponse, PhoneNumber, Transaction,
    TransactionId,
};

/// Interface commune, typée et async, à tous les opérateurs de paiement
/// mobile supportés.
///
/// # Contexte métier
///
/// Les APIs mobile money ouest-africaines sont asynchrones : un paiement
/// initié est généralement `Pending` jusqu'à confirmation de l'utilisateur
/// sur son téléphone (timeouts jusqu'à 90s). Les implémentations ne doivent
/// jamais bloquer : tout passe par `async/await` sur Tokio.
#[async_trait]
pub trait Provider: Send + Sync {
    /// Nom court et stable du provider (`"wave"`, `"orange"`, `"mtn"`, `"moov"`).
    fn name(&self) -> &'static str;

    /// Devise dans laquelle ce provider opère (XOF pour la zone UEMOA).
    fn currency(&self) -> Currency;

    /// Initie un paiement vers `request.to`.
    ///
    /// Retourne généralement une réponse au statut `Pending` : le statut
    /// final s'obtient via [`get_transaction`](Provider::get_transaction)
    /// (polling) ou un webhook côté opérateur.
    async fn initiate_payment(&self, request: PaymentRequest)
        -> Result<PaymentResponse, WaveError>;

    /// Consulte le solde du compte `account`.
    async fn check_balance(&self, account: &PhoneNumber) -> Result<Money, WaveError>;

    /// Récupère une transaction par son identifiant opérateur.
    async fn get_transaction(&self, id: &TransactionId) -> Result<Transaction, WaveError>;

    /// Liste les transactions du compte `account`, paginées via `opts`.
    async fn list_transactions(
        &self,
        account: &PhoneNumber,
        opts: ListOptions,
    ) -> Result<Vec<Transaction>, WaveError>;
}
