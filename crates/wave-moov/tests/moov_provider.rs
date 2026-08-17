//! Tests d'intégration du provider Moov Africa — mock server wiremock,
//! zéro appel réseau réel.

use std::time::Duration;

use wave_core::{
    ListOptions, Money, PaymentRequest, PhoneNumber, Provider, TransactionId, TransactionStatus,
    WaveError,
};
use wave_moov::{MoovConfig, MoovProvider};
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const FIXTURE_PAYMENT_PENDING: &str = include_str!("fixtures/moov/payment_pending.json");
const FIXTURE_BALANCE: &str = include_str!("fixtures/moov/balance.json");
const FIXTURE_TRANSACTION: &str = include_str!("fixtures/moov/transaction_completed.json");
const FIXTURE_LIST: &str = include_str!("fixtures/moov/transactions_list.json");
const FIXTURE_ERR_BALANCE: &str = include_str!("fixtures/moov/error_insufficient_balance.json");
const FIXTURE_ERR_BLOCKED: &str = include_str!("fixtures/moov/error_wallet_blocked.json");

fn provider(server: &MockServer) -> MoovProvider {
    let config = MoovConfig::new("cle-moov-001")
        .with_base_url(server.uri())
        .with_timeout(Duration::from_secs(2));
    MoovProvider::new(config).unwrap()
}

fn account() -> PhoneNumber {
    PhoneNumber::parse("+2250707070707").unwrap()
}

fn json_response(status: u16, body: &str) -> ResponseTemplate {
    ResponseTemplate::new(status).set_body_raw(body, "application/json")
}

#[tokio::test]
async fn test_moov_payment_success() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/payments"))
        .and(header("X-API-Key", "cle-moov-001"))
        .respond_with(json_response(200, FIXTURE_PAYMENT_PENDING))
        .expect(1)
        .mount(&server)
        .await;

    let request = PaymentRequest::new(account(), Money::xof(12_000)).with_note("Abonnement Canal");
    let response = provider(&server).initiate_payment(request).await.unwrap();

    assert_eq!(response.transaction_id, TransactionId::from("mv_txn_001"));
    assert_eq!(response.status, TransactionStatus::Pending);
    assert_eq!(response.provider, "moov");
}

#[tokio::test]
async fn test_moov_payment_sends_idempotency_key() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/payments"))
        .and(header("Idempotency-Key", "ref-moov-001"))
        .respond_with(json_response(200, FIXTURE_PAYMENT_PENDING))
        .expect(1)
        .mount(&server)
        .await;

    let request = PaymentRequest::new(account(), Money::xof(12_000)).with_reference("ref-moov-001");
    provider(&server).initiate_payment(request).await.unwrap();
}

#[tokio::test]
async fn test_moov_payment_insufficient_funds() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/payments"))
        .respond_with(json_response(400, FIXTURE_ERR_BALANCE))
        .mount(&server)
        .await;

    let request = PaymentRequest::new(account(), Money::xof(10_000_000));
    let error = provider(&server)
        .initiate_payment(request)
        .await
        .unwrap_err();

    assert!(matches!(error, WaveError::InsufficientFunds));
}

#[tokio::test]
async fn test_moov_payment_auth_failed() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/payments"))
        .respond_with(json_response(403, "{}"))
        .mount(&server)
        .await;

    let request = PaymentRequest::new(account(), Money::xof(12_000));
    let error = provider(&server)
        .initiate_payment(request)
        .await
        .unwrap_err();

    assert!(matches!(error, WaveError::AuthFailed { provider } if provider == "moov"));
}

#[tokio::test]
async fn test_moov_payment_api_error_mapped() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/payments"))
        .respond_with(json_response(409, FIXTURE_ERR_BLOCKED))
        .mount(&server)
        .await;

    let request = PaymentRequest::new(account(), Money::xof(12_000));
    let error = provider(&server)
        .initiate_payment(request)
        .await
        .unwrap_err();

    match error {
        WaveError::ApiError {
            provider,
            code,
            message,
        } => {
            assert_eq!(provider, "moov");
            assert_eq!(code, "WALLET_BLOCKED");
            assert_eq!(message, "Le portefeuille du destinataire est bloqué");
        }
        other => panic!("attendu ApiError, obtenu {other:?}"),
    }
}

#[tokio::test]
async fn test_moov_payment_rate_limited() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/payments"))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "15"))
        .mount(&server)
        .await;

    let request = PaymentRequest::new(account(), Money::xof(12_000));
    let error = provider(&server)
        .initiate_payment(request)
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        WaveError::RateLimited {
            retry_after_secs: 15
        }
    ));
}

#[tokio::test]
async fn test_moov_balance_success() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/balance"))
        .and(query_param("account", "+2250707070707"))
        .and(header("X-API-Key", "cle-moov-001"))
        .respond_with(json_response(200, FIXTURE_BALANCE))
        .expect(1)
        .mount(&server)
        .await;

    let balance = provider(&server).check_balance(&account()).await.unwrap();

    assert_eq!(balance, Money::xof(41_000));
}

#[tokio::test]
async fn test_moov_get_transaction_completed_maps_to_successful() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/transactions/mv_txn_001"))
        .respond_with(json_response(200, FIXTURE_TRANSACTION))
        .mount(&server)
        .await;

    let id = TransactionId::from("mv_txn_001");
    let tx = provider(&server).get_transaction(&id).await.unwrap();

    assert_eq!(tx.id, id);
    assert_eq!(tx.provider, "moov");
    // Statut wire "completed" → Successful unifié.
    assert_eq!(tx.status, TransactionStatus::Successful);
    assert_eq!(tx.amount, Money::xof(12_000));
    assert_eq!(tx.counterparty, Some(account()));
    assert_eq!(tx.note.as_deref(), Some("Abonnement Canal"));
}

#[tokio::test]
async fn test_moov_list_transactions_success() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/transactions"))
        .and(query_param("account", "+2250707070707"))
        .and(query_param("limit", "20"))
        .respond_with(json_response(200, FIXTURE_LIST))
        .expect(1)
        .mount(&server)
        .await;

    let opts = ListOptions {
        limit: Some(20),
        cursor: None,
    };
    let txs = provider(&server)
        .list_transactions(&account(), opts)
        .await
        .unwrap();

    assert_eq!(txs.len(), 2);
    assert_eq!(txs[0].status, TransactionStatus::Successful);
    assert_eq!(txs[1].status, TransactionStatus::Cancelled);
    assert_eq!(txs[1].counterparty, None);
    assert_eq!(txs[1].amount, Money::xof(900));
}

#[tokio::test]
async fn test_moov_list_transactions_empty_returns_empty_vec() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/transactions"))
        .respond_with(json_response(200, r#"{"items": []}"#))
        .expect(1)
        .mount(&server)
        .await;

    // Un compte sans historique n'est pas une erreur.
    let txs = provider(&server)
        .list_transactions(&account(), ListOptions::default())
        .await
        .unwrap();

    assert!(txs.is_empty());
}

#[tokio::test]
async fn test_moov_malformed_json_returns_serialization_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/balance"))
        .respond_with(json_response(200, "{ ceci n'est pas du JSON }"))
        .mount(&server)
        .await;

    let err = provider(&server)
        .check_balance(&account())
        .await
        .unwrap_err();

    assert!(matches!(err, WaveError::Serialization(_)));
}
