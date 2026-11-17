# OneAgent — Registro de decisões

> Atualizado na Fase 1 · 2026-11-17. Status: **Aceita** (decidida pelo usuário) · **Implementada** · **Pendente** (exige decisão ou investigação) · **Proposta** (recomendação aguardando OK).
> Fatos de fornecedor marcados **[D]** vêm de documentação oficial lida nesta data; **[V]** = verificado nesta máquina.

## Decisões aceitas

| ID | Decisão | Fonte |
|---|---|---|
| A-01 | Pasta do projeto `~/Documents/OneAgent`; trabalho por fases, cada uma só avança com autorização | Pedido |
| A-02 | Componentes: OneAgent.app, OneHub, Agent Gateway (OneNode), OneContext, OneOrchestrator, OneSkills Registry, OneAgent Factory | Pedido |
| A-03 | Validação local no Mac primeiro; VPS Apliflow só após o MVP local | Pedido |
| A-04 | Motores: Codex CLI, Claude Code, Cursor CLI, atrás de adaptadores | Pedido |
| A-05 | Orquestrador decide e delega; CLIs executam; Gateway controla; OneHub aplica política | Pedido |
| A-06 | **Stack:** Tauri 2 + React + TypeScript no app; Rust no OneHub e no Gateway; SQLite com FTS5 | Fase 1 |
| A-07 | Git local, branch `main`, **sem remoto** nesta fase; nenhum commit sem autorização | Fase 1 |
| A-08 | pnpm com versão fixada (`packageManager`); Rust pelo rustup oficial, sem privilégio administrativo | Fase 1 |
| A-09 | Sem Docker; sem Xcode completo (só CLT, suficiente para o build atual [V]) | Fase 1 |
| A-10 | Sem chamadas pagas a LLM sem autorização; priorizar plan usage oficial se disponível e elegível | Pedido atual |
| A-11 | Acesso remoto e instalação em VPS exigem aprovação explícita; nenhuma autorização atual para a VPS | Pedido atual |

## Decisões de arquitetura

### D-001 — Provedor do orquestrador · **Histórica; substituída por D-026**
Registro da investigação inicial [D], não é mais a decisão vigente. A conclusão e prioridade atuais estão em D-026; reconfirmar disponibilidade e elegibilidade antes da 3B.

| Mecanismo | O que a documentação diz | Veredito |
|---|---|---|
| **Sign in with ChatGPT / Plan Usage** | Na investigação inicial, fluxo descrito como preview, OAuth authorization code + PKCE, callback loopback e uso da cota em `/v1/responses`. Requisitos e elegibilidade podem ter mudado; verificar D-026 antes de implementar. Não usar `backend-api` nem tokens do Codex. | **Prioridade condicional na 3B**, se oficialmente disponível e elegível; bloqueio deve ser documentado |
| **Responses API com function calling** (chave de API) | Function tools são o caminho oficial para ferramentas próprias; `strict` recomendado; streaming de argumentos. Chave só no servidor (não embutir no app). Limites por organização/projeto. | **Candidata para o MVP**, pelo caminho desacoplado. Exige orçamento aprovado |
| **ChatKit** | Exige servidor próprio (`ChatKitServer`, pacote `openai-chatkit`) ou fluxo hospedado (`/v1/chatkit/sessions`). Sem menção a Tauri ou webview. Taxa própria só para armazenamento de upload; o restante é token. | **Não recomendado**: UI própria é requisito e o ganho é pequeno |
| **Agent Builder** | Descontinuado; **encerramento em 30/11/2026** [D]. ChatKit e Agents SDK continuam. | Não usar. Não confundir com ChatKit |
| **MCP** | Responses API aceita MCP remoto por HTTPS (Streamable HTTP ou SSE); MCP local/privado só via Secure MCP Tunnel. ChatGPT aceita custom MCP por endpoint HTTPS público ou Tunnel, em planos ainda não confirmados. | Serve para expor ferramentas no futuro. **Não substitui** o orquestrador |
| **Token OAuth do Codex em proxy** | Documentação do Codex: uso do app-server com autenticação ChatGPT "has never been permitted for commercial or hosted services". Terceiros fazem isso sem contrato. | **Rejeitada** |

Preços de referência da API [D, tabela convertida automaticamente; conferir antes de decidir]: gpt-6.1-sol US$ 2,00 / 10,00 por 1M tokens (entrada/saída); gpt-6.1-terra US$ 1,00 / 5,00; gpt-6-luna US$ 0,10 / 0,50; gpt-6-astra US$ 10,00 / 50,00. Modelo recomendado pela própria OpenAI para trabalho complexo: Sol 6.1.

**Registro histórico:** a investigação e a decisão originais foram substituídas por D-026. A implementação local não commitada da base 3A agora contém `OrchestratorProvider`, `DisconnectedProvider` e `ScriptedProvider`; isso não habilita chamadas reais. O estado detalhado está em `ORCHESTRATION-PLAN.md`.

Não há autorização para chamadas pagas de API. Qualquer alternativa paga depende de aprovação e orçamento separados.

### D-002 — Shell do app: Tauri 2 · **Aceita e implementada**
Versões fixadas: `tauri` 2.12.x, `@tauri-apps/api` 2.12.5, `@tauri-apps/cli` 2.12.5 [V; crates.io e npm]. Tauri 3 aparece no crates.io só como alpha; **não usar**.

### D-003 — Linguagem do OneHub · **Aceita: Rust** · **Implementada** (`src-tauri` + `one-store`)
### D-004 — Linguagem do Gateway · **Aceita: Rust** · **Implementada parcialmente** (`one-adapters`; o binário do Gateway ainda não existe)

### D-005 — Sandbox no macOS · **Parcialmente resolvida (Fase 2A; ver `ISOLATION.md`)**
Leitura confinada pelo perfil de permissão do Codex (D-024). Seatbelt externo próprio foi **rejeitado**: o Codex deixa de executar comandos dentro dele. Pendentes: modo restrito para terceiros com usuário dedicado/VM e o modo de VPS.

(texto anterior)
Proposta: MVP confia nas sandboxes das CLIs + verificação pós-execução + worktree isolada. Reavaliar na fase de endurecimento. Fatos [D]: Cursor headless exige `--sandbox enabled` ou `--force` para escrita; a documentação do sandbox não foi lida.

### D-006 — Aprovação no meio da execução · **Proposta**
Antes da execução: modo, escopo e worktree. Durante a execução: só onde o motor suporta (Claude via `--permission-prompts host`; Codex app-server, experimental). Cursor: escrita só em worktree isolada, com revisão de diff.

### D-007 — Transporte Hub ↔ Gateway remoto · **Proposta** (ver `GATEWAY.md` §5–6)

### D-008 — Termos de uso das CLIs em automação · **Parcialmente investigada; pendente**
Achados [D], das páginas oficiais lidas nesta data:
- **Anthropic:** a página legal diz que não impede o usuário final de entrar no binário Claude Code não modificado com a própria assinatura. Proibido a terceiros oferecer login Claude.ai no próprio app, rotear requisições por credenciais de plano em nome de usuários ou armazenar/intermediar tokens de sessão. **Não consegui confirmar** se um app que executa o `claude -p` local do próprio usuário é permitido; a leitura favorável é inferência minha. Créditos: em 7/out/2026 Max, Ultra e Team ganharam créditos mensais de API para Agent SDK e `claude -p` **com chave de API**; com login de plano, o consumo continua na cota da assinatura.
- **OpenAI/Codex:** o login da assinatura para scripts e CI não é recomendado (CI deve usar chave de API). Não achei regra explícita sobre copiar `auth.json`.
- **Cursor:** os Termos (atualizados em 03/09/2026) não proíbem nem autorizam expressamente apps de terceiros que chamam o CLI. Admin de time pode desativar o headless.

**Conduta adotada:** o Gateway só executa as CLIs oficiais já logadas pelo próprio usuário; nunca lê, copia ou guarda tokens delas; detecção apenas lê status. Antes de usar execução headless em produto de terceiros, **consultar as áreas jurídica/contratual** (Anthropic: contato comercial, conforme a própria página).

### D-009 — Toolchain · **Resolvida**
Rust **1.100.0** estável instalado com rustup 1.29.3 (`~/.cargo`, `~/.rustup`), perfil minimal + clippy + rustfmt, fixado em `rust-toolchain.toml`. Procedência: `rustup-init` baixado de `static.rust-lang.org` com SHA-256 conferido contra o `.sha256` publicado (`63752a19…20f9f`). Nada em `.zshrc` (instalação com `--no-modify-path`). Xcode completo **não** instalado: não foi necessário.

### D-010 — Modelo e custo do orquestrador · **Pendente** (depende de D-001)

### D-011 — Idioma do System Prompt · **Proposta: pt-BR** (avaliar nos evals)

### D-012 — Repositório e distribuição · **Implementada localmente; remoto pendente**
Repositório Git local em `main`, sem remoto. Pendente: criar remoto GitHub (público/privado, nome), política de releases do Gateway e conta Apple Developer para assinatura/notarização (dispensável no uso pessoal).

### D-013 — Modelos disponíveis · **Implementada**
- **Codex:** `codex debug models` (JSON, `visibility == "list"`) [V]. Nesta máquina: gpt-6.1-sol, gpt-6-astra, gpt-6.1-terra, gpt-6-sol, gpt-6-luna, gpt-5.6-*.
- **Cursor:** `cursor-agent --list-models` [V]; logado, com dezenas de modelos.
- **Claude Code:** sem listagem [V]. Apelidos `fable`, `opus`, `sonnet` marcados **não confirmados**. Confirmar o modelo real no evento de `system/init` da execução (Fase 2).

### D-014 — OneContext no MVP · **Proposta** (sem mudança: FTS5 primeiro; embeddings só se AC-08 falhar)

### D-015 — Estratégia de execução por motor · **Proposta (nova)**
Codex: `codex exec --json` com o login da CLI do próprio usuário, **não** o app-server com tokens. Claude: `claude -p --output-format stream-json`. Cursor: `cursor-agent -p --output-format stream-json`, sempre com `--output-format` explícito (o `--help` e a documentação divergem sobre o padrão). Motivo: a documentação do Codex restringe o app-server, e os formatos de saída do Cursor e do Claude são documentados.

### D-016 — Multi-cliente: um Gateway por ambiente · **Aceita (pedido da Fase 1)**
Ver `GATEWAY.md`. Credenciais e clones ficam no Gateway; o Mac guarda metadados e referências.

### D-017 — Confiança de executáveis · **Implementada (Fase 1, correção da auditoria)**
Detectar ≠ executar. Aprovação humana por diálogo nativo; identidade persistida e revalidada. Detalhes e riscos residuais em `SECURITY.md` §12. Regra de permissões: dono = usuário atual ou root; nada gravável por grupo ou outros; sem ACL com escrita; interpretadores resolvidos com diretórios do sistema primeiro (este Mac tem um segundo administrador no grupo `admin`, então `/opt/homebrew` é gravável por outra pessoa e é reprovado de fato) (decisão endurecida após a revisão adversarial; verificado que os caminhos reais de Codex, Claude e Cursor neste Mac passam).

### D-018 — Dependências adicionadas na correção · **Implementada**
`sha2` 0.11 (SHA-256 da identidade; RustCrypto), `libc` 0.2 (grupo de processos, `O_NONBLOCK`, `geteuid`; só Unix), `tauri-plugin-dialog` 2.8 (diálogo nativo, usado só pelo Rust). Versões conferidas no crates.io em 17/11/2026; todas mantidas por projetos amplamente usados. Nenhuma instalação global.

### D-019 — Migração V2 · **Implementada**
Aplicada no banco real do usuário na primeira abertura (V1 vazio → V2), com cópia `oneagent.db.pre-v2.bak` ao lado. Mudanças futuras: nunca editar uma migração publicada.

### D-020 — Execução do Codex (Fase 2A) · **Implementada**
`codex exec --json --ignore-user-config --ephemeral --skip-git-repo-check --color never -s read-only -C <workspace> [-m <modelo>] -` com o prompt por stdin (Codex 0.169.0, conferido no `--help` e em execuções reais). `--ephemeral` evita gravar rollout em `~/.codex`; `--ignore-user-config` evita carregar MCPs/plugins do usuário. Sem `app-server` e sem tokens (D-015). Formato de eventos observado: `thread.started`, `turn.started`, `item.started/completed` (`agent_message`, `command_execution`, `error`), `turn.completed{usage}`, `error`, `turn.failed`; o modelo **não** aparece.

### D-021 — UI acompanha a execução por consulta (polling, 400 ms) · **Implementada**
Sem permissão de eventos na webview e sem canal extra: os eventos já estão no SQLite e a UI lê `get_run(run_id, after_seq)`. Troca por push é possível depois sem mudar o contrato.

### D-022 — Contrato independente de motor · **Implementada**
`EngineAdapter` (argumentos estruturados + `EngineParser` incremental) normaliza para `EventPayload`/`RunView`. Claude e Cursor entram implementando o mesmo trait (Fase 2B); o serviço (`one-exec`) não conhece formato de motor.

### D-023 — Sessão do motor ≠ contexto da tarefa · **Implementada**
`runs.engine_session_id` guarda a sessão interna (thread do Codex) separada de `tasks`/`conversations`; o contexto da tarefa (prompt, workspace, resumo) é do OneAgent. É a base do OneContext Handoff (não implementado).

### D-024 — Confinamento de leitura do Codex · **Implementada (padrão), decisão do usuário pendente sobre o modo amplo**
`default_permissions="oa_workspace_ro"` + perfil `:root=deny, :minimal=read, :tmpdir=deny, :slash_tmp=deny, :workspace_roots={"."=read}`, sem `-s` (o sandbox legado anularia o perfil). Evidência em `ISOLATION.md` e nos testes `os_level_profile_denies_reads_outside_the_workspace` (SO) e `real_codex_*` (execução real). Custo medido: `git` (Xcode CLT) e `node` falham dentro do agente. `Confinement::TrustedLocalRead` não é exposto. Recurso Beta: reexecutar o teste de SO a cada versão do Codex.

### D-025 — Decisões de encerramento da Fase 2A · **Aceita**
(1) Perfil restrito de leitura como padrão. (2) `TrustedLocalRead` não exposto na interface. (3) Nenhuma permissão ampliada para consertar Git/Node. (4) Limitação de Git/Node registrada. (5) Três modos futuros: Investigação Restrita (atual), Desenvolvimento Isolado e Administração Autorizada (`ISOLATION.md` §6). (6) O perfil Beta não é garantia absoluta de isolamento do processo inteiro. (7) Sem sandbox de Linux ou Windows nesta fase.

### D-026 — Provedor do orquestrador · **Prioridade: ChatGPT Plan Usage, se disponível e elegível**
Na 3B, priorizar a integração oficial Sign in with ChatGPT/Plan Usage. **Não usar API paga sem autorização explícita; não assumir elegibilidade.** Se o fluxo oficial bloquear o usuário ou o projeto, registrar a evidência e apresentar alternativas antes de decidir. Não alterar a visibilidade do repositório para contornar elegibilidade. A investigação documental foi feita em 2026-11-17; reconfirmar disponibilidade, termos e elegibilidade antes da implementação, pois o recurso é preview:
- **Autorização:** `GET https://auth.openai.com/api/accounts/authorize` no navegador do sistema, com `client_id=dynamic_agent_client` (1º registro) e `agent_name_hint`, `ext_agent_host_id`, `response_type=code`, `redirect_uri=http://127.0.0.1:<porta>/auth/callback` (não `localhost`; listener antes do navegador), `scope="openid profile email offline_access resource.invoke chatgpt.tokens.use.direct"`, `resource=https://api.openai.com/v1`, `state`, `nonce`, PKCE `S256`. O callback devolve o `client_id` emitido (`oaiapp_…`), que deve ser **salvo antes** da troca e reutilizado.
- **Troca e refresh:** `POST https://auth.openai.com/api/accounts/oauth/token` (form, sem secret). Access ~1 h, refresh ~30 dias com rotação (guardar o novo; serializar refreshes). **Revogação:** `revocation_endpoint` do discovery (`/.well-known/openid-configuration`; URL literal não documentada), 200 vazio = sucesso.
- **Validação do ID token:** assinatura via JWKS (`/.well-known/jwks.json`), `iss=https://auth.openai.com`, `aud` = client_id emitido, `exp`, `nonce`.
- **Login × plan usage:** "A valid ID token alone does not authorize ChatGPT plan usage": exigir `chatgpt.tokens.use.direct` nos escopos retornados. Sem ele, manter o login e deixar o plan usage desligado, **sem trocar de cobrança em silêncio**.
- **Modelos:** `GET https://api.openai.com/v1/models`, filtrar `visibility == "list"`, usar `slug`.
- **Chamada:** `POST https://api.openai.com/v1/responses` com `store:false` e `stream:true`, histórico completo em `input`, `instructions` (sem item `system`), **sem** `previous_response_id`, `max_output_tokens`, `temperature` e demais campos listados como não suportados; function tools suportadas; MCP hospedado, file search e code interpreter não. Sucesso só após `response.completed`; `response.failed` pode chegar com o stream aberto.
- **Erros:** `subscription_sharing_user_not_eligible` (403, não repetir), `…usage_limit_exceeded` (429, pausar), `…usage_unavailable`/`…user_unavailable` (503, backoff), `…unsupported_capability` (400, remover o recurso), `…route_not_supported` (403), `…invalid_user` (401), `chatpass_v2_*` (403).
- **Credenciais:** a documentação só exige arquivo `0600`; o OneAgent usará o **Keychain** (mais forte) e nunca SQLite, log ou webview. Guardar `id_token` para `id_token_hint` na reautorização. Um perfil por conta/workspace.
- **Não documentado** (confirmar na 3B): formato do `function_call` no stream (guia de function calling), URL literal de revogação, status HTTP de access token expirado, schema completo de `/v1/models`. **Não lidos:** políticas gerais da OpenAI e o DevKit.
**Chamada real somente com autorização expressa do usuário.** A implementação de OAuth também exige autorização para iniciar a etapa 3B.

### D-027 — Arquitetura e dois modos · **Aceita**
Tauri 2 + React + TS, OneHub em Rust, Gateways independentes, SQLite, OneContext persistente, `OrchestratorProvider` desacoplado, ferramentas tipadas autorizadas pelo Hub. **Orquestrador:** ChatGPT → OneHub → Gateway → CLI. **Direto:** usuário → OneHub → Gateway → CLI (a Fase 2A). O Codex nunca é apresentado como ChatGPT Orquestrador.

### D-028 — Segurança do orquestrador · **Aceita**
Sem shell irrestrito, caminhos arbitrários nem credenciais SSH para o orquestrador. O Hub valida todas as ações; aprovações sensíveis são do usuário e verificadas pelo backend.

### D-029 — Git · **Aceita**
Preservar a baseline `fase-1-baseline` e o commit da 2A (`41c0ab3`); sem merge automático, sem remoto, sem publicar código.

### D-030 — SSH · **Aceita (planejamento; sem autorização de acesso)**
Provisionamento remoto é prioridade, via o alias `flowsharp` já configurado no Mac e pelo serviço controlado do Hub (`ORCHESTRATION-PLAN.md` §7). A autorização atual **não** permite acessar a VPS, instalar nem modificar serviços.

### D-031 — Ordem de implementação · **Aceita**
3A (OneOrchestrator) → 3B (ChatGPT Plus) → 2B (provisionamento SSH, antes de Claude/Cursor) → Claude e Cursor → OneContext Handoff → OneProject Discovery → OneSkills Registry → OneUsage → OneAgent Factory (os últimos só planejados). "2B" passa a significar provisionamento remoto.

## Riscos e incompatibilidades (atualizados)

1. Plan Usage/Sign in with ChatGPT está sujeito a mudanças, elegibilidade e termos que precisam ser confirmados antes da 3B (D-026).
2. Termos das assinaturas em automação não confirmados para produto de terceiros (D-008).
3. Aprovação mid-run desigual entre motores (D-006).
4. Cursor `-f`/`--yolo` libera tudo; isolamento por worktree obrigatório.
5. `codex app-server` marcado experimental; não usar com tokens (D-015).
6. Claude sem listagem de modelos (D-013).
7. Build Rust em 16 GB: o primeiro build release levou ~1 min após cache; builds frios são mais lentos.
8. Sem Docker local: testes de integração com CLIs falsas e diretórios temporários.
9. Webview com conteúdo não confiável: CSP estrita aplicada (`SECURITY.md` §11).
10. Agent Builder encerra em 30/11/2026; não depender dele.
11. Fontes de terceiros continuam sendo confirmadas nas páginas oficiais antes de virarem requisito.
12. Páginas oficiais de Termos e Central de Ajuda da OpenAI retornaram 403 na pesquisa de 2026-11-17; confirmar diretamente na 3B.

## Fontes consultadas (2026-11-17)

OpenAI: https://developers.openai.com/siwc/quickstart · https://developers.openai.com/siwc/token-sharing-open-source · .../sign-in · .../models-and-inference · .../preview-limitations · https://developers.openai.com/siwc/request-client-id · https://developers.openai.com/api/docs/guides/function-calling · https://developers.openai.com/api/docs/guides/chatkit · https://developers.openai.com/api/docs/guides/agent-builder · https://developers.openai.com/api/docs/deprecations · https://developers.openai.com/api/docs/pricing · https://developers.openai.com/api/docs/guides/tools-remote-mcp · https://developers.openai.com/api/docs/guides/developer-mode · https://learn.chatgpt.com/docs/auth · https://learn.chatgpt.com/docs/app-server
Anthropic: https://code.claude.com/docs/en/legal-and-compliance · https://support.claude.com/en/articles/15036540-use-the-agent-sdk-with-your-claude-plan
Cursor: https://cursor.com/docs/cli/headless · https://cursor.com/docs/cli/reference/output-format · https://cursor.com/docs/cli/reference/permissions · https://cursor.com/docs/cli/reference/authentication · https://cursor.com/terms-of-service
Tauri: https://v2.tauri.app/develop/sidecar · https://v2.tauri.app/plugin/shell/

Limites: páginas de termos e de ajuda da OpenAI retornaram 403; parte das respostas foi lida por resumo automático (marcado nos relatórios de origem). Antes de qualquer decisão contratual, conferir essas páginas diretamente.
