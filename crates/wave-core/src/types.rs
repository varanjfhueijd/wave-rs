//! Types partagés du SDK : monnaie, numéros de téléphone, transactions.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::error::WaveError;

/// Devise supportée par le SDK.
///
/// Les quatre opérateurs ciblés opèrent dans la zone UEMOA (franc CFA).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Currency {
    /// Franc CFA BCEAO (zone UEMOA : CI, SN, BJ, TG, ML, BF, NE, GW).
    XOF,
}

impl fmt::Display for Currency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Currency::XOF => f.write_str("XOF"),
        }
    }
}

/// Montant monétaire en unités entières (francs CFA entiers).
///
/// Jamais de `f64` pour l'argent : le XOF n'a pas de subdivision en
/// circulation, `amount` est donc un nombre entier de francs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Money {
    pub amount: u64,
    pub currency: Currency,
}

impl Money {
    pub fn new(amount: u64, currency: Currency) -> Self {
        Self { amount, currency }
    }

    /// Raccourci pour un montant en francs CFA.
    pub fn xof(amount: u64) -> Self {
        Self::new(amount, Currency::XOF)
    }
}

impl fmt::Display for Money {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.amount, self.currency)
    }
}

/// Numéro de téléphone validé et normalisé en E.164 (ex. `+2250707070707`).
///
/// Invariant : il est impossible de construire un `PhoneNumber` sans passer
/// par la validation `phonenumber` — un numéro non validé n'existe pas
/// dans ce type.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PhoneNumber(String);

impl PhoneNumber {
    /// Parse un numéro déjà en format international (`+225...`, `+221...`).
    pub fn parse(input: &str) -> Result<Self, WaveError> {
        Self::parse_inner(None, input)
    }

    /// Parse un numéro local court (ex. `07 07 07 07 07`) en fournissant
    /// le code pays ISO 3166-1 alpha-2 (`"CI"`, `"SN"`, ...).
    ///
    /// C'est le point d'entrée pour normaliser les formats courts locaux
    /// en E.164 avant tout appel API.
    pub fn parse_with_region(region: &str, input: &str) -> Result<Self, WaveError> {
        let region_id = phonenumber::country::Id::from_str(region).map_err(|_| {
            WaveError::InvalidPhoneNumber {
                number: input.to_string(),
            }
        })?;
        Self::parse_inner(Some(region_id), input)
    }

    fn parse_inner(
        region: Option<phonenumber::country::Id>,
        input: &str,
    ) -> Result<Self, WaveError> {
        let invalid = || WaveError::InvalidPhoneNumber {
            number: input.to_string(),
        };
        let parsed = phonenumber::parse(region, input).map_err(|_| invalid())?;
        if !phonenumber::is_valid(&parsed) {
            return Err(invalid());
        }
        Ok(Self(
            parsed.format().mode(phonenumber::Mode::E164).to_string(),
        ))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PhoneNumber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for PhoneNumber {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for PhoneNumber {
    type Error = WaveError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<PhoneNumber> for String {
    fn from(value: PhoneNumber) -> Self {
        value.0
    }
}

/// Identifiant opaque d'une transaction, tel que retourné par l'opérateur.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TransactionId(String);

impl TransactionId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TransactionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<String> for TransactionId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for TransactionId {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

/// Statut d'une transaction.
///
/// Les paiements mobile money sont asynchrones : une transaction naît
/// [`Pending`](TransactionStatus::Pending) et atteint son statut final via
/// webhook ou polling, une fois que l'utilisateur a confirmé (ou non) sur
/// son téléphone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransactionStatus {
    /// Initiée, en attente de confirmation de l'utilisateur.
    Pending,
    /// Confirmée et débitée.
    Successful,
    /// Refusée par l'opérateur ou l'utilisateur.
    Failed,
    /// Annulée avant confirmation.
    Cancelled,
    /// Expirée sans confirmation dans le délai imparti.
    Expired,
}

impl TransactionStatus {
    /// `true` si le statut ne changera plus (inutile de continuer à poller).
    pub fn is_final(&self) -> bool {
        !matches!(self, TransactionStatus::Pending)
    }
}

/// Une transaction telle que vue par l'opérateur.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transaction {
    pub id: TransactionId,
    /// Nom du provider (`"wave"`, `"orange"`, `"mtn"`, `"moov"`).
    pub provider: String,
    pub status: TransactionStatus,
    pub amount: Money,
    /// Contrepartie (destinataire pour un envoi, émetteur pour une réception).
    pub counterparty: Option<PhoneNumber>,
    pub note: Option<String>,
    /// Horodatage RFC 3339 tel que retourné par l'API de l'opérateur.
    pub created_at: String,
}

/// Requête d'initiation de paiement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaymentRequest {
    pub to: PhoneNumber,
    pub amount: Money,
    pub note: Option<String>,
    /// Référence idempotente côté client : rejouer la même référence ne doit
    /// pas créer un second paiement chez l'opérateur.
    pub reference: Option<String>,
}

impl PaymentRequest {
    pub fn new(to: PhoneNumber, amount: Money) -> Self {
        Self {
            to,
            amount,
            note: None,
            reference: None,
        }
    }

    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.note = Some(note.into());
        self
    }

    pub fn with_reference(mut self, reference: impl Into<String>) -> Self {
        self.reference = Some(reference.into());
        self
    }
}

/// Réponse à une initiation de paiement.
///
/// Le statut initial est presque toujours
/// [`Pending`](TransactionStatus::Pending) : suivre la transaction via
/// [`Provider::get_transaction`](crate::Provider::get_transaction).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaymentResponse {
    pub transaction_id: TransactionId,
    pub status: TransactionStatus,
    /// Nom du provider ayant traité la requête.
    pub provider: String,
}

/// Options de pagination pour la liste des transactions.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListOptions {
    /// Nombre maximum de transactions à retourner.
    pub limit: Option<u32>,
    /// Curseur de pagination opaque retourné par l'appel précédent.
    pub cursor: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_core_phone_number_parse_e164_ci() {
        let number = PhoneNumber::parse("+2250707070707").unwrap();
        assert_eq!(number.as_str(), "+2250707070707");
    }

    #[test]
    fn test_core_phone_number_parse_e164_sn() {
        let number = PhoneNumber::parse("+221771234567").unwrap();
        assert_eq!(number.as_str(), "+221771234567");
    }

    #[test]
    fn test_core_phone_number_parse_with_region_short_format() {
        let number = PhoneNumber::parse_with_region("CI", "07 07 07 07 07").unwrap();
        assert_eq!(number.as_str(), "+2250707070707");
    }

    #[test]
    fn test_core_phone_number_parse_invalid_rejected() {
        assert!(matches!(
            PhoneNumber::parse("hello"),
            Err(WaveError::InvalidPhoneNumber { .. })
        ));
        assert!(matches!(
            PhoneNumber::parse("+225"),
            Err(WaveError::InvalidPhoneNumber { .. })
        ));
    }

    #[test]
    fn test_core_phone_number_short_format_without_region_rejected() {
        assert!(matches!(
            PhoneNumber::parse("07 07 07 07 07"),
            Err(WaveError::InvalidPhoneNumber { .. })
        ));
    }

    #[test]
    fn test_core_phone_number_serde_roundtrip() {
        let number = PhoneNumber::parse("+2250707070707").unwrap();
        let json = serde_json::to_string(&number).unwrap();
        assert_eq!(json, "\"+2250707070707\"");
        let back: PhoneNumber = serde_json::from_str(&json).unwrap();
        assert_eq!(back, number);
    }

    #[test]
    fn test_core_phone_number_serde_rejects_invalid() {
        let result: Result<PhoneNumber, _> = serde_json::from_str("\"not-a-number\"");
        assert!(result.is_err());
    }

    #[test]
    fn test_core_phone_number_letters_after_prefix_rejected() {
        assert!(matches!(
            PhoneNumber::parse("+225abc"),
            Err(WaveError::InvalidPhoneNumber { .. })
        ));
    }

    #[test]
    fn test_core_phone_number_empty_rejected() {
        assert!(matches!(
            PhoneNumber::parse(""),
            Err(WaveError::InvalidPhoneNumber { .. })
        ));
    }

    #[test]
    fn test_core_money_xof_sets_amount_and_currency() {
        let money = Money::xof(5000);
        assert_eq!(money.amount, 5000);
        assert_eq!(money.currency, Currency::XOF);
        assert_eq!(money, Money::new(5000, Currency::XOF));
    }

    /// Le montant est un entier : aucune perte de précision, même sur des
    /// valeurs qu'un f64 arrondirait.
    #[test]
    fn test_core_money_large_amount_is_exact() {
        let money = Money::xof(9_007_199_254_740_993);
        assert_eq!(money.amount, 9_007_199_254_740_993);
    }

    #[test]
    fn test_core_money_display() {
        assert_eq!(Money::xof(5000).to_string(), "5000 XOF");
    }

    #[test]
    fn test_core_money_serde_roundtrip() {
        let money = Money::xof(5000);
        let json = serde_json::to_string(&money).unwrap();
        let back: Money = serde_json::from_str(&json).unwrap();
        assert_eq!(back, money);
    }

    /// Le `rename_all = "snake_case"` doit produire des valeurs stables :
    /// les fixtures et les APIs opérateur en dépendent.
    #[test]
    fn test_core_transaction_status_serde_roundtrip() {
        let cases = [
            (TransactionStatus::Pending, "\"pending\""),
            (TransactionStatus::Successful, "\"successful\""),
            (TransactionStatus::Failed, "\"failed\""),
            (TransactionStatus::Cancelled, "\"cancelled\""),
            (TransactionStatus::Expired, "\"expired\""),
        ];
        for (status, expected_json) in cases {
            let json = serde_json::to_string(&status).unwrap();
            assert_eq!(json, expected_json);
            let back: TransactionStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(back, status);
        }
    }

    #[test]
    fn test_core_list_options_defaults_are_none() {
        let opts = ListOptions::default();
        assert_eq!(opts.limit, None);
        assert_eq!(opts.cursor, None);
    }

    #[test]
    fn test_core_transaction_status_is_final() {
        assert!(!TransactionStatus::Pending.is_final());
        assert!(TransactionStatus::Successful.is_final());
        assert!(TransactionStatus::Failed.is_final());
        assert!(TransactionStatus::Cancelled.is_final());
        assert!(TransactionStatus::Expired.is_final());
    }

    #[test]
    fn test_core_transaction_serde_roundtrip() {
        let tx = Transaction {
            id: TransactionId::from("txn_abc123"),
            provider: "wave".to_string(),
            status: TransactionStatus::Pending,
            amount: Money::xof(5000),
            counterparty: Some(PhoneNumber::parse("+2250707070707").unwrap()),
            note: Some("Loyer".to_string()),
            created_at: "2026-08-09T12:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&tx).unwrap();
        let back: Transaction = serde_json::from_str(&json).unwrap();
        assert_eq!(back, tx);
    }

    #[test]
    fn test_core_payment_request_builder() {
        let to = PhoneNumber::parse("+2250707070707").unwrap();
        let request = PaymentRequest::new(to, Money::xof(5000))
            .with_note("Loyer")
            .with_reference("ref-001");
        assert_eq!(request.note.as_deref(), Some("Loyer"));
        assert_eq!(request.reference.as_deref(), Some("ref-001"));
    }
}
