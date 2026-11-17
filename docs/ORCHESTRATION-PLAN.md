# OneAgent — Plano da arquitetura de orquestração

> 2026-11-17 · **Plano aprovado.** A Fase 2A permanece como Modo Direto. A implementação 3A foi concluída e verificada neste working tree, ainda sem commit. OAuth real, chamadas pagas e acesso à VPS não estão autorizados.
> Reaproveita, sem repetir: `ORCHESTRATOR.md` (prompt, hierarquia, aprovações, testes de comportamento), `GATEWAY.md` (isolamento e provisionamento), `ISOLATION.md`, `TOKEN-OPTIMIZATION.md`, `CAPABILITIES.md`, `CONTEXT.md`.

## 1. Correção de rumo

A Fase 2A entregou o **Modo direto** (usuário → OneHub → Gateway → Codex). Foi útil para validar execução, aprovação, persistência e isolamento, mas **não é** a arquitetura definitiva. O produto é: o usuário conversa com o **OneOrchestrator (ChatGPT)**, que planeja e delega ao OneHub; o OneHub aplica a política e aciona Gateways e motores. O acesso direto às CLIs continua existindo, mas como modo separado e rotulado.

**Estado do trabalho:** a implementação 3A está presente como alterações locais não commitadas nesta branch. Este plano descreve o alvo e os critérios; o `git status` e a revisão do código determinam o que já está implementado e o que falta validar. Não confundir alterações locais com trabalho integrado ou fase concluída.

## 2. O que existe e o que falta

| Componente | Estado | Observação |
|---|---|---|
| OneAgent.app (Tauri, 7 seções, ACL por comando, CSP) | **Feito** | Chat hoje só executa em Modo direto; o resto é demonstração marcada |
| OneHub: persistência (V4 nas alterações locais), aprovação de executáveis, workspaces autorizados, auditoria | **Implementação local** | Falta política central por Gateway e fluxo de aprovação para ações sensíveis |
| Gateway local + Codex (execução real, leitura confinada, cancelamento, métricas) | **Feito**, validação manual pendente | Sem binário de Gateway separado; roda dentro do app |
| Contrato de motor (`EngineAdapter`/`EngineParser`) | **Feito** (só Codex) | Claude e Cursor ausentes |
| OneOrchestrator (provedor, loop de ferramentas, conversa) | **3A implementada e verificada localmente** | Contrato, provedor desconectado, provedor roteirizado de teste, persistência e loop limitado; nenhum provedor real |
| Ferramentas do OneHub para o orquestrador | **3A implementada e verificada localmente** | Catálogo tipado, políticas Rust e isolamento por conversa; escrita permanece bloqueada e execução read-only |
| OneContext (ingestão, recuperação, handoff) | **Só esquema** (V1) | FTS5 pronto, sem ingestão |
| SSH e provisionamento remoto | **Ausente** | Só planejamento (`GATEWAY.md` §6) |
| Claude Code e Cursor | **Ausentes** | |
| OneSkills Registry e Factory | **Ausentes** | |

## 3. Arquitetura-alvo e os dois modos

```
                Modo Orquestrador                                  Modo direto
  Usuário ─► OneOrchestrator (ChatGPT) ─► OneHub ─► Gateway ─► CLI      Usuário ─► OneHub ─► Gateway ─► CLI
     ▲              │  chamadas de ferramenta         │                      ▲                                │
     └── resposta ◄─┴── resultados (não confiáveis) ◄─┘                      └────────── resultado ◄──────────┘
```

Regras comuns (valem nos dois modos; **uma só implementação** no OneHub):
- Toda execução passa por `ExecService`: aprovação do executável, workspace autorizado, perfil de leitura, limites, cancelamento, persistência.
- O orquestrador **nunca** executa nada, nunca recebe caminho livre, SSH, shell nem credenciais. Só chama ferramentas tipadas do Hub, que validam e aplicam política.
- Resultados de ferramentas e conteúdo de repositórios são **dados não confiáveis** para o orquestrador (`ORCHESTRATOR.md` §3–§4).
- Cada mensagem e cada tarefa guardam a **origem**: `orchestrator` ou `direct`, e o iniciador (`user` ou `orchestrator`), para auditoria e métricas separadas. O Modo direto **nunca** é apresentado como ChatGPT.
- Aprovação de nível ≥ 1 (`ORCHESTRATOR.md` §10) é sempre humana, pelo mesmo mecanismo nos dois modos. No Modo Orquestrador o orquestrador **pede**; só o usuário aprova.

**Coexistência na interface:** um seletor explícito **Orquestrador | Direto**, persistente por conversa. Em Modo direto a barra mostra "Modo direto · sem ChatGPT" e o orquestrador não participa. Trocar de modo abre uma conversa nova (não se mistura histórico).

## 4. Integração do ChatGPT (opções oficiais)

Fatos de fontes oficiais lidas em 2026-11-17 (detalhe e URLs em `DECISIONS.md` D-001); o que não foi lido está marcado.

| Opção | Estado e restrições | Papel proposto |
|---|---|---|
| **A. Responses API com function calling e chave de API** | Oficial, cobra por token. Exige chave e orçamento; não usar por padrão | Alternativa somente após decisão e autorização específicas do usuário |
| **B. Sign in with ChatGPT / ChatGPT Plan Usage** | Preview, disponibilidade e elegibilidade sujeitas às regras atuais da OpenAI e à resposta real do fluxo. Protocolo OAuth/PKCE e limitações registradas em `DECISIONS.md` D-026. Não reutilizar sessão, cookies ou tokens internos do Codex | **Prioridade para 3B**, se oficialmente disponível e a conta/projeto forem elegíveis; se bloqueado, registrar evidência e apresentar alternativas antes de qualquer integração paga |
| **C. Remote MCP / MCP no ChatGPT** | Exige endpoint HTTPS público ou Secure MCP Tunnel; não cobre o app local e não é o caminho para o orquestrador do app | Não usar agora; reavaliar para expor o Hub a clientes externos |
| **D. ChatKit** | Exige servidor próprio ou fluxo hospedado; UI própria é requisito | Não usar |
| **E. Cookies, sessão da web, token do Codex, `backend-api`** | Não documentados e fora de política | **Proibido** |

**Regra de decisão:** tentar primeiro o fluxo oficial de plan usage na 3B, após fechar a validação documental e receber autorização para essa etapa. A elegibilidade não é presumida. Se houver bloqueio, documentar a resposta e comparar alternativas; API paga depende de autorização explícita e orçamento aprovado. Não alterar a visibilidade do repositório para contornar elegibilidade.

### 4.1 Interface de provedor

```rust
trait OrchestratorProvider: Send + Sync {
    fn id(&self) -> &'static str;                    // "openai_api", "chatgpt_oauth", "scripted"
    fn status(&self) -> ProviderStatus;              // Disconnected | Connected{account_hint, models} | Error
    fn complete(&self, req: TurnRequest, sink: &mut dyn FnMut(TurnEvent)) -> Result<TurnOutcome, ProviderError>;
}
// TurnRequest: instruções + mensagens + catálogo de ferramentas + limites (tokens, tempo)
// TurnEvent: TextDelta | ToolCall{id,name,args_json} | Usage{...} ; TurnOutcome: Final | NeedsTools | Refused
```
O Hub guarda o histórico e monta cada turno; o provedor só traduz o protocolo. A implementação local atual contém `ScriptedProvider` para testes e `DisconnectedProvider` no app. O provedor oficial de plan usage será considerado na 3B. Um provedor de API paga não será adicionado sem autorização.

### 4.2 Autenticação — arquitetura oficial proposta
- **Plan usage (3B):** após confirmar disponibilidade e elegibilidade, o núcleo Rust abre o navegador, usa OAuth com PKCE e callback loopback, valida `state`, `nonce` e tokens, e guarda credenciais no Keychain. Nunca reutilizar tokens internos do Codex. Sem credenciais em SQLite, logs, prompts ou webview; chamadas de rede só pelo Rust para endpoints oficiais.
- **API paga:** fora do escopo autorizado; só considerar com autorização explícita, teto e orçamento antes de implementar ou chamar.

### 4.3 Salvaguardas contra custo não autorizado
- **Sem chamada paga por padrão**: o estado inicial é "desconectado".
- Plan usage não deve acionar faturamento de API por fallback. Tratar cota esgotada como pausa e informar o estado.
- Qualquer eventual API paga exige teto duro no Hub e confirmação explícita de modelo, limite e custo antes da primeira chamada.
- Testes e desenvolvimento usam `scripted` e gravações; chamadas reais só em testes marcados `#[ignore]`, executados sob sua autorização.

## 5. Como o OneHub disponibiliza ferramentas ao orquestrador

Execução das ferramentas **no Hub (Rust)**, nunca no modelo e nunca na webview. Sem MCP local: function tools do Responses API bastam e mantêm a política no mesmo processo. Cada ferramenta: esquema JSON estrito, validação, política, auditoria, orçamento.

| Ferramenta | Mapeia para (já existe) | Política |
|---|---|---|
| `gateways.list`, `gateways.capabilities` | `detect_local_gateway` | leitura |
| `workspaces.list` | `list_workspaces` | leitura (só ids e rótulos) |
| `task.run{gateway, workspace_id, engine, model?, prompt, mode:"read_only"}` | `ExecService::start` | leitura: automática no workspace autorizado; `write`: **aprovação humana** |
| `task.status`, `task.events`, `task.result` | `get_run`, `list_runs` | leitura; saída truncada e marcada `untrusted` |
| `task.cancel` | `cancel_run` | permitido |
| `context.get`, `context.search`, `context.propose_update` | OneContext (a criar) | leitura; a proposta nasce `proposed` |
| `approval.request{ação, onde, por quê, impacto, reversão}` | diálogo nativo (a criar) | só o usuário decide |
| `capabilities.list` | OneSkills (a criar) | leitura |

Fora do catálogo, por desenho: shell, leitura de arquivo, rede, SSH, instalação, qualquer caminho livre.

**Ciclo de um turno:** Hub monta (prompt de sistema + contexto limitado + histórico + catálogo) → provedor faz streaming → `ToolCall` → Hub valida o esquema, aplica política, pede aprovação se preciso, executa → devolve `function_call_output` (truncado, redigido, rotulado) → repete até resposta final ou limite (turnos, tokens, tempo, tentativas). Tudo registrado (mensagens, chamadas, aprovações, custo).

**Limites herdados:** o orquestrador escolhe motor e modelo só dentre o que o Gateway **confirmou**; "modelo confirmado" vem do executor (o Codex hoje não informa: mostrar "não informado pelo motor").

## 6. Plano incremental (ordem aprovada)

Cada etapa só começa com a sua aprovação. Nenhuma faz chamada paga sem autorização, e **não haverá integração por chave de API** por ora.

| Etapa | Entrega | Custo | Critérios de aceitação |
|---|---|---|---|
| **3A — Fundação do OneOrchestrator** (**concluída localmente; sem commit**) | Contrato `OrchestratorProvider`; provedor simulado só para testes; catálogo tipado; política Rust; UI com modos separados e estado real | R$ 0, sem rede | `pnpm check` e `pnpm build` passam; delegação simulada, limites, argumentos estritos, proteção contra injeção, modo direto preservado, provedor desconectado sem respostas simuladas e escrita bloqueada |
| **3B — ChatGPT Plan Usage** | Confirmar disponibilidade/elegibilidade oficial; se elegível, OAuth PKCE, Keychain, validação de escopos, modelos, refresh e revogação | Cota do plano, sem API paga | Só iniciar após autorização da etapa; chamada real exige autorização expressa. Bloqueio documentado com alternativas propostas |
| **2B — Provisionamento remoto por SSH** | Via alias `flowsharp`: conexão, inspeção Linux somente leitura, plano de instalação do OneNode Linux, registro, autenticação e heartbeat | R$ 0 planejado | Nenhum acesso ou alteração na VPS até aprovação explícita para cada etapa. Depois desta etapa vêm Claude e Cursor |
| **Seguintes** | Claude Code e Cursor CLI → OneContext Handoff → OneProject Discovery → OneSkills Registry → OneUsage → OneAgent Factory | | **Somente planejados**; nada implementado antecipadamente |

Observação de nomenclatura: "2B" deixa de significar "adaptadores de Claude e Cursor" (documentos anteriores) e passa a ser o provisionamento remoto.

## 7. Provisionamento remoto por SSH (Etapa seguinte; planejamento)

Serviço **`ssh-controller` do OneHub**. O orquestrador e o Codex **não** têm ferramenta de SSH e não escolhem comando remoto.

- **Cadastro:** nome (ex.: FlowSharp) e o **alias SSH `flowsharp`**, já configurado no Mac do usuário. Sem senha; a chave é usada pelo próprio OpenSSH/agente e nunca é lida, copiada nem armazenada pelo OneAgent.
- **Usar a configuração existente, com cuidado:** antes de conectar, `ssh -G flowsharp` (**local, sem rede**) resolve a configuração efetiva, que o app mostra para confirmação. O app **recusa** alias cujo resultado contenha `ProxyCommand`, `LocalCommand`/`PermitLocalCommand`, `Match exec` ou `KnownHostsCommand` (executam comandos locais) e força opções seguras na linha de comando: `BatchMode=yes`, `StrictHostKeyChecking=yes`, sem `ForwardAgent`, sem `ForwardX11`, sem túneis. A conexão já existente (`ControlMaster` do usuário) pode ser reaproveitada.
- **Execução:** o `ssh` do sistema, pelo mesmo runner endurecido da Fase 2A (sem shell local, grupo de processos, prazo, limite de saída). Comandos remotos vêm de um **conjunto fixo e embutido**, com hash conhecido; nenhuma string do LLM chega ao shell remoto.
- **Identidade do host:** primeira conexão mostra o fingerprint (SHA-256) em diálogo nativo; mudança de chave bloqueia.
- **Etapas, cada uma com autorização própria:** (1) teste de conexão; (2) inspeção somente leitura (SO, arquitetura, usuário, disco, CLIs presentes, projetos candidatos, `git` por projeto); (3) plano de instalação mostrado a você; (4) envio de release **verificada** (checksum e assinatura) e instalação **sem root**, em diretório do usuário, serviço de usuário; (5) registro: identidade do Gateway, chave do Hub revogável, **heartbeat autenticado**; (6) inventário (projetos, CLIs, modelos, skills), com dados que ficam no Gateway.
- **Autorização atual: nenhuma.** O alias `flowsharp` não foi acessado. Esta etapa requer aprovação explícita para inspeção remota; instalação e registro exigem aprovação própria antes de alterar a VPS.
- **Gateway Linux independente:** binário próprio, sem privilégio administrativo permanente, com seus repositórios, credenciais, CLIs, logs e política (`GATEWAY.md`). Sandbox de Linux ainda não existe (`ISOLATION.md` §6).
- **Provas:** testes com `ssh` falso e `ssh -G` simulado (sem rede); a primeira prova real só com o seu aval.

## 8. Interface (requisitos)

Barra de estado fixa no Chat, sempre com dados reais ou "—":
`ChatGPT: Desconectado | Conectado (provedor, conta)` · `Modo: Orquestrador | Direto` · `Gateway` · `Projeto` · `Motor` · `Modelo solicitado` · `Modelo confirmado: <valor> | não informado pelo motor` · `Capacidades` (lidas do Gateway, nunca presumidas).
- Em **Modo direto** aparece o selo permanente "Modo direto · sem ChatGPT".
- Sem orquestrador conectado, o campo do Modo Orquestrador fica desabilitado com o motivo; **a conversa de demonstração deixa de existir** no Chat (hoje ela mostra "Orquestrador" com texto de exemplo). Dados de demonstração só em telas ainda não integradas, sempre com o selo DEMO.
- Mensagens e tarefas mostram quem originou (orquestrador ou usuário) e permitem abrir o evento bruto da execução.

## 9. Decisões consolidadas em 2026-11-17

Registradas em `DECISIONS.md` (D-026 a D-031): priorizar plan usage oficial se elegível, sem API paga sem autorização; arquitetura e dois modos; ferramentas e política no Rust; Git preservado; SSH via `flowsharp` como etapa remota seguinte, sem autorização de acesso/alteração; ordem 3A → 3B → 2B → backlog posterior.

## 10. Riscos
- Elegibilidade e disponibilidade de ChatGPT Plan Usage precisam de confirmação atual; os termos da OpenAI não foram lidos na pesquisa anterior.
- Preview e protocolo de SIWC podem mudar.
- O orquestrador amplia a superfície de prompt injection: tudo vindo de motores e repositórios é não confiável, e o catálogo de ferramentas é mínimo e tipado.
- Custo: mitigado por tetos duros e ausência de chamadas por padrão.
- A coexistência dos modos pode confundir o usuário: mitigada por seletor explícito e rótulos permanentes.

## 11. Elegibilidade do Sign in with ChatGPT / Plan Usage

A pesquisa oficial registrada em `DECISIONS.md` D-026 (2026-11-17) descreveu suporte a projetos pessoais locais e registro dinâmico, mas não substitui uma verificação atual do fluxo, dos termos nem da elegibilidade da conta. A disponibilidade pode mudar por ser preview; a resposta definitiva depende do fluxo oficial para este app e usuário.

Na 3B, antes de implementar: reconfirmar documentação e termos oficiais; testar apenas o caminho de autenticação autorizado; registrar qualquer erro de elegibilidade sem repetir tentativas indevidas. Se elegibilidade for negada ou incerta, parar a integração e apresentar opções. **Não** publicar ou alterar visibilidade do repositório por inferência. API paga requer decisão e autorização independentes.

## 12. Requisitos transversais (valem para todas as etapas; sem repetir os detalhes)

- **Continuidade de contexto:** o contexto da tarefa é do OneAgent (OneContext), independente de motor, sessão do motor e conversa do ChatGPT; nunca se presume acesso às memórias ou conversas privadas do ChatGPT. Troca de motor ou de Gateway reaproveita o OneContext (`CONTEXT.md`).
- **Economia de tokens:** o orquestrador só recebe pacotes de contexto limitados, resultados truncados e resumos; ferramentas determinísticas antes do LLM; teto por turno, conversa e mês; sem agentes conversando em laço (`TOKEN-OPTIMIZATION.md`).
- **Isolamento entre clientes:** cada Gateway tem ambiente, credenciais, repositórios e logs próprios; o Mac guarda só metadados e referências; nada é compartilhado entre Gateways (`GATEWAY.md`).
- **Git independente por Gateway:** cada Gateway mantém seus próprios repositórios, configuração Git e credenciais Git; o OneHub não clona repositórios de clientes no Mac e não propaga credenciais Git entre Gateways. Operações Git que alteram estado (commit, push, merge) são ações de aprovação humana.
