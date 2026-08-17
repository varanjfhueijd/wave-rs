//! Tests d'intégration du provider Wave — mock server wiremock,
//! zéro appel réseau réel.

use std::time::Duration;

use wave_core::{
    ListOptions, Money, PaymentRequest, PhoneNumber, Provider, TransactionId, TransactionStatus,
    WaveError,
};
use wave_wave::{WaveConfig, WaveProvider};
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const FIXTURE_PAYOUT_PENDING: &str = include_str!("fixtures/wave/payout_pending.json");
const FIXTURE_BALANCE: &str = include_str!("fixtures/wave/balance.json");
const FIXTURE_TRANSACTION: &str = include_str!("fixtures/wave/transaction_successful.json");
const FIXTURE_LIST: &str = include_str!("fixtures/wave/transactions_list.json");
const FIXTURE_ERR_FUNDS: &str = include_str!("fixtures/wave/error_insufficient_funds.json");
const FIXTURE_ERR_RECIPIENT: &str = include_str!("fixtures/wave/error_recipient_not_found.json");

fn provider(server: &MockServer) -> WaveProvider {
    let config = WaveConfig::new("test-key", "m-001")
        .with_base_url(server.uri())
        .with_timeout(Duration::from_secs(2));
    WaveProvider::new(config).unwrap()
}

fn account() -> PhoneNumber {
    PhoneNumber::parse("+2250707070707").unwrap()
}

fn json_response(status: u16, body: &str) -> ResponseTemplate {
    ResponseTemplate::new(status).set_body_raw(body, "application/json")
}

#[tokio::test]
async fn test_wave_payment_success() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/payouts"))
        .and(header("authorization", "Bearer test-key"))
        .respond_with(json_response(200, FIXTURE_PAYOUT_PENDING))
        .expect(1)
        .mount(&server)
        .await;

    let request = PaymentRequest::new(account(), Money::xof(5000)).with_note("Loyer");
    let response = provider(&server).initiate_payment(request).await.unwrap();

    assert_eq!(response.transaction_id, TransactionId::from("txn_wave_001"));
    assert_eq!(response.status, TransactionStatus::Pending);
    assert_eq!(response.provider, "wave");
}

#[tokio::test]
async fn test_wave_payment_sends_idempotency_key() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/payouts"))
        .and(header("Idempotency-Key", "ref-001"))
        .respond_with(json_response(200, FIXTURE_PAYOUT_PENDING))
        .expect(1)
        .mount(&server)
        .await;

    let request = PaymentRequest::new(account(), Money::xof(5000)).with_reference("ref-001");
    provider(&server).initiate_payment(request).await.unwrap();
}

#[tokio::test]
async fn test_wave_payment_insufficient_funds() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/payouts"))
        .respond_with(json_response(400, FIXTURE_ERR_FUNDS))
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
async fn test_wave_payment_auth_failed() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/payouts"))
        .respond_with(json_response(401, "{}"))
        .mount(&server)
        .await;

    let request = PaymentRequest::new(account(), Money::xof(5000));
    let error = provider(&server)
        .initiate_payment(request)
        .await
        .unwrap_err();

    assert!(matches!(error, WaveError::AuthFailed { provider } if provider == "wave"));
}

#[tokio::test]
async fn test_wave_payment_rate_limited() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/payouts"))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "30"))
        .mount(&server)
        .await;

    let request = PaymentRequest::new(account(), Money::xof(5000));
    let error = provider(&server)
        .initiate_payment(request)
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        WaveError::RateLimited {
            retry_after_secs: 30
        }
    ));
}

#[tokio::test]
async fn test_wave_payment_api_error_mapped() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/payouts"))
        .respond_with(json_response(404, FIXTURE_ERR_RECIPIENT))
        .mount(&server)
        .await;

    let request = PaymentRequest::new(account(), Money::xof(5000));
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
            assert_eq!(provider, "wave");
            assert_eq!(code, "recipient-not-found");
            assert_eq!(message, "Compte destinataire introuvable");
        }
        other => panic!("attendu ApiError, obtenu {other:?}"),
    }
}

#[tokio::test]
async fn test_wave_payment_timeout() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/payouts"))
        .respond_with(json_response(200, FIXTURE_PAYOUT_PENDING).set_delay(Duration::from_secs(5)))
        .mount(&server)
        .await;

    let config = WaveConfig::new("test-key", "m-001")
        .with_base_url(server.uri())
        .with_timeout(Duration::from_millis(200));
    let provider = WaveProvider::new(config).unwrap();

    let request = PaymentRequest::new(account(), Money::xof(5000));
    let error = provider.initiate_payment(request).await.unwrap_err();

    assert!(matches!(error, WaveError::Timeout { .. }));
}

#[tokio::test]
async fn test_wave_balance_success() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/balance"))
        .and(query_param("account", "+2250707070707"))
        .and(header("authorization", "Bearer test-key"))
        .respond_with(json_response(200, FIXTURE_BALANCE))
        .expect(1)
        .mount(&server)
        .await;

    let balance = provider(&server).check_balance(&account()).await.unwrap();

    assert_eq!(balance, Money::xof(150_000));
}

#[tokio::test]
async fn test_wave_get_transaction_success() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/transactions/txn_wave_001"))
        .respond_with(json_response(200, FIXTURE_TRANSACTION))
        .mount(&server)
        .await;

    let id = TransactionId::from("txn_wave_001");
    let tx = provider(&server).get_transaction(&id).await.unwrap();

    assert_eq!(tx.id, id);
    assert_eq!(tx.provider, "wave");
    assert_eq!(tx.status, TransactionStatus::Successful);
    assert_eq!(tx.amount, Money::xof(5000));
    assert_eq!(tx.counterparty, Some(account()));
    assert_eq!(tx.note.as_deref(), Some("Loyer"));
}

#[tokio::test]
async fn test_wave_list_transactions_success() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/transactions"))
        .and(query_param("account", "+2250707070707"))
        .and(query_param("limit", "10"))
        .and(query_param("cursor", "cur_001"))
        .respond_with(json_response(200, FIXTURE_LIST))
        .expect(1)
        .mount(&server)
        .await;

    let opts = ListOptions {
        limit: Some(10),
        cursor: Some("cur_001".to_string()),
    };
    let txs = provider(&server)
        .list_transactions(&account(), opts)
        .await
        .unwrap();

    assert_eq!(txs.len(), 2);
    assert_eq!(txs[0].status, TransactionStatus::Successful);
    assert_eq!(txs[1].status, TransactionStatus::Pending);
    assert_eq!(txs[1].counterparty, None);
    assert_eq!(txs[1].amount, Money::xof(12_500));
}

#[tokio::test]
async fn test_wave_provider_metadata() {
    let server = MockServer::start().await;
    let provider = provider(&server);
    assert_eq!(provider.name(), "wave");
    assert_eq!(provider.currency(), wave_core::Currency::XOF);
}

#[tokio::test]
async fn test_wave_list_transactions_empty_returns_empty_vec() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/transactions"))
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
async fn test_wave_malformed_json_returns_serialization_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/balance"))
        .respond_with(json_response(200, "{ ceci n'est pas du JSON }"))
        .mount(&server)
        .await;

    let err = provider(&server)
        .check_balance(&account())
        .await
        .unwrap_err();

    assert!(matches!(err, WaveError::Serialization(_)));
}
