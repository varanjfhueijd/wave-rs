//! Tests d'intégration transverses : le même code appelant doit fonctionner
//! avec les quatre opérateurs derrière `&dyn Provider`, sans rien savoir de
//! leurs dialectes wire respectifs. Zéro appel réseau réel (wiremock).

use std::time::Duration;

use wave_core::{
    Currency, ListOptions, Money, PaymentRequest, PaymentResponse, PhoneNumber, Provider,
    Transaction, TransactionId, TransactionStatus, WaveError,
};
use wave_moov::{MoovConfig, MoovProvider};
use wave_mtn::{MtnConfig, MtnProvider};
use wave_orange::{OrangeConfig, OrangeProvider};
use wave_wave::{WaveConfig, WaveProvider};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn account() -> PhoneNumber {
    PhoneNumber::parse("+2250707070707").unwrap()
}

fn json_response(status: u16, body: &str) -> ResponseTemplate {
    ResponseTemplate::new(status).set_body_raw(body, "application/json")
}

/// Scénario unique exécuté à l'identique sur chaque provider : c'est la
/// garantie d'interchangeabilité que vend le SDK.
async fn assert_payment_contract(provider: &dyn Provider, expected_name: &str) {
    assert_eq!(provider.name(), expected_name);
    assert_eq!(provider.currency(), Currency::XOF);

    let request = PaymentRequest::new(account(), Money::xof(5000)).with_note("Loyer");
    let response = provider.initiate_payment(request).await.unwrap();

    assert_eq!(response.provider, expected_name);
    assert_eq!(response.status, TransactionStatus::Pending);
    assert!(!response.transaction_id.as_str().is_empty());
}

async fn mock_wave() -> (MockServer, WaveProvider) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/payouts"))
        .respond_with(json_response(
            200,
            r#"{"id":"txn_wave_x","status":"pending"}"#,
        ))
        .mount(&server)
        .await;
    let provider = WaveProvider::new(
        WaveConfig::new("k", "m")
            .with_base_url(server.uri())
            .with_timeout(Duration::from_secs(2)),
    )
    .unwrap();
    (server, provider)
}

async fn mock_orange() -> (MockServer, OrangeProvider) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/v3/token"))
        .respond_with(json_response(
            200,
            r#"{"access_token":"t","token_type":"Bearer","expires_in":3600}"#,
        ))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/omoney/v1/payments"))
        .respond_with(json_response(
            200,
            r#"{"transaction_id":"OM-x","status":"PENDING"}"#,
        ))
        .mount(&server)
        .await;
    let provider = OrangeProvider::new(
        OrangeConfig::new("c", "s")
            .with_base_url(server.uri())
            .with_timeout(Duration::from_secs(2)),
    )
    .unwrap();
    (server, provider)
}

async fn mock_mtn() -> (MockServer, MtnProvider) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/disbursement/token/"))
        .respond_with(json_response(
            200,
            r#"{"access_token":"t","token_type":"access_token","expires_in":3600}"#,
        ))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/disbursement/v1_0/transfer"))
        .respond_with(ResponseTemplate::new(202))
        .mount(&server)
        .await;
    let provider = MtnProvider::new(
        MtnConfig::new("sub", "u", "k")
            .with_base_url(server.uri())
            .with_target_environment("sandbox")
            .with_timeout(Duration::from_secs(2)),
    )
    .unwrap();
    (server, provider)
}

async fn mock_moov() -> (MockServer, MoovProvider) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/payments"))
        .respond_with(json_response(200, r#"{"id":"mv_x","status":"pending"}"#))
        .mount(&server)
        .await;
    let provider = MoovProvider::new(
        MoovConfig::new("k")
            .with_base_url(server.uri())
            .with_timeout(Duration::from_secs(2)),
    )
    .unwrap();
    (server, provider)
}

#[tokio::test]
async fn test_wave_payment_contract_via_trait_object() {
    let (_server, provider) = mock_wave().await;
    assert_payment_contract(&provider, "wave").await;
}

#[tokio::test]
async fn test_orange_payment_contract_via_trait_object() {
    let (_server, provider) = mock_orange().await;
    assert_payment_contract(&provider, "orange").await;
}

#[tokio::test]
async fn test_mtn_payment_contract_via_trait_object() {
    let (_server, provider) = mock_mtn().await;
    assert_payment_contract(&provider, "mtn").await;
}

#[tokio::test]
async fn test_moov_payment_contract_via_trait_object() {
    let (_server, provider) = mock_moov().await;
    assert_payment_contract(&provider, "moov").await;
}

#[tokio::test]
async fn test_all_providers_interchangeable_in_one_collection() {
    // L'usage cible du SDK : une collection hétérogène de providers
    // pilotée par un code strictement identique.
    let (_s1, wave) = mock_wave().await;
    let (_s2, orange) = mock_orange().await;
    let (_s3, mtn) = mock_mtn().await;
    let (_s4, moov) = mock_moov().await;

    let providers: Vec<(Box<dyn Provider>, &str)> = vec![
        (Box::new(wave), "wave"),
        (Box::new(orange), "orange"),
        (Box::new(mtn), "mtn"),
        (Box::new(moov), "moov"),
    ];

    for (provider, name) in &providers {
        assert_payment_contract(provider.as_ref(), name).await;
    }
}

// ---------------------------------------------------------------------------
// Mocking du trait pour le code consommateur (mockall)
// ---------------------------------------------------------------------------

mockall::mock! {
    Provider {}

    #[async_trait::async_trait]
    impl Provider for Provider {
        fn name(&self) -> &'static str;
        fn currency(&self) -> Currency;
        async fn initiate_payment(
            &self,
            request: PaymentRequest,
        ) -> Result<PaymentResponse, WaveError>;
        async fn check_balance(&self, account: &PhoneNumber) -> Result<Money, WaveError>;
        async fn get_transaction(&self, id: &TransactionId) -> Result<Transaction, WaveError>;
        async fn list_transactions(
            &self,
            account: &PhoneNumber,
            opts: ListOptions,
        ) -> Result<Vec<Transaction>, WaveError>;
    }
}

/// Exemple de logique applicative écrite contre `dyn Provider`.
async fn payer_si_solde_suffisant(
    provider: &dyn Provider,
    to: PhoneNumber,
    amount: Money,
) -> Result<Option<PaymentResponse>, WaveError> {
    let balance = provider.check_balance(&to).await?;
    if balance.amount < amount.amount {
        return Ok(None);
    }
    provider
        .initiate_payment(PaymentRequest::new(to, amount))
        .await
        .map(Some)
}

#[tokio::test]
async fn test_mockall_consumer_logic_skips_payment_when_balance_low() {
    let mut mock = MockProvider::new();
    mock.expect_check_balance()
        .returning(|_| Ok(Money::xof(1000)));
    mock.expect_initiate_payment().never();

    let result = payer_si_solde_suffisant(&mock, account(), Money::xof(5000))
        .await
        .unwrap();

    assert!(result.is_none());
}

#[tokio::test]
async fn test_mockall_consumer_logic_pays_when_balance_sufficient() {
    let mut mock = MockProvider::new();
    mock.expect_check_balance()
        .returning(|_| Ok(Money::xof(10_000)));
    mock.expect_initiate_payment().times(1).returning(|_| {
        Ok(PaymentResponse {
            transaction_id: TransactionId::from("txn_mock"),
            status: TransactionStatus::Pending,
            provider: "mock".to_string(),
        })
    });

    let result = payer_si_solde_suffisant(&mock, account(), Money::xof(5000))
        .await
        .unwrap();

    assert_eq!(
        result.map(|r| r.transaction_id),
        Some(TransactionId::from("txn_mock"))
    );
}
