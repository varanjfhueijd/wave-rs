//! Tests d'intégration du provider Orange Money — mock server wiremock,
//! zéro appel réseau réel. Couvre notamment le cycle de vie du token OAuth2.

use std::time::Duration;

use wave_core::{
    ListOptions, Money, PaymentRequest, PhoneNumber, Provider, TransactionId, TransactionStatus,
    WaveError,
};
use wave_orange::{OrangeConfig, OrangeProvider};
use wiremock::matchers::{basic_auth, body_string_contains, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const FIXTURE_TOKEN: &str = include_str!("fixtures/orange/token.json");
const FIXTURE_TOKEN_SHORT: &str = include_str!("fixtures/orange/token_short_lived.json");
const FIXTURE_PAYMENT_PENDING: &str = include_str!("fixtures/orange/payment_pending.json");
const FIXTURE_BALANCE: &str = include_str!("fixtures/orange/balance.json");
const FIXTURE_TRANSACTION: &str = include_str!("fixtures/orange/transaction_success.json");
const FIXTURE_LIST: &str = include_str!("fixtures/orange/transactions_list.json");
const FIXTURE_ERR_FUNDS: &str = include_str!("fixtures/orange/error_insufficient_funds.json");
const FIXTURE_ERR_RECEIVER: &str = include_str!("fixtures/orange/error_receiver_unknown.json");

fn provider(server: &MockServer) -> OrangeProvider {
    let config = OrangeConfig::new("client-001", "secret-001")
        .with_base_url(server.uri())
        .with_timeout(Duration::from_secs(2));
    OrangeProvider::new(config).unwrap()
}

fn account() -> PhoneNumber {
    PhoneNumber::parse("+2250707070707").unwrap()
}

fn json_response(status: u16, body: &str) -> ResponseTemplate {
    ResponseTemplate::new(status).set_body_raw(body, "application/json")
}

/// Monte le endpoint de token OAuth2 en vérifiant le Basic auth et le
/// grant_type, et en imposant un nombre exact d'appels attendus.
async fn mount_token(server: &MockServer, fixture: &str, expected_calls: u64) {
    Mock::given(method("POST"))
        .and(path("/oauth/v3/token"))
        .and(basic_auth("client-001", "secret-001"))
        .and(body_string_contains("grant_type=client_credentials"))
        .respond_with(json_response(200, fixture))
        .expect(expected_calls)
        .mount(server)
        .await;
}

#[tokio::test]
async fn test_orange_payment_success() {
    let server = MockServer::start().await;
    mount_token(&server, FIXTURE_TOKEN, 1).await;
    Mock::given(method("POST"))
        .and(path("/omoney/v1/payments"))
        .and(header("authorization", "Bearer tok-orange-123"))
        .respond_with(json_response(200, FIXTURE_PAYMENT_PENDING))
        .expect(1)
        .mount(&server)
        .await;

    let request = PaymentRequest::new(account(), Money::xof(7500)).with_note("Facture CIE");
    let response = provider(&server).initiate_payment(request).await.unwrap();

    assert_eq!(response.transaction_id, TransactionId::from("OM-2026-001"));
    assert_eq!(response.status, TransactionStatus::Pending);
    assert_eq!(response.provider, "orange");
}

#[tokio::test]
async fn test_orange_token_cached_across_calls() {
    let server = MockServer::start().await;
    // Un seul appel token attendu pour deux appels API : le cache travaille.
    mount_token(&server, FIXTURE_TOKEN, 1).await;
    Mock::given(method("GET"))
        .and(path("/omoney/v1/balance"))
        .respond_with(json_response(200, FIXTURE_BALANCE))
        .expect(2)
        .mount(&server)
        .await;

    let provider = provider(&server);
    provider.check_balance(&account()).await.unwrap();
    provider.check_balance(&account()).await.unwrap();
}

#[tokio::test]
async fn test_orange_token_refreshed_when_short_lived() {
    let server = MockServer::start().await;
    // expires_in (30s) < marge de sécurité (60s) : le token est considéré
    // expiré immédiatement, donc re-demandé à chaque appel API.
    mount_token(&server, FIXTURE_TOKEN_SHORT, 2).await;
    Mock::given(method("GET"))
        .and(path("/omoney/v1/balance"))
        .respond_with(json_response(200, FIXTURE_BALANCE))
        .expect(2)
        .mount(&server)
        .await;

    let provider = provider(&server);
    provider.check_balance(&account()).await.unwrap();
    provider.check_balance(&account()).await.unwrap();
}

#[tokio::test]
async fn test_orange_auth_failed_on_token_endpoint() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/v3/token"))
        .respond_with(json_response(401, "{}"))
        .mount(&server)
        .await;

    let request = PaymentRequest::new(account(), Money::xof(7500));
    let error = provider(&server)
        .initiate_payment(request)
        .await
        .unwrap_err();

    assert!(matches!(error, WaveError::AuthFailed { provider } if provider == "orange"));
}

#[tokio::test]
async fn test_orange_api_401_invalidates_cached_token() {
    let server = MockServer::start().await;
    // Deux appels token attendus : le 401 de l'API purge le cache.
    mount_token(&server, FIXTURE_TOKEN, 2).await;
    // Premier appel API : 401 (token révoqué côté Orange).
    Mock::given(method("GET"))
        .and(path("/omoney/v1/balance"))
        .respond_with(json_response(401, "{}"))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    // Appels suivants : succès.
    Mock::given(method("GET"))
        .and(path("/omoney/v1/balance"))
        .respond_with(json_response(200, FIXTURE_BALANCE))
        .mount(&server)
        .await;

    let provider = provider(&server);
    let error = provider.check_balance(&account()).await.unwrap_err();
    assert!(matches!(error, WaveError::AuthFailed { .. }));

    let balance = provider.check_balance(&account()).await.unwrap();
    assert_eq!(balance, Money::xof(82_500));
}

#[tokio::test]
async fn test_orange_payment_insufficient_funds() {
    let server = MockServer::start().await;
    mount_token(&server, FIXTURE_TOKEN, 1).await;
    Mock::given(method("POST"))
        .and(path("/omoney/v1/payments"))
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
async fn test_orange_payment_api_error_mapped() {
    let server = MockServer::start().await;
    mount_token(&server, FIXTURE_TOKEN, 1).await;
    Mock::given(method("POST"))
        .and(path("/omoney/v1/payments"))
        .respond_with(json_response(404, FIXTURE_ERR_RECEIVER))
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
            assert_eq!(provider, "orange");
            assert_eq!(code, "60011");
            assert_eq!(message, "Destinataire inconnu du réseau Orange Money");
        }
        other => panic!("attendu ApiError, obtenu {other:?}"),
    }
}

#[tokio::test]
async fn test_orange_payment_rate_limited() {
    let server = MockServer::start().await;
    mount_token(&server, FIXTURE_TOKEN, 1).await;
    Mock::given(method("POST"))
        .and(path("/omoney/v1/payments"))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "45"))
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
            retry_after_secs: 45
        }
    ));
}

#[tokio::test]
async fn test_orange_balance_success() {
    let server = MockServer::start().await;
    mount_token(&server, FIXTURE_TOKEN, 1).await;
    Mock::given(method("GET"))
        .and(path("/omoney/v1/balance"))
        .and(query_param("msisdn", "+2250707070707"))
        .respond_with(json_response(200, FIXTURE_BALANCE))
        .expect(1)
        .mount(&server)
        .await;

    let balance = provider(&server).check_balance(&account()).await.unwrap();

    assert_eq!(balance, Money::xof(82_500));
}

#[tokio::test]
async fn test_orange_get_transaction_success() {
    let server = MockServer::start().await;
    mount_token(&server, FIXTURE_TOKEN, 1).await;
    Mock::given(method("GET"))
        .and(path("/omoney/v1/transactions/OM-2026-001"))
        .respond_with(json_response(200, FIXTURE_TRANSACTION))
        .mount(&server)
        .await;

    let id = TransactionId::from("OM-2026-001");
    let tx = provider(&server).get_transaction(&id).await.unwrap();

    assert_eq!(tx.id, id);
    assert_eq!(tx.provider, "orange");
    assert_eq!(tx.status, TransactionStatus::Successful);
    assert_eq!(tx.amount, Money::xof(7500));
    assert_eq!(tx.counterparty, Some(account()));
    assert_eq!(tx.note.as_deref(), Some("Facture CIE"));
}

#[tokio::test]
async fn test_orange_list_transactions_success() {
    let server = MockServer::start().await;
    mount_token(&server, FIXTURE_TOKEN, 1).await;
    Mock::given(method("GET"))
        .and(path("/omoney/v1/transactions"))
        .and(query_param("msisdn", "+2250707070707"))
        .and(query_param("limit", "5"))
        .respond_with(json_response(200, FIXTURE_LIST))
        .expect(1)
        .mount(&server)
        .await;

    let opts = ListOptions {
        limit: Some(5),
        cursor: None,
    };
    let txs = provider(&server)
        .list_transactions(&account(), opts)
        .await
        .unwrap();

    assert_eq!(txs.len(), 2);
    assert_eq!(txs[0].status, TransactionStatus::Successful);
    assert_eq!(txs[1].status, TransactionStatus::Expired);
    assert_eq!(txs[1].counterparty, None);
    assert_eq!(txs[1].amount, Money::xof(3000));
}
