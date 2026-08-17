//! CLI `wave` — teste le SDK wave-rs en terminal.
//!
//! ```bash
//! wave pay --provider wave --to +2250700000000 --amount 5000 --note "Loyer"
//! wave balance --provider wave --account +2250700000000
//! wave transactions --provider wave --account +2250700000000 --limit 10
//! wave status --provider wave --id txn_abc123
//! wave providers
//! ```
//!
//! Sortie en tableau par défaut, `--output json` pour du machine-readable.

use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use comfy_table::presets::UTF8_FULL_CONDENSED;
use comfy_table::{Cell, Color, Table};
use serde::Serialize;
use wave_core::{
    ListOptions, Money, PaymentRequest, PhoneNumber, Provider, Transaction, TransactionId,
    TransactionStatus,
};
use wave_moov::MoovProvider;
use wave_mtn::MtnProvider;
use wave_orange::OrangeProvider;
use wave_wave::WaveProvider;

#[derive(Parser)]
#[command(
    name = "wave",
    version,
    about = "SDK unifié de paiement mobile — Wave, Orange Money, MTN MoMo, Moov Africa"
)]
struct Cli {
    /// Format de sortie
    #[arg(long, global = true, value_enum, default_value_t = OutputFormat::Table)]
    output: OutputFormat,

    #[command(subcommand)]
    command: Command,
}

#[derive(Copy, Clone, ValueEnum)]
enum OutputFormat {
    Table,
    Json,
}

#[derive(Subcommand)]
enum Command {
    /// Envoyer un paiement
    Pay {
        /// Provider à utiliser (wave, orange, mtn, moov)
        #[arg(long)]
        provider: String,
        /// Numéro destinataire au format E.164 (+2250700000000)
        #[arg(long)]
        to: String,
        /// Montant en francs CFA entiers
        #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
        amount: u64,
        /// Note libre attachée au paiement
        #[arg(long)]
        note: Option<String>,
        /// Référence idempotente côté client
        #[arg(long)]
        reference: Option<String>,
    },
    /// Consulter le solde d'un compte
    Balance {
        #[arg(long)]
        provider: String,
        /// Numéro du compte au format E.164
        #[arg(long)]
        account: String,
    },
    /// Lister les transactions d'un compte
    Transactions {
        #[arg(long)]
        provider: String,
        #[arg(long)]
        account: String,
        /// Nombre maximum de transactions
        #[arg(long)]
        limit: Option<u32>,
        /// Curseur de pagination retourné par l'appel précédent
        #[arg(long)]
        cursor: Option<String>,
    },
    /// Vérifier le statut d'une transaction
    Status {
        #[arg(long)]
        provider: String,
        /// Identifiant de la transaction (ex. txn_abc123)
        #[arg(long)]
        id: String,
    },
    /// Lister les providers disponibles
    Providers,
}

#[tokio::main]
async fn main() -> ExitCode {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();
    match run(cli).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("Erreur : {message}");
            ExitCode::FAILURE
        }
    }
}

async fn run(cli: Cli) -> Result<(), String> {
    match cli.command {
        Command::Pay {
            provider,
            to,
            amount,
            note,
            reference,
        } => {
            let provider = make_provider(&provider)?;
            let to = parse_phone(&to)?;
            let mut request = PaymentRequest::new(to, Money::xof(amount));
            if let Some(note) = note {
                request = request.with_note(note);
            }
            if let Some(reference) = reference {
                request = request.with_reference(reference);
            }
            let response = provider
                .initiate_payment(request)
                .await
                .map_err(|e| e.to_string())?;

            match cli.output {
                OutputFormat::Json => print_json(&response),
                OutputFormat::Table => {
                    let mut table = new_table(vec!["Transaction", "Statut", "Provider"]);
                    table.add_row(vec![
                        Cell::new(response.transaction_id.as_str()),
                        status_cell(response.status),
                        Cell::new(&response.provider),
                    ]);
                    println!("{table}");
                    if response.status == TransactionStatus::Pending {
                        println!(
                            "Paiement en attente de confirmation — suivre avec : \
                             wave status --provider {} --id {}",
                            response.provider, response.transaction_id
                        );
                    }
                    Ok(())
                }
            }
        }

        Command::Balance { provider, account } => {
            let provider = make_provider(&provider)?;
            let account = parse_phone(&account)?;
            let balance = provider
                .check_balance(&account)
                .await
                .map_err(|e| e.to_string())?;

            match cli.output {
                OutputFormat::Json => print_json(&balance),
                OutputFormat::Table => {
                    let mut table = new_table(vec!["Compte", "Solde"]);
                    table.add_row(vec![
                        Cell::new(account.as_str()),
                        Cell::new(format_money(&balance)).fg(Color::Green),
                    ]);
                    println!("{table}");
                    Ok(())
                }
            }
        }

        Command::Transactions {
            provider,
            account,
            limit,
            cursor,
        } => {
            let provider = make_provider(&provider)?;
            let account = parse_phone(&account)?;
            let opts = ListOptions { limit, cursor };
            let transactions = provider
                .list_transactions(&account, opts)
                .await
                .map_err(|e| e.to_string())?;

            match cli.output {
                OutputFormat::Json => print_json(&transactions),
                OutputFormat::Table => {
                    print_transactions_table(&transactions);
                    Ok(())
                }
            }
        }

        Command::Status { provider, id } => {
            let provider = make_provider(&provider)?;
            let id = TransactionId::from(id.as_str());
            let transaction = provider
                .get_transaction(&id)
                .await
                .map_err(|e| e.to_string())?;

            match cli.output {
                OutputFormat::Json => print_json(&transaction),
                OutputFormat::Table => {
                    print_transactions_table(std::slice::from_ref(&transaction));
                    Ok(())
                }
            }
        }

        Command::Providers => {
            let providers = provider_catalog();
            match cli.output {
                OutputFormat::Json => print_json(&providers),
                OutputFormat::Table => {
                    let mut table = new_table(vec!["Provider", "Devise", "Statut"]);
                    for info in providers {
                        let status = if info.available {
                            Cell::new("disponible").fg(Color::Green)
                        } else {
                            Cell::new("à venir").fg(Color::Yellow)
                        };
                        table.add_row(vec![Cell::new(info.name), Cell::new(info.currency), status]);
                    }
                    println!("{table}");
                    Ok(())
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Sélection de provider
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct ProviderInfo {
    name: &'static str,
    currency: &'static str,
    available: bool,
}

fn provider_catalog() -> Vec<ProviderInfo> {
    vec![
        ProviderInfo {
            name: "wave",
            currency: "XOF",
            available: true,
        },
        ProviderInfo {
            name: "orange",
            currency: "XOF",
            available: true,
        },
        ProviderInfo {
            name: "mtn",
            currency: "XOF",
            available: true,
        },
        ProviderInfo {
            name: "moov",
            currency: "XOF",
            available: true,
        },
    ]
}

fn make_provider(name: &str) -> Result<Box<dyn Provider>, String> {
    match name {
        "wave" => WaveProvider::from_env()
            .map(|p| Box::new(p) as Box<dyn Provider>)
            .map_err(|e| {
                format!(
                    "impossible d'initialiser le provider wave : {e}\n\
                     Astuce : renseignez WAVE_API_KEY et WAVE_MERCHANT_ID \
                     (fichier .env ou variables d'environnement)."
                )
            }),
        "orange" => OrangeProvider::from_env()
            .map(|p| Box::new(p) as Box<dyn Provider>)
            .map_err(|e| {
                format!(
                    "impossible d'initialiser le provider orange : {e}\n\
                     Astuce : renseignez ORANGE_CLIENT_ID et ORANGE_CLIENT_SECRET \
                     (fichier .env ou variables d'environnement)."
                )
            }),
        "mtn" => MtnProvider::from_env()
            .map(|p| Box::new(p) as Box<dyn Provider>)
            .map_err(|e| {
                format!(
                    "impossible d'initialiser le provider mtn : {e}\n\
                     Astuce : renseignez MTN_SUBSCRIPTION_KEY, MTN_API_USER et MTN_API_KEY \
                     (fichier .env ou variables d'environnement)."
                )
            }),
        "moov" => MoovProvider::from_env()
            .map(|p| Box::new(p) as Box<dyn Provider>)
            .map_err(|e| {
                format!(
                    "impossible d'initialiser le provider moov : {e}\n\
                     Astuce : renseignez MOOV_API_KEY \
                     (fichier .env ou variables d'environnement)."
                )
            }),
        _ => Err(format!(
            "provider inconnu : '{name}' — disponibles : wave, orange, mtn, moov"
        )),
    }
}

// ---------------------------------------------------------------------------
// Rendu
// ---------------------------------------------------------------------------

fn parse_phone(input: &str) -> Result<PhoneNumber, String> {
    PhoneNumber::parse(input).map_err(|_| {
        format!(
            "numéro de téléphone invalide : '{input}' — format attendu E.164, ex. +2250700000000"
        )
    })
}

fn print_json<T: Serialize>(value: &T) -> Result<(), String> {
    let json = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    println!("{json}");
    Ok(())
}

fn new_table(headers: Vec<&str>) -> Table {
    let mut table = Table::new();
    table.load_preset(UTF8_FULL_CONDENSED);
    table.set_header(headers);
    table
}

fn print_transactions_table(transactions: &[Transaction]) {
    if transactions.is_empty() {
        println!("Aucune transaction.");
        return;
    }
    let mut table = new_table(vec![
        "ID",
        "Statut",
        "Montant",
        "Contrepartie",
        "Note",
        "Date",
    ]);
    for tx in transactions {
        table.add_row(vec![
            Cell::new(tx.id.as_str()),
            status_cell(tx.status),
            Cell::new(format_money(&tx.amount)),
            Cell::new(
                tx.counterparty
                    .as_ref()
                    .map(PhoneNumber::as_str)
                    .unwrap_or("—"),
            ),
            Cell::new(tx.note.as_deref().unwrap_or("—")),
            Cell::new(&tx.created_at),
        ]);
    }
    println!("{table}");
}

fn status_cell(status: TransactionStatus) -> Cell {
    let (label, color) = match status {
        TransactionStatus::Pending => ("en attente", Color::Yellow),
        TransactionStatus::Successful => ("réussie", Color::Green),
        TransactionStatus::Failed => ("échouée", Color::Red),
        TransactionStatus::Cancelled => ("annulée", Color::Red),
        TransactionStatus::Expired => ("expirée", Color::DarkGrey),
    };
    Cell::new(label).fg(color)
}

/// `5000` → `5 000 XOF` — séparateur de milliers pour la lisibilité.
fn format_money(money: &Money) -> String {
    let digits = money.amount.to_string();
    let mut formatted = String::with_capacity(digits.len() + digits.len() / 3 + 4);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            formatted.push(' ');
        }
        formatted.push(c);
    }
    formatted.push(' ');
    formatted.push_str(&money.currency.to_string());
    formatted
}
