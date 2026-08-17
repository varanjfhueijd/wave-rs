//! Exemple : envoyer un paiement avec n'importe quel provider.
//!
//! ```bash
//! cargo run -p wave-cli --example send_payment -- wave +2250700000000 5000 "Loyer"
//! ```
//!
//! Credentials lus depuis `.env` (voir `.env.example`).

use wave_core::{Money, PaymentRequest, PhoneNumber, Provider};

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
    let to = args.next().unwrap_or_else(|| "+2250700000000".to_string());
    let amount: u64 = args.next().unwrap_or_else(|| "5000".to_string()).parse()?;
    let note = args.next();

    let provider = make_provider(&provider_name)?;
    let to = PhoneNumber::parse(&to)?;

    let mut request = PaymentRequest::new(to, Money::xof(amount));
    if let Some(note) = note {
        request = request.with_note(note);
    }

    let response = provider.initiate_payment(request).await?;
    println!(
        "[{}] transaction {} — statut {:?}",
        response.provider, response.transaction_id, response.status
    );
    Ok(())
}
