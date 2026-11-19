# OneCoding Workspaces Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implementar Workspaces persistidos e abas independentes no OneCoding Web, mantendo Settings global e dados isolados por usuário.

**Architecture:** OneHub Central continua sendo o backend único. PostgreSQL guarda Workspaces, relações de contexto e associação das conversas; o navegador guarda apenas quais abas estão abertas e qual está ativa.

**Tech Stack:** Rust, Axum, tokio-postgres, PostgreSQL, React, TypeScript, Vitest.

**Spec:** `docs/superpowers/specs/2026-11-19-onecoding-workspaces-design.md`

## Global Constraints

- Não apagar nem reescrever dados de conversas existentes.
- Workspace é distinto de Projeto; UI mantém um contexto ativo por Workspace nesta etapa.
- Toda leitura/escrita de Workspace valida `owner_id`; contexto de projeto exige concessão ativa OneNode × projeto.
- OneNode identifica o servidor de um cliente, não um projeto. O modelo futuro deve permitir vários projetos por Workspace com identidade de cliente isolada e associação por projeto de caminho autorizado, remoto Git, branch e OneExecutor.
- O caminho autorizado vem da concessão OneNode × projeto (`root_path`); associação de repositório não concede acesso ao filesystem. Git exige capacidade/autorização separada e deploy segue autorização distinta.
- Não instalar GitHub nas VPS, executar operações Git ou alterar servidores durante esta entrega.
- Fechar aba só altera estado visual local; não chama endpoint de exclusão.
- Não incluir dados falsos de Workspace nem implementar módulos indisponíveis.
- Não implementar VibeAgent, novos motores, Workspaces multiprojeto ou abas internas por projeto.
- Não implementar configuração/execução de Git ou deploy nesta etapa; não presumir capacidade Git em OneNodes.
- Não executar push, merge ou deploy.

## Review Focus

- ID de Workspace de outro usuário: API retorna não encontrado, sem revelar existência.
- Migração com conversas de vários usuários e contextos distintos: todas permanecem acessíveis e contexto histórico não é trocado.
- Contexto revogado: seleção nova é recusada; conversa antiga continua histórica sem criar nova execução autorizada.
- Um OneNode associado a vários projetos não deve compartilhar caminho, remoto, branch, credenciais ou permissão entre clientes; repositório e deploy não ampliam a autorização já existente.
- Aba fechada e depois reaberta: histórico e tarefa em andamento continuam no servidor.
- localStorage contém ID apagado ou inacessível: UI descarta a referência e abre Settings sem erro.

## Arquivos e responsabilidades

- `crates/one-central/migrations/0011_workspaces.sql`: tabelas, índices, associação compatível de conversas antigas e contexto herdado.
- `crates/one-central/src/routes.rs`: endpoints e validação de propriedade, contexto, filtros e compatibilidade.
- `crates/one-central/tests/central.rs`: cobertura de criação/listagem/renomeação, isolamento, conversa e contexto.
- `apps/desktop/src/transport/types.ts`, `http.ts`: contratos e chamadas do OneHub existentes.
- `apps/desktop/src/central/WorkspaceTabs.tsx`, `SettingsHome.tsx`: abas e painel global.
- `apps/desktop/src/central/CentralApp.tsx`, `Sidebar.tsx`, `central.css`, `pages.tsx`: seleção ativa, histórico isolado, navegação e responsividade.
- `apps/desktop/src/__tests__/central-web.test.tsx`: fluxos de navegação e persistência visual.

---

### Task 1: Persistência e API de Workspace

**Interfaces:**
- Workspace JSON: `{ id, name, node_id, project_id, updated_at }`.
- Rotas: `GET/POST /v1/workspaces`, `PUT /v1/workspaces/{id}`, `PUT /v1/workspaces/{id}/context`.
- Conversas aceitam `workspace_id` opcional no POST e query opcional no GET.

- [ ] Adicionar cenários de integração para criar/listar/renomear, proibir acesso cruzado e selecionar contexto com concessão.
- [ ] Confirmar que os cenários falham antes da implementação.
- [ ] Criar a migration aditiva 0011: Workspaces pessoais, tabela extensível de contextos e associação de conversas históricas por usuário.
- [ ] Implementar endpoints autenticados, sempre com filtros explícitos por proprietário; manter criação/lista legada sem `workspace_id`.
- [ ] Associar nova conversa ao Workspace solicitado e herdar o contexto ativo; filtrar listagem quando houver `workspace_id`.
- [ ] Compilar e executar os testes de Central disponíveis; testes que exigem PostgreSQL externo permanecem condicionados ao banco de teste já documentado.

### Task 2: Contratos e navegação por abas

- [ ] Adicionar teste de transporte para os métodos de Workspace e payloads.
- [ ] Implementar tipos `CentralWorkspace`, `workspaces`, `createWorkspace`, `renameWorkspace`, `setWorkspaceContext`; estender conversa com `workspace_id`.
- [ ] Adicionar teste do componente de abas: Settings fixo, abrir workspace não duplica, fechar aba mantém dados, botão `+` lista existentes e cria.
- [ ] Implementar `WorkspaceTabs` e persistir apenas IDs de abas e aba ativa, reconciliando-os com os dados autenticados do servidor.
- [ ] Executar testes frontend e typecheck.

### Task 3: Sidebar e painel Settings da referência

- [ ] Adicionar testes para lista de Workspaces em Settings, histórico filtrado no Workspace ativo, nova conversa vazia com contexto e navegação Settings/Provider AI.
- [ ] Implementar sidebar que mostra Workspaces no Settings ou Workspace ativo + conversas na aba de projeto.
- [ ] Implementar o painel Settings com cards que navegam às configurações existentes e estados derivados dos endpoints reais; manter indisponibilidade explícita onde não há backend.
- [ ] Aplicar CSS responsivo alinhado à referência, sem adicionar outra biblioteca ou reformular tokens globais.
- [ ] Executar typecheck, todos os testes frontend e build web.

### Task 4: Isolamento, recuperação e entrega

- [ ] Testar seleção de conversa por Workspace e troca de abas durante stream; troca não chama cancelamento.
- [ ] Testar query string/histórico e restauração de abas após autenticação/reload.
- [ ] Validar acesso de membro/admin, provider e rotas de configuração.
- [ ] Rodar `pnpm test`, `pnpm typecheck`, `pnpm --filter @oneagent/desktop build:web`, `cargo fmt --all --check` e `cargo clippy --workspace --all-targets -- -D warnings`.
- [ ] Abrir em navegador real quando o controle estiver disponível; registrar indisponibilidade concreta caso contrário.
- [ ] Revisar o diff, stage apenas mudanças desta entrega e criar commit local sem publicar.
