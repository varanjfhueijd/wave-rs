//! Tests d'intégration du provider MTN MoMo — mock server wiremock,
//! zéro appel réseau réel. Couvre le flux en deux temps (202 sans corps)
//! et les normalisations (montants chaînes, MSISDN sans `+`).

use std::time::Duration;

use wave_core::{
    ListOptions, Money, PaymentRequest, PhoneNumber, Provider, TransactionId, TransactionStatus,
    WaveError,
};
use wave_mtn::{MtnConfig, MtnProvider};
use wiremock::matchers::{
    basic_auth, body_string_contains, header, header_exists, method, path, query_param,
};
use wiremock::{Mock, MockServer, ResponseTemplate};

const FIXTURE_TOKEN: &str = include_str!("fixtures/mtn/token.json");
const FIXTURE_BALANCE: &str = include_str!("fixtures/mtn/balance.json");
const FIXTURE_TRANSFER: &str = include_str!("fixtures/mtn/transfer_successful.json");
const FIXTURE_LIST: &str = include_str!("fixtures/mtn/transactions_list.json");
const FIXTURE_ERR_FUNDS: &str = include_str!("fixtures/mtn/error_not_enough_funds.json");
const FIXTURE_ERR_PAYEE: &str = include_str!("fixtures/mtn/error_payee_not_found.json");

fn provider(server: &MockServer) -> MtnProvider {
    let config = MtnConfig::new("sub-001", "user-001", "key-001")
        .with_base_url(server.uri())
        .with_target_environment("sandbox")
        .with_timeout(Duration::from_secs(2));
    MtnProvider::new(config).unwrap()
}

fn account() -> PhoneNumber {
    PhoneNumber::parse("+2250707070707").unwrap()
}

fn json_response(status: u16, body: &str) -> ResponseTemplate {
    ResponseTemplate::new(status).set_body_raw(body, "application/json")
}

/// Monte le endpoint token MoMo (Basic auth + clé de souscription).
async fn mount_token(server: &MockServer, expected_calls: u64) {
    Mock::given(method("POST"))
        .and(path("/disbursement/token/"))
        .and(basic_auth("user-001", "key-001"))
        .and(header("Ocp-Apim-Subscription-Key", "sub-001"))
        .respond_with(json_response(200, FIXTURE_TOKEN))
        .expect(expected_calls)
        .mount(server)
        .await;
}

#[tokio::test]
async fn test_mtn_payment_success_two_step_flow() {
    let server = MockServer::start().await;
    mount_token(&server, 1).await;
    // 202 Accepted SANS corps : le SDK doit générer lui-même la référence
    // (header X-Reference-Id) et la retourner comme transaction_id.
    Mock::given(method("POST"))
        .and(path("/disbursement/v1_0/transfer"))
        .and(header("authorization", "Bearer tok-momo-123"))
        .and(header("Ocp-Apim-Subscription-Key", "sub-001"))
        .and(header("X-Target-Environment", "sandbox"))
        .and(header_exists("X-Reference-Id"))
        .and(body_string_contains("\"partyId\":\"2250707070707\""))
        .and(body_string_contains("\"amount\":\"7500\""))
        .respond_with(ResponseTemplate::new(202))
        .expect(1)
        .mount(&server)
        .await;

    let request = PaymentRequest::new(account(), Money::xof(7500)).with_reference("ref-001");
    let response = provider(&server).initiate_payment(request).await.unwrap();

    assert_eq!(response.status, TransactionStatus::Pending);
    assert_eq!(response.provider, "mtn");
    // La référence retournée est un UUID v4 utilisable pour le polling.
    let id = response.transaction_id.to_string();
    assert_eq!(id.len(), 36);
    assert_eq!(&id[14..15], "4");
}

#[tokio::test]
async fn test_mtn_payment_insufficient_funds() {
    let server = MockServer::start().await;
    mount_token(&server, 1).await;
    Mock::given(method("POST"))
        .and(path("/disbursement/v1_0/transfer"))
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
async fn test_mtn_payment_api_error_mapped() {
    let server = MockServer::start().await;
    mount_token(&server, 1).await;
    Mock::given(method("POST"))
        .and(path("/disbursement/v1_0/transfer"))
        .respond_with(json_response(404, FIXTURE_ERR_PAYEE))
        .mount(&server)
        .await;

    let request = PaymentRequest::new(account(), Money::xof(7500));
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
            assert_eq!(provider, "mtn");
            assert_eq!(code, "PAYEE_NOT_FOUND");
            assert_eq!(message, "Payee does not exist on the MoMo network");
        }
        other => panic!("attendu ApiError, obtenu {other:?}"),
    }
}

#[tokio::test]
async fn test_mtn_auth_failed_on_token_endpoint() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/disbursement/token/"))
        .respond_with(json_response(401, "{}"))
        .mount(&server)
        .await;

    let error = provider(&server)
        .check_balance(&account())
        .await
        .unwrap_err();

    assert!(matches!(error, WaveError::AuthFailed { provider } if provider == "mtn"));
}

#[tokio::test]
async fn test_mtn_token_cached_across_calls() {
    let server = MockServer::start().await;
    mount_token(&server, 1).await;
    Mock::given(method("GET"))
        .and(path("/disbursement/v1_0/account/balance"))
        .respond_with(json_response(200, FIXTURE_BALANCE))
        .expect(2)
        .mount(&server)
        .await;

    let provider = provider(&server);
    provider.check_balance(&account()).await.unwrap();
    provider.check_balance(&account()).await.unwrap();
}

#[tokio::test]
async fn test_mtn_payment_rate_limited() {
    let server = MockServer::start().await;
    mount_token(&server, 1).await;
    Mock::given(method("POST"))
        .and(path("/disbursement/v1_0/transfer"))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "120"))
        .mount(&server)
        .await;

    let request = PaymentRequest::new(account(), Money::xof(7500));
    let error = provider(&server)
        .initiate_payment(request)
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        WaveError::RateLimited {
            retry_after_secs: 120
        }
    ));
}

#[tokio::test]
async fn test_mtn_balance_success_string_amount_parsed() {
    let server = MockServer::start().await;
    mount_token(&server, 1).await;
    Mock::given(method("GET"))
        .and(path("/disbursement/v1_0/account/balance"))
        .and(query_param("msisdn", "2250707070707"))
        .respond_with(json_response(200, FIXTURE_BALANCE))
        .expect(1)
        .mount(&server)
        .await;

    let balance = provider(&server).check_balance(&account()).await.unwrap();

    // "64000" (chaîne) → 64 000 francs entiers.
    assert_eq!(balance, Money::xof(64_000));
}

#[tokio::test]
async fn test_mtn_get_transaction_success_msisdn_normalized() {
    let server = MockServer::start().await;
    mount_token(&server, 1).await;
    Mock::given(method("GET"))
        .and(path(
            "/disbursement/v1_0/transfer/0f14d0ab-9605-4a62-a9e4-5ed26688389b",
        ))
        .respond_with(json_response(200, FIXTURE_TRANSFER))
        .mount(&server)
        .await;

    let id = TransactionId::from("0f14d0ab-9605-4a62-a9e4-5ed26688389b");
    let tx = provider(&server).get_transaction(&id).await.unwrap();

    assert_eq!(tx.id, id);
    assert_eq!(tx.provider, "mtn");
    assert_eq!(tx.status, TransactionStatus::Successful);
    assert_eq!(tx.amount, Money::xof(7500));
    // "2250707070707" (sans +) → normalisé E.164.
    assert_eq!(tx.counterparty, Some(account()));
    assert_eq!(tx.note.as_deref(), Some("Scolarité"));
    assert_eq!(tx.created_at, "2026-08-10T14:20:00Z");
}

#[tokio::test]
async fn test_mtn_list_transactions_success() {
    let server = MockServer::start().await;
    mount_token(&server, 1).await;
    Mock::given(method("GET"))
        .and(path("/disbursement/v1_0/transactions"))
        .and(query_param("msisdn", "2250707070707"))
        .and(query_param("limit", "10"))
        .respond_with(json_response(200, FIXTURE_LIST))
        .expect(1)
        .mount(&server)
        .await;

    let opts = ListOptions {
        limit: Some(10),
        cursor: None,
    };
    let txs = provider(&server)
        .list_transactions(&account(), opts)
        .await
        .unwrap();

    assert_eq!(txs.len(), 2);
    assert_eq!(
        txs[0].id,
        TransactionId::from("0f14d0ab-9605-4a62-a9e4-5ed26688389b")
    );
    assert_eq!(txs[0].status, TransactionStatus::Successful);
    assert_eq!(txs[1].status, TransactionStatus::Failed);
    assert_eq!(txs[1].counterparty, None);
    assert_eq!(txs[1].amount, Money::xof(2000));
}
