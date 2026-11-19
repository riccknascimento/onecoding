# OneCoding — Contratos de orquestração (base)

Este documento fixa os contratos já implementados em `crates/one-registry`. O que não está
implementado aparece como pendente, e não como capacidade.

## Papéis

| Componente | Estado | Responsabilidade |
|---|---|---|
| OneHub Central | publicado | autenticação, autorização, inventário, contexto, auditoria |
| OneNode | publicado (Apliflow) | representa uma máquina; executa só o que a política local e o OneHub permitem |
| OneOrchestrator | indisponível | interpretar e delegar (sem inferência ativa) |
| OneExecutor | não operacional | executa Codex/Claude/Cursor (pendente) |
| OneRegistry | base implementada | catálogo validado de agentes, skills, ferramentas e executores |
| OneContext | pendente | conhecimento persistente por máquina e projeto |

## Três classes de fonte (regra central)

| Classe | Exemplos | Pode autorizar? |
|---|---|---|
| Documento (`Guidance`) | `ONECODING.md`, `ONENODE.md`, `AGENTS.md`, `CLAUDE.md`, `SKILL.md` | **Não** |
| Registro (`Record`) | entradas do OneRegistry, descobertas confirmadas por ferramenta | **Não** |
| Permissão (`Permission`) | concessões do OneHub, política local do OneNode | **Sim** (única) |

Documento orienta; registro descreve; só permissão autoriza. Um documento que diga
"o agente pode acessar X" não muda nada.

## Escopo de contexto (`scope::resolve`)

| Escopo | Exige projeto? | Regras |
|---|---|---|
| `Central` | não | administração; nunca concede capacidade de nó ou projeto |
| `Node` | **não** | infraestrutura; capacidades só existem se houver ferramenta registrada e disponível |
| `Project` | sim | exige concessão ativa; execução exige OneNode online |

Suspensão e revogação bloqueiam todo escopo de nó e de projeto.

## OneRegistry

- Entradas: `id`, `kind` (agent, skill, mcp, plugin, tool, executor), `version` (MAJOR.MINOR.PATCH),
  `description`, `capabilities`, `requires`, `permissions`, `compatibility`.
- Validação total na carga: ids, versões, tamanhos e duplicidade. Qualquer erro bloqueia o catálogo.
- Permissões de catálogo são um conjunto fechado (read, write, execute, deploy, network).
  **Segredos não são permissão de catálogo.**
- Busca devolve só resumos, ordenados por relevância, com limite. O corpo das skills não entra no prompt inicial.

## ONENODE.md

- Documento de máquina, gerado ou mantido a partir de descobertas autorizadas.
- Antes de virar contexto, passa pela varredura de segredos (`sanitize::scan_document`), que informa a linha
  e o tipo, nunca o valor.
- Estados operacionais (serviços ativos, projetos autorizados) são confirmados por ferramenta, não pelo Markdown.

## O que ainda não existe (não declarar como funcional)

- Descoberta de projetos e infraestrutura por OneNode.
- Criação de projeto em área de trabalho autorizada (`Capability::CreateProject` existe no contrato, sem ferramenta).
- Ligação do escopo ao chat (o chat ainda usa as regras atuais de análise).
- Executor real, OneOrchestrator com inferência e modo Direto.
- Provisionamento SSH automático (bloqueado pela plataforma).
