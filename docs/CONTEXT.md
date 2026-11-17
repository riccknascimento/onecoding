# OneContext — conhecimento persistente por projeto

> Fase 1 · 2026-11-17 · Planejamento. Implementação: esquema SQLite + FTS5 em `crates/one-store/src/migrations.rs` (V1). A ingestão e a recuperação são Fase 5.

## 1. O que é

Memória de projeto que sobrevive à troca de motor e de sessão: resumos técnicos, decisões, convenções e referências a evidências. Responde à pergunta "o que já sabemos sobre este projeto, de onde vem isso e quão confiável é?".

## 2. Onde mora cada coisa

| Dado | Mac (OneAgent.app) | Gateway do cliente |
|---|---|---|
| Metadados do projeto (nome, gateway, caminho no Gateway) | ✔ | — |
| Documentação **consolidada** (resumos validados) | ✔ | — |
| Trechos citados com origem (arquivo, linhas, hash) | ✔ (só o trecho usado) | original completo |
| Referências a evidências (`gw://…/runs/<id>/log`) | ✔ | ✔ (conteúdo) |
| Código-fonte, clones, `.env`, logs completos | ✘ | ✔ |
| Decisões e histórico de conversas | ✔ | — |

Regra: o Mac **não** guarda cópia integral de repositório ou log do cliente. Quando é preciso, o Hub busca o trecho sob demanda no Gateway.

## 3. Modelo de dados (migração V1, já aplicada)

- `projects(id, gateway_id, path, instructions)`: `instructions` é o nível 3 da hierarquia (instruções do projeto).
- `context_sources(id, project_id, origin, ref, version, content_hash, captured_at)`: proveniência. `origin` ∈ `file | commit | run | message | url`.
- `context_chunks(id, source_id, module, body, status)`: `status` ∈ `proposed | validated | official`.
- `context_chunks_fts` (FTS5, `unicode61 remove_diacritics 2`): índice sincronizado por triggers, então um `DELETE` remove o termo do índice. Coberto por teste.
- `run_events` e `audit_log`: `audit_log` é append-only (triggers impedem `UPDATE` e `DELETE`), para a trilha de auditoria.

## 4. Ciclo de vida de um item

```
proposed ──(humano valida)──► validated ──(humano promove)──► official
    ▲                                                            │
    └──────────── nunca rebaixado nem sobrescrito automaticamente ┘
```

- O orquestrador só cria `proposed` (via `context.propose_update`).
- Mudança em item `official` gera um **novo** item `proposed` ligado ao anterior; o oficial continua até ser substituído por decisão humana.
- Toda promoção registra quem validou e quando.

## 5. Recuperação (Fase 5)

Consulta sempre por projeto e pode filtrar por módulo, versão/commit e origem. Ordem de ranking proposta: `official` > `validated` > `proposed`, depois relevância FTS5. Pacote devolvido ao orquestrador:

- no máximo N itens (orçamento em `TOKEN-OPTIMIZATION.md` §4);
- cada item com `origem`, `versão` e `status`;
- nunca a documentação inteira.

## 6. Ingestão (Fase 5)

1. O Gateway lista a árvore e devolve **somente** o que foi pedido: README, documentação-chave, manifestos, arquivos apontados pela tarefa.
2. O Hub gera resumos estruturados (o modelo é chamado só quando o resumo for necessário; ver `TOKEN-OPTIMIZATION.md`).
3. Cada resumo guarda o `content_hash` da origem. Se o hash mudar, o item fica marcado como **desatualizado** e é reconfirmado sob demanda, em vez de reprocessar tudo.

## 7. Troca de motor

O pacote de contexto não depende do motor. O adaptador só converte o formato do prompt da CLI escolhida. Assim, trocar de Codex para Claude mantém o mesmo conhecimento sem reler o repositório.

## 8. Critérios de aceitação

- Item de contexto sempre tem origem e status (teste de schema: `CHECK` no banco).
- Busca FTS5 encontra termos sem acento e some após remoção (coberto).
- Atualização proposta nunca sobrescreve `official` (teste de regra a implementar na Fase 5).
- Troca de motor reaproveita o pacote (AC-08 de `PRD.md`).
- Item com hash divergente aparece como desatualizado.

## 9. Fora do escopo do MVP

Embeddings e busca vetorial (só se AC-08 falhar) · sincronização entre Macs · compartilhamento entre Gateways (proibido por política, ver `GATEWAY.md` §1).
