#!/usr/bin/env bash
# Démo wave-rs pour asciinema.
#
#   cargo build --release
#   asciinema rec demo/demo.cast --command "bash demo/record.sh"
#   agg demo/demo.cast demo/demo.gif --theme monokai
#
# `wave providers` fonctionne sans credentials. Les autres commandes
# nécessitent un .env rempli — sans lui, elles affichent l'erreur de config,
# ce qui reste une démo honnête du comportement du binaire.

set -u

WAVE="./target/release/wave"
ACCOUNT="+2250700000000"

if [ ! -x "$WAVE" ]; then
    echo "Binaire introuvable — lance d'abord : cargo build --release" >&2
    exit 1
fi

# Affiche la commande, marque une pause, puis l'exécute.
run() {
    echo "\$ $*"
    sleep 0.6
    "$@" || true
    sleep 2
}

sleep 1
echo "# wave-rs — un SDK Rust unifié pour le mobile money ouest-africain"
sleep 1.5
echo
run "$WAVE" providers
echo
run "$WAVE" balance --provider wave --account "$ACCOUNT"
echo
run "$WAVE" pay --provider wave --to "$ACCOUNT" --amount 5000 --note "Demo wave-rs"
echo
run "$WAVE" balance --provider wave --account "$ACCOUNT" --output json
echo
echo "# github.com/varanjfhueijd/wave-r"
sleep 2
