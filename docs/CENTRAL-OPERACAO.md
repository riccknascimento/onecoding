# OneHub Central — operação, atualização e backup

Estado: procedimento preparado para a futura implantação. Nada disto foi executado no banco principal.

## Configuração (variáveis de ambiente)
- `ONEAGENT_CENTRAL_DATABASE_URL`: URL do PostgreSQL. Nunca registrar em log nem em Git.
- `ONEAGENT_CENTRAL_BIND`: padrão `127.0.0.1:8787`. Endereço não local exige `ONEAGENT_CENTRAL_ALLOW_PUBLIC_BIND=1`, e isso só vale atrás de HTTPS com autorização explícita.
- `ONEAGENT_CENTRAL_MIGRATIONS` (opcional): pasta das migrations. Padrão: `migrations/` do crate.
- `ONEAGENT_CENTRAL_COOKIE_SECURE` (padrão `1`): cookie de sessão com `Secure`. Só `0` em desenvolvimento em `http://localhost`.
- `ONEAGENT_CENTRAL_ALLOWED_ORIGINS` (vazio por padrão): origens cross-site autorizadas, separadas por vírgula (ex.: `https://app.oneagente.tech`). Vale para CORS (lista exata, sem curinga, com credenciais) e para o anti-CSRF das requisições de navegador.

## Sessão e origem (interface Web)
- O navegador recebe a sessão em cookie `oh_session`: `HttpOnly`, `SameSite=Strict`, `Path=/`, validade de 12 h. A interface não guarda token em `localStorage` nem em memória persistente.
- Requisição que altera estado com cookie precisa de `Origin` igual ao `Host` ou listada em `ONEAGENT_CENTRAL_ALLOWED_ORIGINS`; sem isso, `403 origin_not_allowed`. Clientes sem navegador (bearer) não dependem disso.
- Publicação recomendada: Caddy servindo a interface e a API na mesma origem (ex.: `https://app.oneagente.tech` com `/central-api` roteado ao Central). Assim o CORS nem é necessário. Usar `hub.oneagente.tech` como origem separada só com a lista explícita acima.

## Tempo real (SSE)
- `GET /v1/conversations/{id}/state`: snapshot (mensagens, janela de eventos e estado atual dos jobs e tarefas) com `last_seq`.
- `GET /v1/conversations/{id}/events`: stream SSE. Cada evento tem `id` (sequência da conversa) e a conexão retoma com `Last-Event-ID`. Ping a cada 10 s.
- A sequência é alocada sob o lock da conversa, na mesma transação da mudança. Não há lacuna visível nem evento sem estado.
- O stream revalida a sessão e a concessão OneNode × projeto a cada ciclo (1 s). Revogação ou logout encerra a conexão com `closed` e o motivo.
- Cancelamento: `POST /v1/jobs/{id}/cancel`. Job na fila é cancelado na hora. Job em execução só vira `cancelled` quando o OneExecutor confirma pelo `finish` (o executor vê o pedido no heartbeat). Se o executor desaparecer, o lease vencido também encerra como `cancelled`.

## Executor: nova tentativa
- O resultado do job é registrado com nova tentativa para falha transitória (Central reiniciando): esperas de 2, 4, 8 e 16 s. Recusas definitivas (lease vencido) não são repetidas.

## Administrador inicial (uso único)
    printf '%s\n' "$SENHA" | ONEAGENT_BOOTSTRAP_EMAIL=admin@... ONEAGENT_CENTRAL_DATABASE_URL=... one-central bootstrap-admin
A senha vem da entrada padrão. Depois do primeiro administrador, o comando recusa qualquer nova execução.

## Migrations
- Aplicadas em ordem (`NNNN_nome.sql`), cada uma em transação, com checksum registrado em `schema_migrations`.
- Migration já aplicada e alterada no arquivo = o serviço recusa iniciar. Nunca editar migration publicada: criar uma nova.
- Um advisory lock impede duas instâncias migrando ao mesmo tempo.

## Backup (antes de qualquer atualização)
    pg_dump --format=custom --file=oneagent_central-AAAAMMDD-HHMM.dump oneagent_central
- Guardar o dump fora do servidor, com permissão 600, e registrar o hash SHA-256.
- Testar a restauração em um banco `*_test` antes de confiar no backup: `pg_restore --dbname=oneagent_central_test ...`.
- Frequência sugerida: diária, mais um dump antes de cada migration.

## Atualização
1. Parar o serviço do Central.
2. Gerar o backup (acima) e conferir o hash.
3. Copiar o novo binário e a pasta `migrations/` (somente adição de arquivos novos).
4. Iniciar o serviço: as migrations pendentes são aplicadas sozinhas.
5. Testar `/health` e um login.

## Reversão
- Binário: voltar para o binário anterior, mantido ao lado do novo.
- Banco: migrations não são revertidas automaticamente. Se a nova migration causou dano, restaurar o dump anterior em uma janela de manutenção, com aprovação.

## Proteção contra força bruta
- 5 falhas de login para o mesmo e-mail em 15 minutos bloqueiam novas tentativas desse e-mail (HTTP 429), inclusive com a senha correta.
- O bloqueio vale também para e-mails inexistentes, para não revelar quais contas existem.
- Limitação: o bloqueio é por e-mail, não por origem. Uma origem que varra muitos e-mails não é bloqueada por isso. Antes da publicação, somar limite por origem na borda HTTPS (Caddy).
