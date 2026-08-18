# AGENT_CONTEXT.md — wave-rs

## Vue d'ensemble du projet

`wave-rs` est un SDK Rust unifié pour les APIs de paiement mobile en Afrique de l'Ouest
(Wave, Orange Money, MTN Mobile Money, Moov Africa).
Il expose une interface commune, typée et async pour initier des paiements,
consulter des soldes et récupérer l'historique des transactions,
quel que soit l'opérateur sous-jacent.

Le projet inclut également un binaire CLI (`wave`) pour tester le SDK en terminal.

---

## Architecture du projet

```
wave-rs/
├── CLAUDE.md                  ← ce fichier
├── Cargo.toml                 ← workspace root
├── Cargo.lock
├── README.md
├── crates/
│   ├── wave-core/             ← types partagés, traits, erreurs
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── error.rs       ← WaveError enum exhaustif
│   │       ├── types.rs       ← Money, PhoneNumber, Transaction, etc.
│   │       └── provider.rs    ← trait Provider (async)
│   ├── wave-wave/             ← implémentation Wave
│   │   ├── Cargo.toml
│   │   └── src/lib.rs
│   ├── wave-orange/           ← implémentation Orange Money
│   │   ├── Cargo.toml
│   │   └── src/lib.rs
│   ├── wave-mtn/              ← implémentation MTN MoMo
│   │   ├── Cargo.toml
│   │   └── src/lib.rs
│   └── wave-moov/             ← implémentation Moov Africa
│       ├── Cargo.toml
│       └── src/lib.rs
├── cli/                       ← binaire CLI
│   ├── Cargo.toml
│   └── src/main.rs
├── dashboard/                 ← dashboard web local (étape 9, axum)
│   ├── Cargo.toml
│   ├── assets/                ← head.html + wiring.js injectés dans la page
│   └── src/main.rs
├── examples/                  ← exemples d'usage (un fichier par use case)
│   ├── send_payment.rs
│   ├── check_balance.rs
│   └── list_transactions.rs
└── tests/                     ← tests d'intégration avec mock servers
    └── integration/
```

---

## Principes non négociables

### Rust idiomatique — toujours
- Utilise `thiserror` pour les erreurs, jamais `Box<dyn Error>` en surface publique
- Utilise `async/await` avec Tokio partout — pas de code bloquant dans les providers
- Chaque type public doit implémenter `Debug`, `Clone`, `Serialize`, `Deserialize` si pertinent
- Pas de `unwrap()` ni de `expect()` dans le code de bibliothèque — uniquement dans les tests
- Préfère `?` pour la propagation d'erreurs

### Trait Provider — le contrat central
Toute implémentation d'opérateur DOIT implémenter ce trait exactement :

```rust
#[async_trait]
pub trait Provider: Send + Sync {
    fn name(&self) -> &'static str;
    fn currency(&self) -> Currency;  // XOF pour UEMOA

    async fn initiate_payment(&self, request: PaymentRequest)
        -> Result<PaymentResponse, WaveError>;

    async fn check_balance(&self, account: &PhoneNumber)
        -> Result<Money, WaveError>;

    async fn get_transaction(&self, id: &TransactionId)
        -> Result<Transaction, WaveError>;

    async fn list_transactions(&self, account: &PhoneNumber, opts: ListOptions)
        -> Result<Vec<Transaction>, WaveError>;
}
```

Ne modifie JAMAIS la signature de ce trait sans me demander d'abord.

### Gestion des erreurs
Utilise cette hiérarchie d'erreurs dans `wave-core/src/error.rs` :

```rust
#[derive(Debug, thiserror::Error)]
pub enum WaveError {
    #[error("Erreur réseau : {0}")]
    Network(#[from] reqwest::Error),

    #[error("Authentification refusée par {provider}")]
    AuthFailed { provider: String },

    #[error("Solde insuffisant")]
    InsufficientFunds,

    #[error("Numéro de téléphone invalide : {number}")]
    InvalidPhoneNumber { number: String },

    #[error("Timeout après {seconds}s")]
    Timeout { seconds: u64 },

    #[error("Rate limit atteint, réessayer après {retry_after_secs}s")]
    RateLimited { retry_after_secs: u64 },

    #[error("Erreur API {provider} [{code}] : {message}")]
    ApiError {
        provider: String,
        code: String,
        message: String,
    },

    #[error("Erreur de sérialisation : {0}")]
    Serialization(#[from] serde_json::Error),
}
```

---

## Crates autorisés (liste approuvée)

| Crate | Usage | Raison |
|-------|-------|--------|
| `tokio` | Runtime async | Standard de facto |
| `reqwest` | Client HTTP | Features: json, rustls-tls |
| `serde` + `serde_json` | Sérialisation | Universel |
| `thiserror` | Dérivation d'erreurs | Ergonomique |
| `async-trait` | Traits async | Nécessaire jusqu'à stabilisation |
| `clap` | CLI (derive feature) | Standard CLI Rust |
| `tracing` + `tracing-subscriber` | Logs structurés | Pas `println!` |
| `wiremock` | Mock HTTP en tests | Tests d'intégration |
| `mockall` | Mocking de traits | Tests unitaires |
| `dotenvy` | Lecture .env | Config credentials |
| `phonenumber` | Validation E.164 | Numéros africains |
| `axum` | Serveur HTTP du dashboard local (étape 9) | Standard Tokio, approuvé le 2026-08-10 |
| `comfy-table` | Tableaux CLI | Pré-autorisé section CLI |

**N'ajoute JAMAIS un crate sans me le proposer d'abord avec justification.**

---

## Conventions de code

### Nommage
- Types publics : `PascalCase`
- Fonctions et méthodes : `snake_case`
- Constantes : `SCREAMING_SNAKE_CASE`
- Modules : `snake_case`
- Fichiers : `snake_case.rs`

### Numéros de téléphone
Toujours valider en format E.164 avec le crate `phonenumber`.
Les numéros CI commencent par `+225`, SN par `+221`, etc.
Ne jamais stocker un numéro non validé dans un type `PhoneNumber`.

### Montants monétaires
Utilise TOUJOURS des entiers (centimes/francs entiers) — jamais de `f64` pour l'argent.
Le type `Money { amount: u64, currency: Currency }` est obligatoire.

### Logs
- Utilise `tracing::info!`, `tracing::warn!`, `tracing::error!`
- Jamais `println!` dans le code de bibliothèque
- Niveau `debug` pour les requêtes HTTP sortantes (sans credentials)
- Niveau `trace` pour les corps de réponse bruts

---

## Variables d'environnement (fichier .env en dev)

```env
# Wave
WAVE_API_KEY=
WAVE_MERCHANT_ID=
WAVE_SANDBOX=true

# Orange Money CI
ORANGE_CLIENT_ID=
ORANGE_CLIENT_SECRET=
ORANGE_SANDBOX=true

# MTN MoMo
MTN_SUBSCRIPTION_KEY=
MTN_API_USER=
MTN_API_KEY=
MTN_SANDBOX=true

# Moov Africa
MOOV_API_KEY=
MOOV_SANDBOX=true
```

---

## CLI — commandes attendues

```bash
# Envoyer un paiement
wave pay --provider wave --to +2250700000000 --amount 5000 --note "Loyer"

# Vérifier un solde
wave balance --provider orange --account +2250700000000

# Lister les transactions
wave transactions --provider mtn --account +2250700000000 --limit 10

# Vérifier le statut d'une transaction
wave status --provider moov --id txn_abc123

# Lister les providers disponibles
wave providers
```

La CLI doit afficher les résultats en tableau coloré (crate `colored` ou `comfy-table`)
et supporter `--output json` pour la sortie machine-readable.

---

## Tests — règles strictes

1. **Chaque provider a son propre mock server** basé sur `wiremock`
2. **Zéro appel réseau réel** dans les tests unitaires et d'intégration
3. **Nommage des tests** : `test_<provider>_<action>_<scenario>`
   Exemple : `test_wave_payment_insufficient_funds`
4. **Coverage minimum attendu** : 80% sur `wave-core`, 70% sur chaque provider
5. Les fixtures JSON des réponses API vont dans `tests/fixtures/<provider>/`

---

## Ce que tu NE dois PAS faire

- Ne jamais logger les credentials ou tokens (même en mode debug)
- Ne jamais utiliser `reqwest::blocking` — tout doit être async
- Ne jamais `clone()` un `PhoneNumber` sans raison — préfère les références
- Ne jamais `unwrap()` sur une réponse HTTP — gère toujours les status codes non-200
- Ne jamais committer un `.env` avec de vraies clés
- Ne pas créer de nouveaux modules sans suivre la structure ci-dessus

---

## Priorités d'implémentation (ordre)

1. `wave-core` : types + trait Provider + WaveError — doit compiler proprement
2. `wave-wave` : implémentation Wave (plus documentée, commencer ici)
3. CLI de base : `pay`, `balance`, `providers`
4. `wave-orange` : Orange Money CI
5. `wave-mtn` : MTN MoMo (API publique disponible en sandbox)
6. `wave-moov` : Moov Africa
7. Tests d'intégration complets
8. Documentation `cargo doc` + README
9. Dashboard web (post-MVP, hors CLI) : brancher la maquette
   `wave_rs_dashboard.html` sur le SDK via une petite API HTTP locale
   (crate à définir — à spécifier ensemble une fois le MVP CLI livré)

---

## Commandes utiles

```bash
# Build complet
cargo build --workspace

# Tests
cargo test --workspace

# Lint strict
cargo clippy --workspace -- -D warnings

# Formatage
cargo fmt --all

# Documentation
cargo doc --workspace --no-deps --open

# Vérifier les dépendances outdated
cargo outdated

# Audit de sécurité
cargo audit
```

---

## Contexte métier important

Les APIs de paiement mobile en Afrique de l'Ouest ont des particularités :
- Les réponses peuvent être **asynchrones** : le paiement est "en cours" et le statut final arrive via webhook ou polling
- Les **timeouts** sont longs (jusqu'à 90s) car ils attendent la confirmation de l'utilisateur sur son téléphone
- Le **sandbox** a des comportements différents de la production (certains codes d'erreur n'existent qu'en prod)
- Les numéros de téléphone locaux existent en format court (07 00 00 00 00) mais doivent être normalisés en E.164

Tiens compte de ces réalités dans chaque implémentation.
