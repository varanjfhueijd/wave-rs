//! Exemple : consulter un solde avec n'importe quel provider.
//!
//! ```bash
//! cargo run -p wave-cli --example check_balance -- orange +2250700000000
//! ```
//!
//! Credentials lus depuis `.env` (voir `.env.example`).

use wave_core::{PhoneNumber, Provider};

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

    let provider = make_provider(&provider_name)?;
    let account = PhoneNumber::parse(&account)?;

    let balance = provider.check_balance(&account).await?;
    println!("[{}] solde de {account} : {balance}", provider.name());
    Ok(())
}
