//! Tests d'intégration **live** — appellent les vraies APIs sandbox des
//! opérateurs, contrairement au reste de la suite qui est 100 % wiremock.
//!
//! Ils ne tournent jamais en CI automatique : tous marqués `#[ignore]`.
//!
//! ```bash
//! # .env rempli avec de vraies clés sandbox, puis :
//! WAVE_LIVE_TESTS=true cargo test --test live -- --ignored --nocapture
//! ```
//!
//! Chaque test skippe proprement (sans échouer) si `WAVE_LIVE_TESTS` n'est
//! pas activé ou si les credentials de l'opérateur concerné sont absents :
//! un contributeur n'ayant accès qu'à la sandbox MTN peut lancer les tests
//! MTN sans voir échouer les trois autres.

use wave_core::{ListOptions, Money, PaymentRequest, PhoneNumber, Provider, WaveError};

/// Montant volontairement minimal pour les paiements live.
const LIVE_TEST_AMOUNT_XOF: u64 = 100;

/// `true` si les tests live sont explicitement activés.
fn live_enabled() -> bool {
    std::env::var("WAVE_LIVE_TESTS")
        .map(|v| matches!(v.trim(), "true" | "1"))
        .unwrap_or(false)
}

/// Numéro de test sandbox de l'opérateur, ex. `WAVE_TEST_MSISDN`.
fn test_msisdn(var: &str) -> Option<PhoneNumber> {
    let raw = std::env::var(var).ok()?;
    PhoneNumber::parse(raw.trim()).ok()
}

/// Garde commune : charge le `.env` et vérifie que le test doit tourner.
///
/// Retourne `None` (→ skip) plutôt qu'un panic : un test live sauté n'est
/// pas un échec.
fn guard(provider_name: &str, msisdn_var: &str) -> Option<PhoneNumber> {
    let _ = dotenvy::dotenv();

    if !live_enabled() {
        eprintln!("Skipping live test — set WAVE_LIVE_TESTS=true with real sandbox credentials");
        return None;
    }

    match test_msisdn(msisdn_var) {
        Some(number) => Some(number),
        None => {
            eprintln!(
                "Skipping {provider_name} live test — set {msisdn_var} to a valid sandbox number"
            );
            None
        }
    }
}

/// Une config absente signifie « credentials manquants », pas « le SDK est
/// cassé » : on skippe au lieu d'échouer.
///
/// Les providers signalent une variable d'env manquante via
/// `ApiError { code: "config", .. }` (voir `require_env`).
fn skip_if_unconfigured(provider_name: &str, err: &WaveError) -> bool {
    if let WaveError::ApiError { code, message, .. } = err {
        if code == "config" {
            eprintln!("Skipping {provider_name} live test — {message}");
            return true;
        }
    }
    false
}

macro_rules! live_balance_test {
    ($test_name:ident, $provider_ty:path, $label:literal, $msisdn_var:literal) => {
        #[tokio::test]
        #[ignore = "test live — nécessite de vraies clés sandbox"]
        async fn $test_name() {
            let Some(account) = guard($label, $msisdn_var) else {
                return;
            };

            let provider = match <$provider_ty>::from_env() {
                Ok(provider) => provider,
                Err(err) => {
                    assert!(skip_if_unconfigured($label, &err), "{err}");
                    return;
                }
            };

            let balance = provider
                .check_balance(&account)
                .await
                .unwrap_or_else(|err| panic!("{} check_balance a échoué : {err}", $label));

            println!("[{}] solde sandbox = {balance}", $label);
            assert_eq!(balance.currency, provider.currency());
        }
    };
}

live_balance_test!(
    test_live_wave_check_balance,
    wave_wave::WaveProvider,
    "wave",
    "WAVE_TEST_MSISDN"
);
live_balance_test!(
    test_live_orange_check_balance,
    wave_orange::OrangeProvider,
    "orange",
    "ORANGE_TEST_MSISDN"
);
live_balance_test!(
    test_live_mtn_check_balance,
    wave_mtn::MtnProvider,
    "mtn",
    "MTN_TEST_MSISDN"
);
live_balance_test!(
    test_live_moov_check_balance,
    wave_moov::MoovProvider,
    "moov",
    "MOOV_TEST_MSISDN"
);

/// Envoie un vrai paiement sandbox de 100 XOF, puis relit la transaction
/// créée pour vérifier la cohérence des deux endpoints.
///
/// MTN est l'opérateur choisi ici : sa sandbox est ouverte sans accord
/// commercial (momodeveloper.mtn.com).
#[tokio::test]
#[ignore = "test live — envoie un vrai paiement sandbox"]
async fn test_live_mtn_initiate_payment_and_read_back() {
    let Some(account) = guard("mtn", "MTN_TEST_MSISDN") else {
        return;
    };

    let provider = match wave_mtn::MtnProvider::from_env() {
        Ok(provider) => provider,
        Err(err) => {
            assert!(skip_if_unconfigured("mtn", &err), "{err}");
            return;
        }
    };

    let request = PaymentRequest::new(account.clone(), Money::xof(LIVE_TEST_AMOUNT_XOF))
        .with_note("wave-rs live test");

    let response = provider
        .initiate_payment(request)
        .await
        .unwrap_or_else(|err| panic!("mtn initiate_payment a échoué : {err}"));

    println!(
        "[mtn] transaction={} statut={:?}",
        response.transaction_id, response.status
    );
    assert_eq!(response.provider, "mtn");

    // Le paiement est asynchrone : le statut immédiat n'est pas encore final.
    // On vérifie seulement que la transaction est relisible et cohérente.
    let transaction = provider
        .get_transaction(&response.transaction_id)
        .await
        .unwrap_or_else(|err| panic!("mtn get_transaction a échoué : {err}"));

    assert_eq!(transaction.id, response.transaction_id);
    assert_eq!(transaction.amount, Money::xof(LIVE_TEST_AMOUNT_XOF));
    println!("[mtn] statut après relecture = {:?}", transaction.status);
}

#[tokio::test]
#[ignore = "test live — nécessite de vraies clés sandbox"]
async fn test_live_mtn_list_transactions() {
    let Some(account) = guard("mtn", "MTN_TEST_MSISDN") else {
        return;
    };

    let provider = match wave_mtn::MtnProvider::from_env() {
        Ok(provider) => provider,
        Err(err) => {
            assert!(skip_if_unconfigured("mtn", &err), "{err}");
            return;
        }
    };

    let opts = ListOptions {
        limit: Some(5),
        cursor: None,
    };
    let transactions = provider
        .list_transactions(&account, opts)
        .await
        .unwrap_or_else(|err| panic!("mtn list_transactions a échoué : {err}"));

    // Une sandbox fraîche peut n'avoir aucune transaction — ce n'est pas
    // une erreur. On vérifie la limite demandée et la cohérence des devises.
    println!("[mtn] {} transaction(s) retournée(s)", transactions.len());
    assert!(transactions.len() <= 5);
    for transaction in &transactions {
        assert_eq!(transaction.amount.currency, provider.currency());
    }
}
