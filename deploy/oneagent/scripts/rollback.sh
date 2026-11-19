#!/usr/bin/env bash
# Volta a imagem anterior do Central e a configuração guardada no último deploy. Não restaura o banco:
# migrations não são revertidas automaticamente (ver docs). Restauração de banco exige restore-check/aprovação.
set -euo pipefail
cd "$(dirname "$0")/.."
docker image inspect oneagent/central:previous >/dev/null 2>&1 || { echo "sem imagem anterior"; exit 1; }
docker tag oneagent/central:previous oneagent/central:current
docker compose up -d --no-deps central
echo "rollback: imagem anterior reativada. Configuração anterior em /var/backups/oneagent/config/ (restaurar manualmente, se necessário)."
