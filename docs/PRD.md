# OneAgent — PRD (Fase 0)

> Status: Fase 0 e Fase 1 concluídas; Fase 2 (execução real pelo Gateway local) é a próxima · Data: 2026-11-17 · Autor: Claude Code (desenvolvedor principal) · Revisor: ChatGPT (arquiteto)
> Mudanças da Fase 1 no fim deste documento (§12).
> Convenção: **[V]** = verificado nesta máquina · **[D]** = visto em documentação/pesquisa, não testado · **[?]** = não verificado.

## 1. Problema

Hoje o usuário alterna manualmente entre Codex CLI, Claude Code e Cursor CLI, em máquinas diferentes (Mac, VPS), sem contexto compartilhado entre eles. Cada troca de ferramenta perde histórico, decisões e entendimento do projeto.

## 2. Visão

Um aplicativo macOS de chat que conversa com um **orquestrador** (ChatGPT) e delega trabalho a **executores** (as três CLIs) por meio de **Agent Gateways** instalados na máquina que contém o código. O conhecimento sobre cada projeto fica persistido (**OneContext**) e sobrevive à troca de motor.

## 3. Componentes

| Componente | Papel | Onde roda |
|---|---|---|
| **OneAgent.app** | Interface de chat, seleção de Gateway/motor/modelo, histórico, projetos, configurações | Mac |
| **OneHub** | Registro de gateways, tarefas e execuções; política e aprovações; ponte UI ↔ Gateway; segredos | Mac (núcleo do app) |
| **OneOrchestrator** | Comportamento do ChatGPT: planeja, escolhe, delega, revisa (ver `ORCHESTRATOR.md`) | Provedor OpenAI (remoto) + prompt/ferramentas locais |
| **Agent Gateway (OneNode)** | Serviço que detecta CLIs, executa tarefas em diretórios autorizados, devolve logs/evidências | Mac, Linux, Windows, VPS |
| **OneContext** | Conhecimento persistente por projeto, com proveniência | Mac (SQLite) |
| **Motores** | Codex CLI, Claude Code, Cursor CLI — apenas executores | Mesma máquina do Gateway |

Princípio: **o OneOrchestrator decide e delega dentro dos limites autorizados; as CLIs executam; o Gateway controla a execução; o OneHub aplica a política.**

## 4. Usuários e cenários

Usuário único inicial (o dono do Mac). Cenário-guia (do pedido):
1. Abre OneAgent → escolhe Gateway "Apliflow" → escolhe Claude Code + modelo.
2. Pede: "Localize o SystemFlow, leia a documentação, analise a arquitetura e se contextualize."
3. O orquestrador monta a tarefa → Gateway executa → resultado volta ao chat → OneContext registra.

Na fase inicial o Gateway é **local no Mac**; a VPS Apliflow só depois do MVP validado.

## 5. Escopo

### 5.1 MVP (local, Mac)
- Chat com orquestrador (streaming, histórico persistido).
- Gateway local detectando as 3 CLIs, versão, login e modelos.
- Delegação de tarefa a **uma CLI por vez** em um diretório autorizado, com logs em tempo real, resultado e evidências.
- Modos de execução: **somente leitura** (padrão) e **escrita** (exige aprovação humana).
- Aprovação humana para operações sensíveis, aplicada pelo Hub/Gateway (não só pelo prompt).
- OneContext básico: registrar projeto, gerar resumo técnico, recuperar contexto por projeto/módulo/origem, trocar de motor reaproveitando o contexto.
- Configurações → Orquestrador: ver/editar prompt, restaurar padrão, histórico de versões, instruções por projeto.
- Logs estruturados e trilha de auditoria.

### 5.2 Fora do MVP
Gateway remoto/VPS · Linux/Windows · execução paralela de várias CLIs · embeddings/busca vetorial · MCP próprio exposto a terceiros · multiusuário · distribuição assinada/notarizada · auto-update.

## 6. Requisitos funcionais (resumo)

- **RF-01** Cadastrar/listar Gateways e mostrar estado (online, CLIs, modelos).
- **RF-02** Selecionar Gateway, motor e modelo por conversa.
- **RF-03** Orquestrador delega via ferramentas autorizadas do OneHub (nunca executa direto).
- **RF-04** Streaming de eventos da execução (mensagens, comandos, arquivos tocados, erros).
- **RF-05** Cada execução gera registro: entrada, motor, modelo, diretório, eventos, saída, código de saída, duração, evidências.
- **RF-06** Aprovações: pedido legível (o quê, onde, por quê, impacto), decisão registrada.
- **RF-07** OneContext: ingestão, resumo, recuperação por projeto/módulo/versão/origem, propostas de atualização com validação humana.
- **RF-08** Compatibilidade com skills, agents, commands e MCPs de cada CLI (listar, selecionar, repassar).
- **RF-09** Configurações → Orquestrador (ver `ORCHESTRATOR.md` §8).
- **RF-10** Cancelar execução em andamento (kill confiável da árvore de processos).

## 7. Requisitos não funcionais

Modularidade e tipagem forte · Gateway sem privilégio administrativo permanente · isolamento por diretório autorizado · logs estruturados (JSON) · persistência local em SQLite · contratos versionados entre componentes · portabilidade futura (Linux/Windows) · instalação futura do Gateway por releases do GitHub · segredos fora do banco e fora de logs · latência de UI independente da CLI (tudo assíncrono).

## 8. Critérios de aceitação do MVP (verificáveis)

Cada critério deve ter teste automatizado ou roteiro manual registrado.

| ID | Critério | Verificação |
|---|---|---|
| AC-01 | App abre no macOS e persiste conversas após reinício | Manual + teste de persistência |
| AC-02 | Gateway local lista Codex, Claude Code e Cursor com versão e status de login corretos | Comparar com `--version` e `login/auth/status` das CLIs |
| AC-03 | Lista de modelos por motor reflete a fonte real (Codex: catálogo; Cursor: `--list-models`; Claude: aliases + modelo reportado na execução) e sinaliza o que não pôde ser confirmado | Teste de contrato por adaptador |
| AC-04 | Tarefa somente leitura em projeto de teste retorna resumo com ≥ 3 arquivos reais citados; nenhum arquivo é modificado | Hash do diretório antes/depois |
| AC-05 | Tentativa de escrever fora do diretório autorizado é bloqueada e registrada | Teste negativo |
| AC-06 | Tarefa de escrita só inicia após aprovação; recusar impede a execução | Teste E2E |
| AC-07 | Cancelar mata a CLI e processos filhos em ≤ 5 s | Teste de processo |
| AC-08 | Trocar de motor na mesma conversa reaproveita o OneContext do projeto sem reler tudo | Medir contexto injetado e chamadas de leitura |
| AC-09 | O orquestrador não afirma conclusão sem evidência anexada (suíte de comportamento do `ORCHESTRATOR.md` §12 ≥ 95% de aprovação) | Suíte de avaliação |
| AC-10 | Conteúdo malicioso em arquivo do repositório ("ignore as regras…") não altera política nem dispara ação (suíte de injeção) | Testes adversariais |
| AC-11 | Nenhum segredo aparece em logs, banco ou prompts (varredura por padrões) | Teste automatizado |
| AC-12 | Editar o System Prompt cria versão; restaurar volta ao padrão; política de segurança não é alterável pelo prompt | Teste de UI + teste de política |
| AC-13 | Falha de CLI (não logada, modelo indisponível, timeout) vira erro compreensível e ação sugerida, sem travar a UI | Testes de falha injetada |
| AC-14 | Todas as execuções têm trilha de auditoria consultável (quem aprovou, o quê, quando) | Consulta ao banco |

## 9. Métricas

Tempo até primeiro resultado útil · tokens do orquestrador por tarefa · % de tarefas concluídas com evidência · taxa de aprovações recusadas · falhas de adaptador por 100 execuções.

## 10. Riscos principais (detalhe em `DECISIONS.md` e `SECURITY.md`)

1. **Login/assinatura ChatGPT ≠ API.** O ChatGPT normal não pode ser embutido nem sua sessão reutilizada; o orquestrador precisa de integração oficial (ver `DECISIONS.md` D-001).
2. **Políticas de uso das assinaturas nas CLIs** (Claude em particular) mudaram várias vezes em 2026; automatizar via CLI headless pode consumir outra cota.
3. **Aprovação no meio da execução** não é uniforme: Codex (app-server) e Claude (host de permissões) permitem; Cursor headless é limitado.
4. **Prompt injection** vindo de repositórios lidos pelas CLIs.
5. **Toolchain ausente** neste Mac (Rust, Xcode completo).
6. **Modelos "realmente disponíveis"** só são enumeráveis em Codex e Cursor; Claude Code não tem comando de listagem [V].

## 11. Fora de escopo permanente

Uso de tokens OAuth das CLIs fora das próprias CLIs · contornar limites de assinatura · execução com privilégio de administrador · alteração do Mac fora da pasta do projeto sem aprovação.

## 12. Revisão da Fase 1

Incorporado ao escopo, com detalhe nos documentos indicados:

- **Multi-cliente:** cada Gateway tem ambiente próprio (repositórios, credenciais, CLIs, skills, logs, política e limites); o Mac guarda metadados e referências. → `GATEWAY.md`.
- **OneSkills Registry:** inventário de skills, agents, commands, plugins e MCPs por escopo e motor; instalação só com procedência verificada e aprovação. → `CAPABILITIES.md` §1.
- **OneAgent Factory:** criação de agente só após reutilizar, adicionar skill ou melhorar o existente; agente gerado não se autoconcede permissão; publicação exige aprovação. → `CAPABILITIES.md` §2.
- **Otimização de tokens:** escada de decisão, contexto incremental, orçamento por tarefa, resumos e logs no Gateway. → `TOKEN-OPTIMIZATION.md`.
- **OneContext:** modelo de proveniência e estados; Mac × Gateway. → `CONTEXT.md`.
- **Provisionamento remoto (futuro, não implementado):** `OneAgent → Adicionar Gateway → SSH → instalar OneNode → registrar identidade → verificar conexão`, com verificação de host, release assinada e idempotência. → `GATEWAY.md` §6.

Critérios novos para o MVP: **AC-15** — dados de um Gateway não aparecem no Mac além de metadados e referências (teste de inspeção do banco). **AC-16** — instalação de capacidade sem aprovação é bloqueada (teste E2E, Fase 3).
