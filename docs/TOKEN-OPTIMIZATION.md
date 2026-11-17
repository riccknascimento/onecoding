# Otimização de tokens e de chamadas ao LLM

> Fase 1 · 2026-11-17 · Princípio: **o LLM é o último recurso**. Antes dele, usar ferramentas determinísticas; depois, o menor modelo e o menor contexto que resolvem a tarefa.

## 1. Regras gerais

1. **Determinístico primeiro:** listagem de arquivos, busca por texto, leitura de trecho, leitura de versão de CLI, parse de JSON, contagem de linhas, testes e lint são feitos sem LLM.
2. **Contexto incremental:** cada turno envia só o delta (mensagem nova + resumo rolante + pacote do OneContext). Nada de reenviar o histórico inteiro.
3. **Consulta direcionada:** antes de varrer um repositório, buscar por termos e ler só os arquivos indicados pelo resultado.
4. **Logs ficam no Gateway:** o orquestrador recebe o resumo estruturado e, no máximo, trechos com referência (`evidence_refs`). Nunca o log bruto.
5. **Carregamento progressivo de skills:** o índice (nome + 1 linha) é o único custo fixo; o corpo de uma skill entra quando ela é selecionada.
6. **Uma chamada é melhor que várias:** a CLI faz o trabalho agentic dentro do seu próprio loop; o orquestrador não repete o que ela já fez.
7. **Sem conversa contínua entre agentes:** não existe múltiplos agentes trocando mensagens em loop. Só um caminho de ida e volta, com etapa explícita. Qualquer paralelismo entra só com necessidade medida.
8. **Motor e modelo escolhidos explicitamente** pelo usuário; o padrão é o modelo menor que cumpre a tarefa, declarado na delegação.

## 2. Escada de decisão (menor custo primeiro)

| Nível | Quando | Custo | Exemplo |
|---|---|---|---|
| 0 | Resposta vem de dado local | zero LLM | "qual versão do Codex está instalada?" |
| 1 | Resposta vem do OneContext validado | só leitura local + pouco contexto | "como o gateway autoriza diretórios?" |
| 2 | Tarefa simples de uma CLI | 1 execução direta | "rode os testes do módulo X" |
| 3 | Tarefa complexa de uma CLI | 1 execução + revisão do orquestrador | "investigue a arquitetura do SystemFlow" |
| 4 | Revisão por segundo motor | 2 execuções | só quando justificada (§6) |

O orquestrador começa no nível mínimo que resolve. Subir de nível exige motivo registrado.

## 3. Carregamento progressivo de skills e capacidades

- Índice enviado ao orquestrador: `{id, nome, descrição de 1 linha, motor, escopo}`, até ~60 tokens por item (`CAPABILITIES.md` §1.6).
- Ao selecionar, o Gateway entrega o corpo da skill **para a CLI** (ela carrega do próprio diretório), e o orquestrador não o copia para o próprio contexto.
- Limite de capacidades anexadas por tarefa: 3, salvo justificativa.

## 4. Orçamento por tarefa e por conversa

| Limite | Padrão proposto | Quem aplica |
|---|---|---|
| Contexto do OneContext por turno | ≤ 3.000 tokens | Hub |
| Turnos do orquestrador por tarefa | ≤ 6 | Hub |
| Tentativas automáticas por etapa | ≤ 2 | Hub (ver `ORCHESTRATOR.md` §11) |
| Custo máximo por tarefa | definido pelo usuário; sem valor padrão até haver orçamento aprovado | Hub |
| Tempo máximo por execução | 30 min; 5 min sem progresso | Gateway |
| Orçamento mensal por conversa/projeto | definido pelo usuário (D-010) | Hub |

Ao atingir o limite, a execução para com aviso e o usuário decide. Não há retentativa silenciosa.

## 5. Resumos estruturados

Formato mínimo de um resumo de execução, guardado no Mac:

```
run: <id> · motor: <x> · modelo: <y, confirmado|não confirmado>
status: <succeeded|failed|…> · exit: <n> · duração: <s>
fatos (com evidência): - …  [ref]
hipóteses: - …
arquivos tocados: <lista curta>
erros: <tipo + trecho ≤ 5 linhas + ref>
próxima ação sugerida: …
```

Esse resumo substitui o histórico bruto na conversa. Um resumo de até ~300 tokens cobre uma execução que gerou dezenas de milhares de linhas no Gateway.

## 6. Revisão por segundo motor — quando justificar

Só quando, pelo menos, uma destas condições for verdadeira:
- mudança em autenticação, dados, pagamentos, migrações ou permissões (o mesmo critério da revisão obrigatória do fluxo do usuário);
- o primeiro motor reportou sucesso sem evidência suficiente;
- o usuário pediu explicitamente.

Caso contrário, a revisão é feita pelo próprio orquestrador sobre o diff e os testes, que custa menos.

## 7. Sem LLM onde não precisa

| Tarefa | Ferramenta |
|---|---|
| Detectar CLIs, versão, login e modelos | `one-adapters` (processos e parse) |
| Listar skills e MCPs instalados | leitura de diretórios e arquivos de configuração |
| Verificar hash de arquivo ou contexto desatualizado | SHA-256 |
| Buscar no OneContext | FTS5 |
| Validar esquema de contratos | `one-protocol` e testes de fixture |
| Rodar testes, lint, typecheck e build | comandos do projeto (no Gateway, com aprovação quando alteram algo) |
| Detectar segredos e padrões de injeção em conteúdo | regras determinísticas (expressões regulares e listas), antes de qualquer LLM |

## 8. Medição

Métricas registradas por execução: tokens de entrada e saída do orquestrador, chamadas ao modelo, tentativas, tamanho do pacote do OneContext, tempo até o primeiro resultado útil. Meta do MVP: **OR-18** em `ORCHESTRATOR.md` (orçamento de tokens por tarefa dentro da meta definida) e **OR-15** (contexto ≤ orçamento).

Sem medição não há otimização: a primeira entrega da Fase 2 já grava esses números, para que as metas sejam ajustadas com dados e não por palpite.

## 9. O que não fazer

- Multiplicar agentes que conversam entre si sem ganho mensurável.
- Reenviar a documentação inteira a cada mensagem.
- Chamar o LLM para tarefas que um comando resolve.
- Repetir uma ação que já falhou duas vezes com o mesmo resultado.
- Trocar de modelo para "garantir" qualidade sem medir.

## 10. OneUsage básico (Fase 2A, implementado)

Por execução, em `runs` (migração V3): duração (`duration_ms`), motor, modelo solicitado (`tasks.model`), modelo **informado** (`model_reported`, vazio se o motor não informa), status, `tool_calls` (contagem das chamadas de ferramenta observadas), tokens de entrada/saída/cache/raciocínio **como informados pelo motor** (`NULL` quando ausentes), `attempts` (1 nesta fase; sem retentativa automática) e erro (`error_kind`/`error_message`). Nada é estimado. Exemplo real (Codex 0.169.0): 45.017 tokens de entrada, 34.048 em cache, 169 de saída, 2 chamadas de ferramenta, 9,9 s.

Fora do escopo desta fase: painel, percentuais de assinatura, preço e troca automática de motor.
