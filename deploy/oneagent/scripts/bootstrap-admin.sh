#!/usr/bin/env bash
# Cria o administrador inicial (uso único). A senha é lida do terminal (sem eco), nunca gravada em arquivo.
set -euo pipefail
[ "${ONEAGENT_CONFIRM_BOOTSTRAP:-}" = "sim" ] || { echo "recusado: defina ONEAGENT_CONFIRM_BOOTSTRAP=sim (autorização explícita)"; exit 1; }
: "${ONEAGENT_BOOTSTRAP_EMAIL:?defina ONEAGENT_BOOTSTRAP_EMAIL}"
cd "$(dirname "$0")/.."
read -r -s -p "senha do administrador: " PW; echo
printf '%s\n' "$PW" | docker compose run --rm -T --no-deps -e ONEAGENT_BOOTSTRAP_EMAIL="$ONEAGENT_BOOTSTRAP_EMAIL" central bootstrap-admin
unset PW
