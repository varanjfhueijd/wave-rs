# Contributing to wave-rs

Thanks for your interest. The most valuable contribution is **a new operator
provider** — the SDK is designed so that adding one touches nothing else.

## Ground rules

- `cargo fmt --all` and `cargo clippy --workspace --all-targets -- -D warnings`
  must both pass. CI enforces them.
- No `unwrap()` or `expect()` in library code — tests only.
- No real network calls in tests. Every provider ships its own `wiremock`
  mock server.
- Money is always integer minor units (`Money { amount: u64, currency }`).
  Never `f64`.
- Never log credentials, tokens, or full phone numbers.

## Adding a new provider

A provider crate implements exactly four async methods. That's the whole
contract.

### 1. Create the crate

```bash
mkdir -p crates/wave-newop/src
```

Add it to the `members` list in the root [`Cargo.toml`](Cargo.toml), and give
it a `Cargo.toml` that inherits the workspace metadata:

```toml
[package]
name = "wave-newop"
version.workspace = true
edition.workspace = true
license.workspace = true
repository.workspace = true
keywords.workspace = true
categories.workspace = true
description = "NewOp implementation of the wave-rs Provider trait"

[dependencies]
wave-core = { workspace = true }
async-trait = { workspace = true }
reqwest = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
tracing = { workspace = true }

[dev-dependencies]
tokio = { workspace = true }
wiremock = { workspace = true }
```

### 2. Implement the `Provider` trait

```rust
use async_trait::async_trait;
use wave_core::{
    Currency, ListOptions, Money, PaymentRequest, PaymentResponse,
    PhoneNumber, Provider, Transaction, TransactionId, WaveError,
};

pub struct NewOpProvider {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
}

#[async_trait]
impl Provider for NewOpProvider {
    fn name(&self) -> &'static str { "newop" }
    fn currency(&self) -> Currency { Currency::Xof }

    async fn initiate_payment(&self, request: PaymentRequest)
        -> Result<PaymentResponse, WaveError> { todo!() }

    async fn check_balance(&self, account: &PhoneNumber)
        -> Result<Money, WaveError> { todo!() }

    async fn get_transaction(&self, id: &TransactionId)
        -> Result<Transaction, WaveError> { todo!() }

    async fn list_transactions(&self, account: &PhoneNumber, opts: ListOptions)
        -> Result<Vec<Transaction>, WaveError> { todo!() }
}
```

### 3. Map HTTP status codes to `WaveError`

Always check the status **before** deserializing the body. The expected
mapping across all providers:

| HTTP status        | `WaveError` variant                        |
|--------------------|--------------------------------------------|
| 401 / 403          | `AuthFailed { provider }`                   |
| 402 / insufficient | `InsufficientFunds`                         |
| 408 / reqwest timeout | `Timeout { seconds }`                    |
| 429               | `RateLimited { retry_after_secs }` (from `Retry-After`) |
| other 4xx / 5xx    | `ApiError { provider, code, message }`      |

Operator-specific business error codes (e.g. `NOT_ENOUGH_FUNDS` in a 200
body) should be normalized to the same variants — that normalization *is*
the value this SDK provides.

### 4. Write the mock server and tests

Fixtures go in `crates/wave-newop/tests/fixtures/newop/`. Test names follow
`test_<provider>_<action>_<scenario>`:

```
test_newop_payment_success
test_newop_payment_insufficient_funds
test_newop_balance_returns_xof
test_newop_auth_failed
test_newop_rate_limited_reads_retry_after
```

Look at [`crates/wave-wave/tests/wave_provider.rs`](crates/wave-wave/tests/wave_provider.rs)
for the simplest reference (API key auth), or
[`crates/wave-orange`](crates/wave-orange) if your operator uses OAuth2 with
token caching.

### 5. Wire it into the CLI

Add the variant to the provider match in [`cli/src/main.rs`](cli/src/main.rs)
and update the provider table in the [README](README.md).

## Reporting bugs

Include the provider, the sandbox/production mode, and the `WaveError` you
got. **Redact API keys and phone numbers** before pasting logs.

## License

By contributing, you agree that your contributions will be dual licensed
under MIT and Apache-2.0, as described in the [README](README.md#licence).
