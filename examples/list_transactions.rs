//! Exemple : lister les transactions d'un compte avec n'importe quel provider.
//!
//! ```bash
//! cargo run -p wave-cli --example list_transactions -- mtn +2250700000000 10
//! ```
//!
//! Credentials lus depuis `.env` (voir `.env.example`).

use wave_core::{ListOptions, PhoneNumber, Provider};

fn make_provider(name: &str) -> Result<Box<dyn Provider>, Box<dyn std::error::Error>> {
    Ok(match name {
        "wave" => Box::new(wave_wave::WaveProvider::from_env()?),
        "orange" => Box::new(wave_orange::OrangeProvider::from_env()?),
        "mtn" => Box::new(wave_mtn::MtnProvider::from_env()?),
        "moov" => Box::new(wave_moov::MoovProvider::from_env()?),
        other => return Err(format!("provider inconnu : {other}").into()),
    })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();

    let mut args = std::env::args().skip(1);
    let provider_name = args.next().unwrap_or_else(|| "wave".to_string());
    let account = args.next().unwrap_or_else(|| "+2250700000000".to_string());
    let limit: u32 = args.next().unwrap_or_else(|| "10".to_string()).parse()?;

    let provider = make_provider(&provider_name)?;
    let account = PhoneNumber::parse(&account)?;

    let opts = ListOptions {
        limit: Some(limit),
        cursor: None,
    };
    let transactions = provider.list_transactions(&account, opts).await?;

    if transactions.is_empty() {
        println!("[{}] aucune transaction.", provider.name());
        return Ok(());
    }
    for tx in transactions {
        println!(
            "[{}] {} — {:?} — {} — {}",
            tx.provider,
            tx.id,
            tx.status,
            tx.amount,
            tx.note.as_deref().unwrap_or("—"),
        );
    }
    Ok(())
}
