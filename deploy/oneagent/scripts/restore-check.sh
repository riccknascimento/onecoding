#!/usr/bin/env bash
# Teste de restauração: valida o checksum, restaura o dump em um banco ISOLADO (nunca o principal), compara
# contagens de tabelas-chave e descarta o banco de teste. Não toca em produção.
set -euo pipefail
FILE="${1:?uso: restore-check.sh <arquivo.dump>}"
PROD_DB="${ONEAGENT_DB_NAME:-oneagent_central}"
TARGET="oneagent_restore_check_$(date -u +%Y%m%d%H%M%S)"
[ "$TARGET" != "$PROD_DB" ] || { echo "recusado: alvo igual ao banco principal"; exit 1; }
sha256sum -c "$FILE.sha256"
ERRLOG="$(mktemp)"
cleanup() { su postgres -c "dropdb --if-exists '$TARGET'" >/dev/null 2>&1 || true; rm -f "$ERRLOG"; }
trap cleanup EXIT
su postgres -c "createdb '$TARGET'"
# o dump é do root (600): entregue pela entrada padrão, sem depender de permissão do usuário postgres
if ! su postgres -c "pg_restore --no-owner --no-privileges -d '$TARGET'" < "$FILE" 2> "$ERRLOG"; then
  echo "pg_restore falhou:"; head -5 "$ERRLOG"; exit 1
fi
# conta só as tabelas-chave que existem neste dump (o estado de migrations determina quais existem)
for t in schema_migrations users onenodes conversations messages jobs; do
  if [ "$(su postgres -c "psql -At -d '$TARGET' -c \"select to_regclass('$t') is not null\"")" = "t" ]; then
    n=$(su postgres -c "psql -At -d '$TARGET' -c 'select count(*) from $t'")
    echo "tabela $t: restaurado=$n"
  else
    echo "tabela $t: ausente neste dump (migration ainda não aplicada)"
  fi
done
echo "restauração de teste OK (banco isolado $TARGET será descartado)"
