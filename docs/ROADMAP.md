# OneAgent — Roadmap

> 2026-11-17. **Regra:** cada fase termina com relatório e **só avança com autorização do usuário**. Estimativas são ordens de grandeza para um desenvolvedor principal + revisão do arquiteto, sujeitas a revisão após os spikes.

## Fase 0 — Planejamento (concluída nesta entrega)
Ambiente verificado, requisitos analisados, riscos e decisões, arquitetura, `docs/*`, MVP e critérios. **Saída:** este conjunto de documentos. Aguardando aprovação.

## Fase 1 — Fundação do aplicativo (**concluída** · baseline `fase-1-baseline`)

**Entregue e verificado:**
- App macOS (Tauri 2 + React + TS) abre, tema escuro, 7 seções, dados de demonstração marcados com selo DEMO. Validado por captura da janela e por testes.
- Workspace Rust modular: `one-protocol` (contratos, tipos TS gerados), `one-store` (SQLite V2, FTS5, auditoria append-only, aprovações), `one-adapters` (descoberta, confiança, execução com limites, ACL).
- **Confiança de executáveis:** descobrir ≠ executar; aprovação humana por diálogo nativo (validado manualmente pelo usuário); identidade persistida; mudança invalida; dono, permissões e ACLs verificados.
- Processos: grupo próprio, timeout que encerra descendentes, saída limitada, sem bloqueio.
- Migração V2 (FK composta Gateway/projeto) aplicada com backup; integridade validada.
- Tauri: ACL por comando, CSP sem `unsafe-inline`, throttle da detecção.
- Pesquisa oficial sobre o provedor do orquestrador registrada em `DECISIONS.md` (provedor segue desacoplado; sem chamadas pagas).
- 4 revisões adversariais do Codex; a última sem achados materiais.

**Testes no fechamento:** Rust `one-adapters` 41 (+1 manual), `one-store` 11, `one-protocol` 23, `oneagent-desktop` 3; frontend 15; contratos Rust↔TS em dia; `fmt` e `clippy -D warnings` limpos; `pnpm check` e `pnpm build` ok.

**Riscos residuais conhecidos:** `SECURITY.md` §12.

## Fase 2 — Agent Gateway local + adaptadores (≈ 2 semanas)
Descoberta de CLIs/versões/login/modelos; workspaces autorizados; `task.start/event/cancel/result`; adaptadores Codex, Claude, Cursor (somente leitura primeiro); coleta de evidências; testes com CLIs falsas e reais.
**Aceite:** AC-02, AC-03, AC-04, AC-05, AC-07, AC-13 (parte Gateway).


**Pendências herdadas da Fase 1 (entram na Fase 2):**
- Spikes de execução real por motor (`codex exec --json`, `claude -p stream-json`, `cursor-agent -p stream-json`) e confirmação do modelo reportado.
- Decidir D-005 (sandbox) e D-006 (aprovação durante a execução).
- Reduzir a janela entre verificação e `exec` (revalidar a identidade imediatamente antes de cada execução); avaliar cobertura do pacote npm e do interpretador.
- Provedor do orquestrador continua desacoplado (D-001/D-010) até haver orçamento aprovado.

**Preparação arquitetural (sem implementar ainda):** a execução real pelo Gateway local deve manter os contratos prontos para (a) **OneContext Handoff** — passar o contexto validado de um projeto entre motores e sessões; (b) **OneUsage** — medir tokens, custo e tempo por execução (base: `TOKEN-OPTIMIZATION.md` §8); (c) **múltiplos Gateways** — identidade, política e dados isolados por Gateway (`GATEWAY.md`). Cada um terá documento e fase próprios.

## Fase 3 — OneHub: política, aprovações, persistência (≈ 2 semanas)
Policy engine, objeto de aprovação assinado, SQLite (conversas, tasks, runs, auditoria), Keychain, logs estruturados, modo `write` em worktree isolada.
**Aceite:** AC-06, AC-11, AC-14 e testes de segurança do `SECURITY.md` §9.

## Fase 4 — App e chat com OneOrchestrator (≈ 2–3 semanas)
UI de chat com streaming, seleção de Gateway/motor/modelo, painel de execução, aprovações; provedor do orquestrador; ferramentas do Hub; prompt padrão v1.
**Aceite:** AC-01, AC-09, AC-10; suíte `ORCHESTRATOR.md` §12 (metas do MVP).

## Fase 5 — OneContext (≈ 2 semanas)
Ingestão, resumos, recuperação por projeto/módulo/versão/origem, pacotes com orçamento, propostas e validação, troca de motor com reaproveitamento.
**Aceite:** AC-08, OR-15, OR-16.

## Fase 6 — Configurações → Orquestrador e skills/agents/MCPs (≈ 1–2 semanas)
Editor de prompt com versões e restauração, instruções por projeto, políticas por Gateway, preferências, catálogo de skills/agents/commands/MCPs e seleção.
**Aceite:** AC-12, OR-11, OR-17.

## Fase 7 — Endurecimento e aceitação do MVP (≈ 1–2 semanas)
Evals completos, suíte de injeção, tratamento de falhas, desempenho, documentação de uso.
**Aceite:** todos AC-01..AC-14 e OR-01..OR-18 conforme metas; relatório final do MVP.

## Fase 8 — Gateway remoto (VPS Apliflow) (≈ 2 semanas)
Transporte seguro (D-007), pareamento, instalação sem root por release do GitHub, políticas por Gateway, testes de rede e reconexão. **Só após o MVP local aprovado e autorização explícita para acessar a VPS.**

## Fase 9 — Linux e Windows (Gateway) (≈ 2–3 semanas)
Named pipes, descoberta de CLIs por SO, sandbox por SO, releases multiplataforma, serviço de usuário.

## Fase 10+ — Evolução
Execução paralela, embeddings, segundo provedor do orquestrador (B), multiusuário, assinatura/notarização e auto-update.

## Dependências críticas

D-001 (provedor) bloqueia a Fase 4 · D-008 (termos) bloqueia a Fase 2 · D-009 (toolchain) bloqueia a Fase 1 · Fase 3 antes de qualquer escrita real.
