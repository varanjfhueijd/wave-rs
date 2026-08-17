//! Dashboard web local de `wave-rs` — étape 9 de la roadmap.
//!
//! Sert la maquette `wave_rs_dashboard.html` branchée sur le SDK via une
//! petite API HTTP locale (axum). Aucune exposition réseau : le serveur
//! n'écoute que sur `127.0.0.1`.
//!
//! ```bash
//! cargo run -p wave-dashboard        # puis ouvrir http://127.0.0.1:8787
//! ```
//!
//! Les providers sont construits au démarrage depuis `.env` : ceux dont les
//! credentials manquent apparaissent « non configurés » dans l'interface.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use wave_core::{
    ListOptions, Money, PaymentRequest, PhoneNumber, Provider, TransactionId, WaveError,
};

const PAGE_HEAD: &str = include_str!("../assets/head.html");
const PAGE_BODY: &str = include_str!("../../wave_rs_dashboard.html");
const PAGE_WIRING: &str = include_str!("../assets/wiring.js");
const LISTEN_ADDR: &str = "127.0.0.1:8787";

struct AppState {
    page: String,
    providers: HashMap<&'static str, Box<dyn Provider>>,
}

type SharedState = Arc<AppState>;

#[derive(Serialize)]
struct ProviderInfo {
    name: &'static str,
    currency: &'static str,
    available: bool,
}

#[derive(Serialize)]
struct ErrorReply {
    error: String,
}

#[derive(Deserialize)]
struct AccountQuery {
    provider: String,
    account: String,
    limit: Option<u32>,
}

#[derive(Deserialize)]
struct StatusQuery {
    provider: String,
    id: String,
}

#[derive(Deserialize)]
struct PayBody {
    provider: String,
    to: String,
    amount: u64,
    note: Option<String>,
    reference: Option<String>,
}

/// Erreur HTTP de l'API locale, construite depuis une [`WaveError`] ou une
/// erreur de requête (provider inconnu, numéro invalide, ...).
struct ApiError {
    status: StatusCode,
    message: String,
}

impl From<WaveError> for ApiError {
    fn from(err: WaveError) -> Self {
        let status = match &err {
            WaveError::InvalidPhoneNumber { .. } => StatusCode::BAD_REQUEST,
            WaveError::InsufficientFunds => StatusCode::CONFLICT,
            WaveError::AuthFailed { .. } => StatusCode::BAD_GATEWAY,
            WaveError::RateLimited { .. } => StatusCode::TOO_MANY_REQUESTS,
            _ => StatusCode::BAD_GATEWAY,
        };
        Self {
            status,
            message: err.to_string(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ErrorReply {
                error: self.message,
            }),
        )
            .into_response()
    }
}

fn bad_request(message: impl Into<String>) -> ApiError {
    ApiError {
        status: StatusCode::BAD_REQUEST,
        message: message.into(),
    }
}

/// Construit tous les providers configurables depuis l'environnement.
/// Un provider sans credentials est simplement absent de la map.
fn build_providers() -> HashMap<&'static str, Box<dyn Provider>> {
    let mut providers: HashMap<&'static str, Box<dyn Provider>> = HashMap::new();

    match wave_wave::WaveProvider::from_env() {
        Ok(p) => {
            providers.insert("wave", Box::new(p));
        }
        Err(e) => tracing::info!("provider wave non configuré : {e}"),
    }
    match wave_orange::OrangeProvider::from_env() {
        Ok(p) => {
            providers.insert("orange", Box::new(p));
        }
        Err(e) => tracing::info!("provider orange non configuré : {e}"),
    }
    match wave_mtn::MtnProvider::from_env() {
        Ok(p) => {
            providers.insert("mtn", Box::new(p));
        }
        Err(e) => tracing::info!("provider mtn non configuré : {e}"),
    }
    match wave_moov::MoovProvider::from_env() {
        Ok(p) => {
            providers.insert("moov", Box::new(p));
        }
        Err(e) => tracing::info!("provider moov non configuré : {e}"),
    }

    providers
}

impl AppState {
    fn provider(&self, name: &str) -> Result<&dyn Provider, ApiError> {
        self.providers.get(name).map(|p| p.as_ref()).ok_or_else(|| {
            bad_request(format!(
                "provider '{name}' inconnu ou non configuré (credentials manquants dans .env)"
            ))
        })
    }
}

fn parse_phone(input: &str) -> Result<PhoneNumber, ApiError> {
    PhoneNumber::parse(input).map_err(|_| {
        bad_request(format!(
            "numéro invalide : '{input}' (format E.164 attendu)"
        ))
    })
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn page(State(state): State<SharedState>) -> Html<String> {
    Html(state.page.clone())
}

async fn providers(State(state): State<SharedState>) -> Json<Vec<ProviderInfo>> {
    let catalog = ["wave", "orange", "mtn", "moov"]
        .into_iter()
        .map(|name| ProviderInfo {
            name,
            currency: "XOF",
            available: state.providers.contains_key(name),
        })
        .collect();
    Json(catalog)
}

async fn balance(
    State(state): State<SharedState>,
    Query(query): Query<AccountQuery>,
) -> Result<Json<Money>, ApiError> {
    let provider = state.provider(&query.provider)?;
    let account = parse_phone(&query.account)?;
    Ok(Json(provider.check_balance(&account).await?))
}

async fn transactions(
    State(state): State<SharedState>,
    Query(query): Query<AccountQuery>,
) -> Result<Response, ApiError> {
    let provider = state.provider(&query.provider)?;
    let account = parse_phone(&query.account)?;
    let opts = ListOptions {
        limit: query.limit,
        cursor: None,
    };
    let transactions = provider.list_transactions(&account, opts).await?;
    Ok(Json(transactions).into_response())
}

async fn pay(
    State(state): State<SharedState>,
    Json(body): Json<PayBody>,
) -> Result<Response, ApiError> {
    if body.amount == 0 {
        return Err(bad_request("le montant doit être strictement positif"));
    }
    let provider = state.provider(&body.provider)?;
    let to = parse_phone(&body.to)?;

    let mut request = PaymentRequest::new(to, Money::xof(body.amount));
    if let Some(note) = body.note {
        request = request.with_note(note);
    }
    if let Some(reference) = body.reference {
        request = request.with_reference(reference);
    }

    let response = provider.initiate_payment(request).await?;
    Ok(Json(response).into_response())
}

async fn status(
    State(state): State<SharedState>,
    Query(query): Query<StatusQuery>,
) -> Result<Response, ApiError> {
    let provider = state.provider(&query.provider)?;
    let id = TransactionId::from(query.id.as_str());
    let transaction = provider.get_transaction(&id).await?;
    Ok(Json(transaction).into_response())
}

// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let page_html =
        format!("{PAGE_HEAD}{PAGE_BODY}\n<script>\n{PAGE_WIRING}\n</script>\n</body>\n</html>\n");
    let providers_map = build_providers();
    tracing::info!(
        configurés = ?providers_map.keys().collect::<Vec<_>>(),
        "providers construits depuis l'environnement"
    );

    let state: SharedState = Arc::new(AppState {
        page: page_html,
        providers: providers_map,
    });

    let app = Router::new()
        .route("/", get(page))
        .route("/api/providers", get(providers))
        .route("/api/balance", get(balance))
        .route("/api/transactions", get(transactions))
        .route("/api/pay", post(pay))
        .route("/api/status", get(status))
        .with_state(state);

    let listener = match tokio::net::TcpListener::bind(LISTEN_ADDR).await {
        Ok(listener) => listener,
        Err(e) => {
            tracing::error!("impossible d'écouter sur {LISTEN_ADDR} : {e}");
            std::process::exit(1);
        }
    };
    tracing::info!("dashboard wave-rs sur http://{LISTEN_ADDR}");

    if let Err(e) = axum::serve(listener, app).await {
        tracing::error!("serveur arrêté : {e}");
        std::process::exit(1);
    }
}
