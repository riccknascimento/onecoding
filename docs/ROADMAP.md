# OneAgent — Roadmap

> 2026-11-17. **Regra:** cada etapa termina com relatório e só avança com autorização do usuário. A sequência vigente é a revisão de rumo no fim deste documento; as fases numeradas anteriores são registro histórico.

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

## Fase 2A — Primeira execução real pelo Gateway local, só Codex (**implementação técnica concluída na branch `feature/fase-2-local-execution`**; validação manual ponta a ponta **pendente**; sem merge, sem tag)
App → comando Tauri → `one-exec` → Codex aprovado (identidade revalidada antes do exec) → eventos normalizados no SQLite → Chat com histórico, cancelamento e métricas OneUsage. Somente leitura, uma execução por workspace, workspace por seletor nativo. Detalhes: `SECURITY.md` §13, `DECISIONS.md` D-020 a D-023, `TOKEN-OPTIMIZATION.md` §10.
Evidência: execução real com o Codex em workspace temporário (sucesso, 9 eventos, uso informado, workspace intacto) e 12 testes de integração do pipeline com um motor falso.
**Isolamento de leitura (fechamento da 2A):** perfil de permissão oficial do Codex como padrão; diagnóstico, alternativas e recomendação por modo em `ISOLATION.md`.
**Encerramento (decisões D-025):** leitura confinada ao workspace como padrão e único modo da interface; `TrustedLocalRead` fora da interface; Git/Node indisponíveis no agente por decisão; perfil Beta não é garantia absoluta; sem sandbox de Linux/Windows. Modos futuros: Investigação Restrita (atual), Desenvolvimento Isolado, Administração Autorizada (`ISOLATION.md` §6).
**Pendente para aprovação definitiva da 2A:** validação manual ponta a ponta na interface (aprovar Codex, autorizar pasta, enviar tarefa, conferir histórico após reinício) e confirmação no banco. A interface **não** está validada sem esse teste.
**Registro:**  confirmação do teste manual pela interface (o banco do usuário ainda não tem aprovações, workspaces nem execuções).
**Pendências da execução local:** validação manual ponta a ponta da interface e confirmação no banco; manter fixada a versão do Codex e reexecutar `os_level_profile_denies_reads_outside_the_workspace` a cada atualização. As demais limitações da 2A estão em `SECURITY.md` §14.

## Plano anterior

As fases numeradas abaixo da 2A foram substituídas pela sequência de prioridades aprovada em 2026-11-17. Use `ORCHESTRATION-PLAN.md` como fonte operacional para escopo, critérios, segurança e dependências.

## Revisão de rumo (2026-11-17) — sequência vigente

A Fase 2A implementa o **Modo direto** com Codex. A base 3A está concluída e verificada localmente, ainda sem commit. Ordem aprovada; cada etapa depende de autorização:

1. **3A — Fundação do OneOrchestrator (concluída localmente):** interface, provedor roteirizado de testes, catálogo tipado e política Rust, separação visual dos modos e estado real. Sem rede ou chamadas pagas.
2. **3B — ChatGPT Plan Usage:** priorizar Sign in with ChatGPT se oficialmente disponível e elegível. OAuth PKCE e Keychain. Bloqueio deve ser documentado com alternativas; API paga exige autorização separada.
3. **Provisionamento remoto — Gateway Linux via SSH:** primeiro alias `flowsharp`; inspeção somente leitura, plano de instalação, depois registro/autenticação/heartbeat. Nenhum acesso ou alteração à VPS sem aprovação explícita.
4. **Backlog posterior:** Claude Code e Cursor; OneContext Handoff; OneProject Discovery; OneSkills Registry; OneUsage; OneAgent Factory.

Requisitos de contexto, economia de tokens, isolamento por cliente e Git por Gateway: `ORCHESTRATION-PLAN.md` §12.
