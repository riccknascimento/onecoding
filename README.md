# OneAgent

Central de orquestração de agentes de IA para macOS. O OneAgent.app conversa com um orquestrador e delega tarefas a Agent Gateways, que executam Codex CLI, Claude Code e Cursor CLI nos seus próprios ambientes.

**Estado:** Fase 1 concluída (fundação local). Detecta as CLIs e exige aprovação humana antes de executá-las; ainda não executa tarefas. Veja `docs/ROADMAP.md`.

## Requisitos

- macOS (Apple Silicon testado)
- Node 22+ e pnpm 12.21 (via Corepack)
- Rust 1.100.0 (`rust-toolchain.toml` instala a versão certa com rustup)

## Comandos

```sh
pnpm install            # dependências JS
pnpm dev                # app em modo desenvolvimento (Tauri)
pnpm build              # gera OneAgent.app em target/release/bundle/macos
pnpm check              # typecheck, fmt, clippy, testes Rust e JS, e contratos
```

`ONEAGENT_DATA_DIR` aponta o banco para outro diretório (útil em testes). Por padrão ele fica em `~/Library/Application Support/br.com.apliflow.oneagent/`.

## Estrutura

- `crates/one-protocol`: contratos tipados; gera os tipos TypeScript em `apps/desktop/src/contracts/generated`.
- `crates/one-store`: SQLite com migrações e FTS5.
- `crates/one-adapters`: detecção somente leitura das CLIs.
- `apps/desktop`: shell Tauri 2 e interface React.
- `docs/`: planejamento, arquitetura, segurança e decisões.

## Documentação

`docs/PRD.md` · `docs/ARCHITECTURE.md` · `docs/DECISIONS.md` · `docs/SECURITY.md` · `docs/ORCHESTRATOR.md` · `docs/GATEWAY.md` · `docs/CAPABILITIES.md` · `docs/CONTEXT.md` · `docs/TOKEN-OPTIMIZATION.md` · `docs/ROADMAP.md`
