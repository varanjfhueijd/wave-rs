# wave-rs

**🇫🇷 Français** · [🇬🇧 English](#-english)

SDK Rust unifié pour les APIs de paiement mobile en Afrique de l'Ouest :
**Wave**, **Orange Money**, **MTN Mobile Money** et **Moov Africa**.

Une interface commune, typée et async (Tokio) pour initier des paiements,
consulter des soldes et lister des transactions — quel que soit l'opérateur.
Inclut une CLI (`wave`) pour tester le SDK en terminal.

## Structure du workspace

| Crate | Rôle | État |
|-------|------|------|
| [`crates/wave-core`](crates/wave-core) | Types partagés, trait `Provider`, `WaveError` | ✅ |
| [`crates/wave-wave`](crates/wave-wave) | Provider Wave (clé API) | ✅ |
| [`crates/wave-orange`](crates/wave-orange) | Provider Orange Money CI (OAuth2 + cache token) | ✅ |
| [`crates/wave-mtn`](crates/wave-mtn) | Provider MTN MoMo (flux 202 + polling) | ✅ |
| [`crates/wave-moov`](crates/wave-moov) | Provider Moov Africa (clé API) | ✅ |
| [`cli`](cli) | Binaire `wave` (pay, balance, transactions, status, providers) | ✅ |
| [`tests`](tests) | Tests d'intégration transverses (`dyn Provider`) | ✅ |
| [`dashboard`](dashboard) | Dashboard web local branché sur le SDK (axum) | ✅ |

## Démarrage

```bash
cp .env.example .env        # puis remplir les credentials
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

## Utiliser le SDK

Le contrat central est le trait `Provider` : le même code appelant
fonctionne avec les quatre opérateurs.

```rust
use wave_core::{Money, PaymentRequest, PhoneNumber, Provider};
use wave_wave::WaveProvider;

#[tokio::main]
async fn main() -> Result<(), wave_core::WaveError> {
    let provider = WaveProvider::from_env()?; // WAVE_API_KEY, WAVE_MERCHANT_ID

    let to = PhoneNumber::parse("+2250700000000")?;
    let request = PaymentRequest::new(to, Money::xof(5000)).with_note("Loyer");

    let response = provider.initiate_payment(request).await?;
    // Paiements asynchrones : statut initial `Pending`, suivi par polling.
    println!("{} → {:?}", response.transaction_id, response.status);
    Ok(())
}
```

Exemples complets dans [`examples/`](examples) :

```bash
cargo run -p wave-cli --example send_payment      -- wave  +2250700000000 5000 "Loyer"
cargo run -p wave-cli --example check_balance     -- orange +2250700000000
cargo run -p wave-cli --example list_transactions -- mtn   +2250700000000 10
```

## CLI

```bash
wave pay --provider wave --to +2250700000000 --amount 5000 --note "Loyer"
wave balance --provider orange --account +2250700000000
wave transactions --provider mtn --account +2250700000000 --limit 10
wave status --provider moov --id txn_abc123
wave providers
```

Sortie en tableau coloré par défaut, `--output json` pour du machine-readable.

## Dashboard web

```bash
cargo run -p wave-dashboard        # puis ouvrir http://127.0.0.1:8787
```

Sert la maquette branchée sur le SDK : disponibilité réelle des providers,
envoi de paiements et historique des transactions via l'API locale
(`/api/providers`, `/api/pay`, `/api/balance`, `/api/transactions`,
`/api/status`). Écoute uniquement sur `127.0.0.1` — aucune exposition réseau.
Les providers sans credentials dans `.env` apparaissent « non configurés ».

## Configuration (.env)

Copier [`.env.example`](.env.example) vers `.env`. Chaque provider lit ses
propres variables (`WAVE_*`, `ORANGE_*`, `MTN_*`, `MOOV_*`). Deux conventions
communes :

- **Sandbox par défaut** : tant que `<PROVIDER>_SANDBOX` ne vaut pas
  explicitement `false`, on ne touche jamais la production.
- `<PROVIDER>_BASE_URL` (optionnel) surcharge l'URL de l'API — pratique pour
  les mocks locaux et les environnements de test.

Les secrets ne sont jamais loggés : les implémentations `Debug` des configs
les masquent (`***`).

## Tests

```bash
cargo test --workspace
```

Chaque provider a son propre mock server `wiremock` (zéro appel réseau réel),
ses fixtures JSON dans `tests/fixtures/<provider>/`, et le nommage suit
`test_<provider>_<action>_<scenario>`. Les tests transverses de
[`tests/integration`](tests/integration) vérifient l'interchangeabilité des
quatre providers derrière `dyn Provider`, et montrent comment mocker le trait
avec `mockall` côté code consommateur.

## Particularités métier gérées

- **Paiements asynchrones** : statut initial `Pending`, statut final par
  polling (`get_transaction`) ou webhook côté opérateur.
- **Timeouts longs** (90s par défaut) : l'API attend la confirmation de
  l'utilisateur sur son téléphone.
- **Numéros E.164** : validation stricte via le crate `phonenumber` ; les
  formats courts locaux (`07 00 00 00 00`) sont normalisés avec
  `PhoneNumber::parse_with_region("CI", ...)` ; les MSISDN sans `+` de MTN
  sont normalisés automatiquement.
- **Montants entiers** : `Money { amount: u64, currency }` — jamais de
  flottant pour l'argent ; les montants-chaînes de MTN sont convertis.

---



[🇫🇷 Français](#wave-rs) · **English**

Unified Rust SDK for West African mobile money APIs:
**Wave**, **Orange Money**, **MTN Mobile Money** and **Moov Africa**.

One common, typed, async (Tokio) interface to initiate payments, check
balances and list transactions — whatever the underlying operator.
Ships with a CLI (`wave`) to try the SDK from the terminal.

## Workspace layout

| Crate | Purpose | Status |
|-------|---------|--------|
| [`crates/wave-core`](crates/wave-core) | Shared types, `Provider` trait, `WaveError` | ✅ |
| [`crates/wave-wave`](crates/wave-wave) | Wave provider (API key) | ✅ |
| [`crates/wave-orange`](crates/wave-orange) | Orange Money CI provider (OAuth2 + token cache) | ✅ |
| [`crates/wave-mtn`](crates/wave-mtn) | MTN MoMo provider (202 flow + polling) | ✅ |
| [`crates/wave-moov`](crates/wave-moov) | Moov Africa provider (API key) | ✅ |
| [`cli`](cli) | `wave` binary (pay, balance, transactions, status, providers) | ✅ |
| [`tests`](tests) | Cross-provider integration tests (`dyn Provider`) | ✅ |
| [`dashboard`](dashboard) | Local web dashboard wired to the SDK (axum) | ✅ |

## Getting started

```bash
cp .env.example .env        # then fill in your credentials
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

## Using the SDK

The central contract is the `Provider` trait: the same calling code works
with all four operators.

```rust
use wave_core::{Money, PaymentRequest, PhoneNumber, Provider};
use wave_wave::WaveProvider;

#[tokio::main]
async fn main() -> Result<(), wave_core::WaveError> {
    let provider = WaveProvider::from_env()?; // WAVE_API_KEY, WAVE_MERCHANT_ID

    let to = PhoneNumber::parse("+2250700000000")?;
    let request = PaymentRequest::new(to, Money::xof(5000)).with_note("Rent");

    let response = provider.initiate_payment(request).await?;
    // Payments are asynchronous: initial status is `Pending`, poll for the final one.
    println!("{} → {:?}", response.transaction_id, response.status);
    Ok(())
}
```

Full examples live in [`examples/`](examples):

```bash
cargo run -p wave-cli --example send_payment      -- wave  +2250700000000 5000 "Rent"
cargo run -p wave-cli --example check_balance     -- orange +2250700000000
cargo run -p wave-cli --example list_transactions -- mtn   +2250700000000 10
```

## CLI

```bash
wave pay --provider wave --to +2250700000000 --amount 5000 --note "Rent"
wave balance --provider orange --account +2250700000000
wave transactions --provider mtn --account +2250700000000 --limit 10
wave status --provider moov --id txn_abc123
wave providers
```

Colored table output by default, `--output json` for machine-readable output.

## Web dashboard

```bash
cargo run -p wave-dashboard        # then open http://127.0.0.1:8787
```

Serves the mockup wired to the SDK: real provider availability, payment
sending and transaction history through the local API (`/api/providers`,
`/api/pay`, `/api/balance`, `/api/transactions`, `/api/status`). Binds to
`127.0.0.1` only — no network exposure. Providers without credentials in
`.env` show up as "not configured".

## Configuration (.env)

Copy [`.env.example`](.env.example) to `.env`. Each provider reads its own
variables (`WAVE_*`, `ORANGE_*`, `MTN_*`, `MOOV_*`). Two shared conventions:

- **Sandbox by default**: unless `<PROVIDER>_SANDBOX` is explicitly set to
  `false`, production is never touched.
- `<PROVIDER>_BASE_URL` (optional) overrides the API base URL — handy for
  local mocks and test environments.

Secrets are never logged: the configs' `Debug` implementations redact them
(`***`).

## Tests

```bash
cargo test --workspace
```

Every provider has its own `wiremock` mock server (zero real network calls),
JSON fixtures under `tests/fixtures/<provider>/`, and test names follow
`test_<provider>_<action>_<scenario>`. The cross-provider tests in
[`tests/integration`](tests/integration) lock in interchangeability behind
`dyn Provider`, and show how to mock the trait with `mockall` in consumer
code.

## Domain quirks handled

- **Asynchronous payments**: initial status is `Pending`; the final status
  comes from polling (`get_transaction`) or operator-side webhooks.
- **Long timeouts** (90s by default): the API waits for the user to confirm
  on their phone.
- **E.164 phone numbers**: strict validation via the `phonenumber` crate;
  local short formats (`07 00 00 00 00`) are normalized with
  `PhoneNumber::parse_with_region("CI", ...)`; MTN's `+`-less MSISDNs are
  normalized automatically.
- **Integer amounts**: `Money { amount: u64, currency }` — never floats for
  money; MTN's string amounts are converted.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in this work by you, as defined in the Apache-2.0
license, shall be dual licensed as above, without any additional terms or
conditions.
