# OneAgent — Arquitetura técnica (proposta, Fase 0)

> Status: arquitetura aprovada; Fases 1, 2A e base 3A implementadas neste working tree, ainda sem commit (ver §15) · 2026-11-17 · Legenda: **[V]** verificado nesta máquina · **[D]** documentação/pesquisa · **[?]** não verificado.
> Decisões e pendências: `DECISIONS.md`. Gateway e multi-cliente: `GATEWAY.md`. Catálogo de capacidades: `CAPABILITIES.md`.

## 1. Ambiente de desenvolvimento verificado

| Item | Estado [V] | Impacto |
|---|---|---|
| macOS 26.5.2, arm64 (M4), 16 GB RAM, 69 GB livres | ok | Suficiente; builds Rust pesam em 16 GB, evitar muitos jobs paralelos |
| Node 24.15 / npm 11.12 / pnpm via Corepack | ok; `pnpm --version` tenta **baixar** o pnpm (Corepack) | Fixar `packageManager` no repositório; aprovar download na Fase 1 |
| Rust / cargo / rustup | **ausente** | Necessário para Tauri e para Hub/Gateway em Rust (D-003/D-004) |
| Xcode completo | **ausente** (só Command Line Tools) | Para app desktop macOS, CLT tende a bastar [?]; assinatura/notarização exige conta Apple Developer [D] |
| Swift 6.3.2 | ok | Alternativa SwiftUI possível (D-002) |
| Python 3.9.6 (sistema) | antigo | Não usar para o projeto |
| git 2.50, gh 2.103, sqlite3 3.51 | ok | SQLite do sistema suporta FTS5 [?] (confirmar na Fase 1) |
| Docker | ausente — **regra permanente: não instalar** | Testes e CI não podem depender de Docker local |
| Codex CLI 0.169.0 | logado com ChatGPT; tem `exec --json`, `app-server`, `debug models`, `mcp`, `sandbox` | Adaptador viável |
| Claude Code 2.1.326 | logado (claude.ai); tem `-p`, `--output-format stream-json`, `--input-format stream-json`, `--permission-mode`, `--permission-prompts host`, `--mcp-config`, `--agents`, `--session-id/--resume`, `--bare` | Adaptador viável |
| Cursor CLI `cursor-agent` 2026.11.13 | logado; tem `-p`, `--output-format stream-json`, `--mode plan/ask`, `--sandbox`, `--list-models`, `--workspace` | Adaptador viável, com limites de aprovação |

## 2. Visão geral

```
┌───────────────────────── Mac ─────────────────────────┐
│ OneAgent.app (Tauri 2)                                │
│  ┌────────────┐  IPC tipado  ┌─────────────────────┐  │
│  │ UI React   │◄────────────►│ OneHub (núcleo Rust)│  │
│  │ (webview)  │              │  • policy engine    │  │
│  └────────────┘              │  • aprovações       │  │
│                              │  • SQLite (+FTS5)   │  │
│                              │  • OneContext       │  │
│                              │  • Keychain         │  │
│                              │  • cliente OpenAI   │──┼──► Provedor do orquestrador (HTTPS)
│                              └─────────┬───────────┘  │
└────────────────────────────────────────┼──────────────┘
                                         │ protocolo OneNode (JSON, versionado)
                    local: socket Unix   │   remoto (futuro): WSS/mTLS sobre túnel SSH
                                         ▼
                        ┌────────────────────────────┐
                        │ Agent Gateway (OneNode)    │  usuário comum, sem root
                        │  • descoberta de CLIs      │
                        │  • workspaces autorizados  │
                        │  • adaptadores de motores  │
                        │  • sandbox/worktree        │
                        │  • coleta de evidências    │
                        └───┬──────────┬──────────┬──┘
                            ▼          ▼          ▼
                       Codex CLI  Claude Code  Cursor CLI   (executores)
```

Regras estruturais:
- A **UI nunca fala com Gateway nem provedor**; só com o Hub via comandos tipados.
- O **orquestrador nunca executa nada**: só chama *ferramentas do OneHub*; o Hub valida com o policy engine e encaminha ao Gateway.
- O **policy engine vive fora da webview** (Rust). O Gateway reaplica as regras de diretório/modo (defesa em profundidade).
- As CLIs são **processos filhos** do Gateway; o Gateway nunca lê nem copia credenciais delas.

## 3. Stack proposta

| Camada | Escolha recomendada | Alternativa | Motivo |
|---|---|---|---|
| App desktop | **Tauri 2** + React + TypeScript + Vite | SwiftUI nativo | UI de chat rica reaproveita ecossistema web; núcleo Rust hospeda Hub com segredos/política fora da webview. SwiftUI evita toolchain Rust no app, mas não ajuda o Gateway multiplataforma |
| Hub e Gateway | **Rust** (workspace Cargo; crate `one-protocol` compartilhada) | TypeScript (Node SEA) | Binário único por SO para releases do GitHub, sem runtime, bom controle de processos/sandbox; contratos compartilhados com o app |
| Persistência | **SQLite** (WAL, FTS5) via `rusqlite`/`sqlx` | — | Local, simples, transacional; FTS5 cobre recuperação inicial do OneContext |
| Contratos | JSON Schema como fonte; tipos TS e Rust gerados | — | Evita divergência entre camadas |
| Estado da UI | TanStack Query + Zustand [?] | — | Decidir na Fase 1 |
| Logs | `tracing` (Rust), JSON estruturado, rotação | — | Correlação por `task_id`/`run_id` |
| Segredos | **Keychain do macOS** (Credential Manager/Secret Service depois) | — | Nunca em SQLite, arquivos ou logs |
| Testes | `cargo test`, Vitest, Playwright/WebDriver p/ UI [?], suíte de comportamento do orquestrador | — | Sem Docker |

Pontos a validar antes de fixar (Fase 1, spikes): Tauri 2 compila neste Mac com só CLT [?]; WKWebView renderiza streaming longo sem engasgo [?]; tamanho/tempo de build em 16 GB [?].

## 4. Orquestrador (ChatGPT) — o que é possível

Fato central: **a assinatura ChatGPT (Plus/Pro) não permite embutir a interface do ChatGPT nem reutilizar a sessão de login** em UI própria. Opções oficiais/documentadas [D]:

| Opção | Como funciona | Custo / limite | Encaixe |
|---|---|---|---|
| **A. OpenAI API (Responses API) com function tools** | Hub chama a API; ferramentas = operações do OneHub; aprovação via política local | Cobrança por token e chave de API, independente da assinatura | Alternativa futura somente com autorização explícita e orçamento |
| **B. Sign in with ChatGPT / Plan Usage** | OAuth PKCE; disponibilidade, elegibilidade e limites dependem do fluxo oficial vigente | Preview; revisar documentação e validar elegibilidade antes de implementar. Sem reutilizar tokens internos do Codex | Prioridade para 3B, condicionada à elegibilidade (`DECISIONS.md` D-026) |
| **C. ChatKit** | Widget de chat que exige backend próprio (`ChatKitServer`) ou fluxo hospedado | Taxa própria só para upload de arquivos; o resto é token. **Agent Builder encerra em 30/11/2026** (ChatKit continua) | Não recomendado: UI própria é requisito e o ganho é pequeno |
| **D. MCP** | OneHub exposto como servidor MCP para ChatGPT/apps consumirem | Exige endpoint HTTPS acessível ao cliente; modo desenvolvedor/planos conforme OpenAI | Não serve para app local; reavaliar para uso futuro via ChatGPT web |
| **E. Reutilizar token OAuth do Codex em proxy** | Terceiros fazem isso hoje | Sem garantia contratual; risco de bloqueio | **Rejeitada** (viola o princípio "não assumir capacidades") |

Arquitetura: interface `OrchestratorProvider`, com política e ferramentas no OneHub Rust. A base 3A está presente em alterações locais não commitadas; o provedor disponível no app é desconectado e o roteirizado serve apenas a testes. Plan usage é a prioridade da 3B, se elegível. API paga depende de autorização e orçamento; trocar provedor não deve alterar Hub/Gateway.

## 5. Protocolo OneNode (Hub ↔ Gateway)

- JSON versionado (`protocol_version`), mensagens request/response + eventos assíncronos, IDs de correlação (`task_id`, `run_id`).
- **Transporte local (MVP):** socket Unix em diretório do usuário, permissão 0600 + token de sessão.
- **Transporte remoto (futuro):** Gateway **não abre porta pública**. Hub conecta por túnel SSH a um listener em loopback do Gateway (ou o Gateway disca para fora); autenticação por pareamento de código único + chave do Hub; TLS mútuo sobre o túnel. Decisão final em D-007.
- Operações principais:

| Método | Função |
|---|---|
| `gateway.hello` / `gateway.capabilities` | Versão, SO, CLIs, modelos, skills/agents/commands/MCPs detectados |
| `workspace.list` / `workspace.authorize` | Diretórios autorizados (autorização é ato do usuário no Hub) |
| `task.start` | Motor, modelo, workspace, modo (`read_only`/`write`), prompt, contexto, limites (tempo, orçamento) |
| `task.event` (stream) | `message`, `tool_call`, `command`, `file_change`, `approval_request`, `error`, `usage` |
| `task.approve` / `task.deny` | Resposta a `approval_request` |
| `task.cancel` | Encerra árvore de processos |
| `task.result` | Saída final, código de saída, duração, evidências, diff |

## 6. Adaptadores de motor

Interface única `EngineAdapter { detect(), models(), capabilities(), start(task) -> event stream, cancel(), resume(session) }`.

| | Codex CLI | Claude Code | Cursor CLI |
|---|---|---|---|
| Execução | `codex exec --json` (simples) ou `codex app-server` (JSON-RPC stdio: threads/turns/aprovações; schema via `generate-json-schema`) [V/D] | `claude -p --output-format stream-json --input-format stream-json --verbose` [V] | `cursor-agent -p --output-format stream-json [--stream-partial-output]` [V/D] |
| Sandbox/permissão | `-s read-only/workspace-write`, `codex sandbox`; app-server envia pedidos de aprovação [V/D] | `--permission-mode`, `--permission-prompts host`/`--permission-prompt-tool` para decidir no Hub; `--restricted` [V] | `--mode plan/ask` (somente leitura), `--sandbox enabled`, `-f` libera escrita; aprovação interativa no meio é limitada [V/D] |
| Modelos | `codex debug models` (catálogo JSON) [V existe] | só aliases (`fable/opus/sonnet`) ou nome completo; **sem comando de listagem** [V] | `--list-models` [V] |
| Sessão/retomada | `exec resume`, `fork`; threads no app-server [V] | `--session-id`, `--resume`, `--fork-session` [V] | `--resume`, `--continue` [V] |
| MCP/skills/agents | `codex mcp`, plugins, `agents` [V] | `--mcp-config`, `--agents`, `--plugin-dir`, skills [V] | `--plugin-dir`, `--approve-mcps`, regras em `cli.json` [V/D] |
| Autenticação | login ChatGPT [V] | login claude.ai [V] | conta Cursor [V] |
| Risco específico | `app-server` marcado experimental no `--help` | política de uso da assinatura em automação variou em 2026 [D] | sem hook de aprovação mid-run → só rodar escrita em worktree isolada e revisar diff |

Estratégia: MVP com **`exec --json` / `-p stream-json`** (mais estáveis), usando **o login da própria CLI do usuário**; o app-server do Codex não é usado com tokens ChatGPT, porque a documentação proíbe uso comercial ou hospedado (`DECISIONS.md` D-015). Protocolo de aprovação **antes** do início (modo, diretório, escopo). Aprovação durante a execução entra onde o motor suporta (Claude host, Codex app-server) — decisão em D-006.

Todos os adaptadores normalizam para o mesmo esquema de eventos; o evento bruto é guardado para auditoria/depuração.

## 7. OneContext

- Tabelas: `projects`, `sources` (arquivo/commit/URL, hash, data), `chunks` (FTS5), `summaries` (por projeto/módulo/versão, com `status`: `proposed | validated | official`), `decisions`, `context_updates` (proposta, diff, origem, quem validou).
- Ingestão: Gateway devolve árvore e arquivos relevantes; o Hub indexa. Leitura dirigida (README, docs, manifests) antes de varredura completa.
- Recuperação: por `projeto`, `módulo`, `versão/commit`, `origem`; FTS5 + ranking simples no MVP; embeddings opcionais depois (`sqlite-vec` [?]).
- **Pacote de contexto por tarefa:** limite de tokens definido; inclui só resumos + trechos citados com origem. Nunca a documentação inteira.
- Atualizações propostas pelo orquestrador entram como `proposed`; só viram `validated`/`official` por ação humana. Informação `official` não é sobrescrita automaticamente.
- Troca de motor: o pacote de contexto é independente do motor; o adaptador o traduz para o formato do prompt da CLI.

## 8. Hierarquia de instruções (aplicada em código)

1. **Políticas de segurança e autorizações** (Hub + Gateway; não editáveis por prompt)
2. System Prompt padrão do OneOrchestrator
3. Instruções do projeto
4. Contexto validado do OneContext
5. Solicitação atual do usuário

Conteúdo lido de repositórios/saídas de ferramentas é **dado não confiável**, abaixo do nível 5. Detalhe em `ORCHESTRATOR.md` §4.

## 9. Estrutura do repositório (previsão de fases; a estrutura real está em §13.2)

```
OneAgent/
├── docs/                      # PRD, ARCHITECTURE, ROADMAP, SECURITY, DECISIONS, ORCHESTRATOR
├── protocol/                  # JSON Schemas versionados (fonte dos tipos)
├── apps/
│   └── desktop/               # Tauri 2: src-tauri (núcleo Hub) + src (React/TS)
├── crates/
│   ├── one-protocol/          # tipos gerados + serialização
│   ├── one-hub/               # policy engine, tarefas, aprovações, provedores do orquestrador
│   ├── one-context/           # OneContext (SQLite/FTS5)
│   ├── one-gateway/           # binário do Gateway (OneNode)
│   └── one-adapters/          # codex, claude, cursor
├── orchestrator/              # prompt padrão versionado, definições de ferramentas, suíte de avaliação
├── tests/                     # e2e, segurança (injeção, escape de diretório), contrato dos adaptadores
├── scripts/                   # geração de tipos, empacotamento
├── .github/workflows/         # CI e releases do Gateway (GitHub, runner próprio quando aplicável)
└── Cargo.toml / package.json / pnpm-workspace.yaml
```

Regras: dependências só apontam para dentro (`adapters → protocol`; `hub → protocol, context`; UI só via comandos); nenhum crate de domínio depende do Tauri.

## 10. Observabilidade

Logs JSON com `ts, level, component, task_id, run_id, event`; eventos brutos de motores em tabela separada com retenção configurável; redação de segredos antes de gravar; exportação de um `run` completo para diagnóstico.

## 11. Compatibilidade futura

Gateway em Rust sem dependências do SO além de processos/sockets: Linux e Windows entram trocando (a) socket Unix por named pipe, (b) estratégia de sandbox, (c) caminhos de descoberta das CLIs. Releases por tag no GitHub com binários por alvo, checksums e assinatura; instalação sem root (diretório do usuário, serviço de usuário via launchd/systemd --user).

## 12. Perguntas de arquitetura em aberto

Ver `DECISIONS.md`.

## 13. Estado da implementação (Fase 1)

### 13.1 Topologia atualizada (multi-cliente)
- **Mac:** OneAgent.app (cliente) + OneHub + OneContext com metadados e documentação consolidada.
- **Cada Gateway:** ambiente completo e isolado (repositórios, credenciais, CLIs, skills, logs). Detalhes em `GATEWAY.md`.
- **Gateway local** (este Mac) existe nesta fase só como detecção de leitura; a execução de tarefas entra na Fase 2.
- Credenciais não sincronizam entre clientes ou Gateways.

### 13.2 Estrutura real
```
OneAgent/
├── Cargo.toml · rust-toolchain.toml (1.100.0) · .cargo/config.toml (exporta tipos TS)
├── package.json · pnpm-workspace.yaml · scripts/check-contracts.mjs
├── docs/                     PRD, ARCHITECTURE, ROADMAP, SECURITY, DECISIONS, ORCHESTRATOR,
│                             GATEWAY, CAPABILITIES, CONTEXT, TOKEN-OPTIMIZATION
├── protocol/fixtures/        JSON compartilhado entre Rust e TypeScript
├── crates/
│   ├── one-protocol/         contratos (serde + ts-rs), 23 testes
│   ├── one-store/            SQLite, migrações V1–V2, FTS5, auditoria append-only, aprovações de executáveis, 11 testes
│   └── one-adapters/         descoberta/confiança de executáveis (trust), execução com limites (proc), proteção contra excesso de chamadas (guard), verificação de ACL do macOS (acl), 41 testes
└── apps/desktop/
    ├── src-tauri/            shell Tauri 2, comandos: store_status, detect_local_gateway
    └── src/                  React + TS: App, 7 telas, demo/ (dados marcados), contracts/generated/
```

### 13.3 Versões fixadas (verificadas em 2026-11-17)
Rust 1.100.0 · tauri 2.12 (crate) · tauri-build 2.7 · rusqlite 0.43 (bundled, com FTS5) · ts-rs 12 · serde 1 · thiserror 2 · React 19.3.1 · Vite 8.3.8 · TypeScript 5.9.3 · Vitest 5.1.0 · jsdom 31 · pnpm 12.21.0.

### 13.4 Estado
Fase 1 concluída (tag `fase-1-baseline`): app abre e navega; detecção local somente leitura com aprovação humana; SQLite V2; contratos testados. Próximo: execução real pelo Gateway local (Fase 2). Testes: ver `ROADMAP.md`.

## 14. Execução local (Fase 2A)

```
UI (React) ──invoke──► comandos Tauri (ACL por comando) ──► ExecService (crate one-exec)
   ▲ polling get_run                                          │ valida workspace, revalida identidade, trava por workspace
   │                                                          ▼
SQLite (runs, run_events, projects) ◄── eventos ◄── EngineParser ◄── stdout ◄── Codex CLI (grupo de processos, read-only)
```
- `one-exec`: `ExecService` (start/cancel), `StoreTrust`, `workspace::validate`. Testes com `codex` falso (script real) em `tests/exec.rs` e um teste real sob demanda em `tests/real_codex.rs` (`--ignored`).
- `one-adapters`: `EngineAdapter`/`EngineParser` (contrato de motor), `CodexAdapter`, `Host::run_stream`, `acquire`.
- `one-store` V3: métricas por execução, `engine_session_id`, `run_events.seq`.
- Workspaces autorizados reutilizam `projects` (Gateway `local` criado na abertura do app).

## 15. Camada de orquestração (Etapa 3A; implementada localmente, sem commit)

Crate `one-orchestrator`: contrato `OrchestratorProvider` (status, turno com eventos), `DisconnectedProvider` (registrado no app), `ScriptedProvider` (**somente testes**), catálogo tipado e laço com limites. O `ExecService` registra a origem `orchestrator` ou `user`; a execução continua somente leitura. Migração V4 adiciona modo da conversa, iniciador da tarefa e eventos do orquestrador. A UI separa os modos e informa o estado real sem apresentar o simulador como ChatGPT conectado. `pnpm check` e `pnpm build` passaram. Plano e critérios: `ORCHESTRATION-PLAN.md`.
