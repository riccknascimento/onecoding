# OneCoding — implantação (OneHub Central + Caddy)

Identidade visual: OneCoding. Nomes técnicos mantidos: OneHub Central, OneOrchestrator, OneExecutor, OneNode.
Estado: **preparado e validado em teste; NÃO publicado**. Publicação exige autorização explícita (ver “Autorizações”).

## Arquitetura
- `central` (OneHub, imagem `oneagent/central:current`) e `caddy` rodam com `network_mode: host`, sob **um único
  supervisor**: o próprio Docker Compose, com `restart: unless-stopped`. Nada de systemd junto.
- O Central escuta só em `127.0.0.1:8787`. O PostgreSQL continua em `127.0.0.1:5432` (sem exposição).
- Caddy: HTTPS automático (ACME) para `app.oneagente.tech` (frontend estático em `./web`) e `hub.oneagente.tech`
  (proxy para o Central, com `flush_interval -1` para o SSE). Portas 80/443 **só** quando autorizado.
- Healthcheck do Central: `GET /health` a cada 30 s. O Caddy só sobe depois do Central saudável.
- Logs: Docker (json-file, 10 MB × 5 por serviço) e Caddy (arquivos com rolagem de 10 MB × 5 em `./logs`).

## Arquivos
- `docker-compose.yml`, `central/Dockerfile` (build multi-estágio reproduzível, `--locked`).
- `caddy/Caddyfile` (produção), `caddy/test.Caddyfile` (teste local), `caddy/sites.caddy` (sites).
- `central.env.example` — modelo de `/etc/oneagent-central/central.env` (600). **Não versionar o real.**
- `scripts/backup-central.sh` (somente leitura no banco; dump custom + SHA-256 + `pg_restore --list`; retenção).
- `scripts/restore-check.sh` (valida checksum, restaura em banco isolado, compara contagens, descarta o banco).
- `scripts/deploy.sh` (guarda configuração e banco ANTES; imagem anterior vira `previous`; migrations só com
  `ONEAGENT_APPLY_MIGRATIONS=1`; falha de saúde dispara rollback).
- `scripts/rollback.sh` (reativa a imagem anterior; não reverte migrations).
- `scripts/migrate.sh` (exige `ONEAGENT_CONFIRM_PRINCIPAL_MIGRATE=sim`; faz backup antes).
- `scripts/bootstrap-admin.sh` (exige `ONEAGENT_CONFIRM_BOOTSTRAP=sim`; senha lida do terminal, sem eco e sem arquivo).

## Ordem de implantação (quando autorizada)
1. Criar `/etc/oneagent-central/central.env` (600) a partir do modelo, com a URL do banco principal.
2. `scripts/backup-central.sh` e `scripts/restore-check.sh <dump>` (já feitos em teste).
3. `ONEAGENT_CONFIRM_PRINCIPAL_MIGRATE=sim scripts/migrate.sh` (aplica 0002–0006 no principal, com backup antes).
4. `ONEAGENT_CONFIRM_BOOTSTRAP=sim ONEAGENT_BOOTSTRAP_EMAIL=<email> scripts/bootstrap-admin.sh`.
5. `scripts/deploy.sh` (publica o Central e o Caddy; `ACME_EMAIL` definido).
6. Verificar `https://hub.oneagente.tech/health` e o login em `https://app.oneagente.tech`.

## Rollback
- Serviço: `scripts/rollback.sh` (imagem `previous`).
- Configuração: cópias em `/var/backups/oneagent/config/<data>/` (restaurar manualmente).
- Banco: não há reversão automática de migrations. Restaurar só com aprovação, a partir de um dump validado.

## Autorizações necessárias para publicar
1. Abrir 80/443 e apontar os DNS `app.` e `hub.oneagente.tech` para `187.77.52.164`.
2. Criar `/etc/oneagent-central/central.env` com o banco principal.
3. Aplicar migrations 0002–0006 no banco principal (`migrate.sh`).
4. Criar o administrador de produção (`bootstrap-admin.sh`).
5. Iniciar o Central e o Caddy públicos (`deploy.sh`).
