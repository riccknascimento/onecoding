#!/usr/bin/env bash
# Publicação ordenada do OneCoding no VPS. Ordem: configuração e banco são copiados ANTES de qualquer mudança;
# a imagem anterior é guardada como "previous"; migrations só com ONEAGENT_APPLY_MIGRATIONS=1; falha no healthcheck
# dispara o rollback automático.
set -euo pipefail
cd "$(dirname "$0")/.."
STAMP="$(date -u +%Y%m%d-%H%M%S)"
ENV_FILE="$(. ./.env 2>/dev/null; echo "${CENTRAL_ENV_FILE:-/etc/oneagent-central/central.env}")"
[ -f "$ENV_FILE" ] || { echo "recusado: arquivo de ambiente ausente ($ENV_FILE). Nada foi alterado."; exit 1; }
CONF_BAK="/var/backups/oneagent/config/$STAMP"
mkdir -p "$CONF_BAK" && chmod 700 "$CONF_BAK"
cp -a "$ENV_FILE" "$CONF_BAK/central.env"
cp -a caddy "$CONF_BAK/caddy"
echo "configuração guardada em $CONF_BAK"
./scripts/backup-central.sh
if docker image inspect oneagent/central:current >/dev/null 2>&1; then
  docker tag oneagent/central:current oneagent/central:previous
fi
docker compose build central
if [ "${ONEAGENT_APPLY_MIGRATIONS:-0}" = "1" ]; then
  echo "aplicando migrations pendentes (autorizado)"
  docker compose run --rm --no-deps central migrate
fi
docker compose up -d
for i in $(seq 1 24); do
  state=$(docker inspect -f '{{.State.Health.Status}}' "$(docker compose ps -q central)" 2>/dev/null || echo none)
  if [ "$state" = healthy ]; then echo "central saudável"; exit 0; fi
  sleep 5
done
echo "central não ficou saudável: rollback automático"
./scripts/rollback.sh
exit 1
