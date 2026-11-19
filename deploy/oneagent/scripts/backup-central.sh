#!/usr/bin/env bash
# Backup do PostgreSQL do OneHub: somente leitura no banco de origem. Gera dump custom, checksum SHA-256 e verifica
# que o arquivo abre (pg_restore --list). Retenção: mantém os N mais recentes.
set -euo pipefail
DB="${ONEAGENT_DB_NAME:-oneagent_central}"
DEST="${ONEAGENT_BACKUP_DIR:-/var/backups/oneagent/db}"
KEEP="${ONEAGENT_BACKUP_KEEP:-14}"
STAMP="$(date -u +%Y%m%d-%H%M%S)"
umask 077
mkdir -p "$DEST"
FILE="$DEST/${DB}-${STAMP}.dump"
# pg_dump roda como postgres e a saída vai para um arquivo do root (600)
su postgres -c "pg_dump --format=custom --no-owner --no-privileges '$DB'" > "$FILE"
sha256sum "$FILE" > "$FILE.sha256"
pg_restore --list "$FILE" > /dev/null          # falha se o arquivo não for um dump válido
echo "backup: $FILE ($(stat -c %s "$FILE") bytes) sha256=$(cut -d' ' -f1 "$FILE.sha256")"
# retenção: mantém os KEEP mais recentes
ls -1t "$DEST"/*.dump 2>/dev/null | tail -n +$((KEEP + 1)) | while read -r old; do
  rm -f -- "$old" "$old.sha256"; echo "retenção: removido $(basename "$old")"
done
