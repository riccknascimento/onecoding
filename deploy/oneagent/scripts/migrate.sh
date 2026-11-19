#!/usr/bin/env bash
# Aplica migrations pendentes no banco PRINCIPAL. Exige confirmação explícita. Faz backup antes.
set -euo pipefail
[ "${ONEAGENT_CONFIRM_PRINCIPAL_MIGRATE:-}" = "sim" ] || { echo "recusado: defina ONEAGENT_CONFIRM_PRINCIPAL_MIGRATE=sim (autorização explícita)"; exit 1; }
cd "$(dirname "$0")/.."
./scripts/backup-central.sh
docker compose run --rm --no-deps central migrate
