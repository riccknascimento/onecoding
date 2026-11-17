# OneSkills Registry e OneAgent Factory

> Fase 1 · 2026-11-17 · Planejamento. **Nada implementado.** O catálogo mostrado na UI hoje é demonstração (`demo: true`). Contrato: `Capability` em `crates/one-protocol`.

## 1. OneSkills Registry — catálogo de capacidades

Capacidade = skill, agent, command, plugin ou servidor MCP. O Registry é o **inventário por Gateway**, mantido pelo próprio Gateway (é ele quem vê o disco) e consultado pelo Hub/orquestrador.

### 1.1 Responsabilidades
1. **Descobrir** o que está instalado em cada escopo e CLI (leitura somente).
2. **Verificar** versão, origem, integridade (hash do conteúdo) e compatibilidade com cada motor.
3. **Instalar, atualizar e remover** com procedência verificada e autorização (§1.5).
4. **Escopos:** `global` (usuário do Gateway) · `gateway` (configuração do OneNode) · `project` (dentro do repositório) · `cli` (específico de uma CLI). Precedência: `project` > `gateway` > `global`; um item só vale para os motores compatíveis.
5. **Carregar sob demanda:** o orquestrador recebe só um índice compacto; o corpo de uma skill entra no contexto apenas quando selecionada (`TOKEN-OPTIMIZATION.md` §3).
6. **Não assumir compatibilidade universal:** cada item tem uma matriz de compatibilidade por motor, derivada de verificação, nunca presumida.

### 1.2 Formatos nativos (observados nesta máquina; confirmar por versão na Fase 2)

| Tipo | Claude Code | Codex CLI | Cursor CLI |
|---|---|---|---|
| Skill | `~/.claude/skills/<nome>/SKILL.md` (frontmatter `name`, `description`) | `~/.codex/skills/<nome>/SKILL.md` | `~/.cursor/skills/<nome>/SKILL.md` |
| Agent | `~/.claude/agents/*.md` ou `--agents <json>` | `~/.codex/agents/*.toml` | `~/.cursor/agents/` |
| Command | `~/.claude/commands/*.md` | formato próprio (`codex plugin`/prompts) [?] | `.cursor/commands` [?] |
| Plugin | `~/.claude/plugins` (marketplaces) | `codex plugin` | `--plugin-dir` |
| MCP | `--mcp-config`, `.mcp.json` | `codex mcp` + `config.toml` | `.cursor/mcp.json`, `--approve-mcps` |

Observação: nesta máquina, as skills dos três ambientes compartilham o mesmo `SKILL.md` por links criados por script do usuário; isso é **arranjo local**, não garantia de compatibilidade. Agents e commands **não** são intercambiáveis sem conversão.

### 1.3 Adaptadores por CLI
`CapabilityAdapter { discover(scope) → [Capability], validate(item) → Compat, install(src) , update(id), remove(id) }`, um por motor, dentro de `one-adapters`. Eles traduzem o formato nativo para `Capability` e o inverso. Conversão entre formatos é uma **proposta** (gera arquivo novo para o outro motor com aviso de perda), nunca automática.

### 1.4 Modelo de dados (Gateway, espelhado resumido no Hub)
`capability(id, kind, name, scope, engine, version, source{type,url,ref,commit}, content_hash, installed_at, compat{codex,claude,cursor: ok|partial|no|unknown}, enabled, trust{unverified|verified|pinned})` + `capability_event(...)` (instalou/atualizou/removeu/desativou, quem aprovou). O índice enviado ao orquestrador contém apenas `{id, kind, name, 1 linha de descrição, engine, scope}`.

### 1.5 Instalação, atualização e remoção — procedência e autorização
- **Fontes permitidas** (lista por Gateway): repositório local, repositório Git com commit **fixado por SHA**, marketplace oficial de cada CLI, pacote assinado. Fonte nova exige aprovação do usuário uma vez.
- **Verificações antes de instalar:** origem na lista permitida · SHA/hash fixado · assinatura/checksum quando a fonte oferece · varredura estática do conteúdo (comandos de shell, chamadas de rede, leitura de segredos, instruções suspeitas de injeção) · diff contra a versão instalada em atualizações · compatibilidade com o motor alvo.
- **Autorização:** instalar/atualizar/remover é **ação nível 2** (aprovação por ação, com o resultado da varredura e do diff exibidos). Itens que pedem MCP externo, shell ou rede sobem para nível 3 conforme a política. O orquestrador **pode propor, nunca instalar sozinho**.
- **Isolamento:** instalação em área do Gateway do escopo correto; nada é instalado fora dos diretórios previstos; sem execução de scripts de instalação sem aprovação; atualização automática desligada por padrão (apenas verificação de novas versões).
- **Remoção/rollback:** versão anterior mantida; remover apaga arquivos e referências e registra o evento.
- **Fase 1–2:** só **descoberta somente leitura**. Instalar/atualizar/remover entram em fase própria, após o modelo de aprovação (Fase 3).

### 1.6 Critérios de aceitação (futuros)
Descobre skills/agents/MCPs de cada CLI sem alterá-los · reporta versão/origem/hash e compatibilidade com status `unknown` quando não verificada · item de fonte não permitida é recusado · instalação sem aprovação não ocorre · remoção reversível · índice enviado ao orquestrador ≤ 60 tokens por item · carregar uma skill só ocorre quando selecionada.

## 2. OneAgent Factory — especialização automática

Mecanismo pelo qual o orquestrador adapta o sistema a uma demanda **sem criar agentes à toa**.

### 2.1 Ordem obrigatória de decisão
1. **Reutilizar agente existente** que cubra a demanda (casando descrição, escopo e motor).
2. **Adicionar skill compatível** a um agente/execução existente (carregamento sob demanda).
3. **Propor melhoria** de agente existente (diff do prompt/ferramentas, com justificativa e teste de validação).
4. **Criar novo agente** apenas com necessidade justificada: as opções 1–3 foram avaliadas e falharam, e a demanda é recorrente ou específica o bastante.

O orquestrador registra qual nível usou e por quê; a ordem é verificada nos testes de comportamento (OR-19).

### 2.2 Especificação de um agente gerado
`id` · `name` · `descrição` (1 linha) · `versão` (semver; mudança = nova versão) · `escopo` (global/gateway/projeto/CLI) · `motores compatíveis` · `ferramentas e capacidades autorizadas` (lista explícita e mínima) · `modo máximo` (`read_only`/`write`) · `limites` (tempo, tokens, tentativas) · `critérios de validação` (casos de teste e resultado esperado) · `origem` (qual demanda o motivou, quem aprovou) · `estado` (`draft` → `validated` → `published`).

### 2.3 Regras de segurança (invioláveis)
- **Nenhum agente gerado concede permissão a si mesmo.** O arquivo do agente só **declara** ferramentas pedidas; quem concede é a política do Gateway/Hub, decidida pelo usuário. Pedido acima da política vigente é descartado e registrado, nunca elevado.
- Permissões de um agente gerado ≤ permissões de quem o propôs e ≤ política do escopo de destino. Agente não pode criar agente, editar política, instalar plugin ou alterar a própria definição.
- **Criação e publicação persistente exigem aprovação** (nível 2): mostrar a especificação completa e o diff. Até a aprovação, o agente existe apenas como `draft` descartável (em memória/área temporária do run).
- Validação antes de publicar: roda os critérios de validação em modo `read_only` e anexa o resultado; sem validação aprovada não há `published`.
- Plugins ou MCPs novos criados pela Factory seguem integralmente o §1.5 (procedência + aprovação).
- Todo ciclo (proposta, aprovação, publicação, uso, remoção) entra na trilha de auditoria; é possível desfazer.

### 2.4 Fluxo
Demanda → consulta ao índice do Registry → decisão 1/2/3/4 → (se 2–4) proposta estruturada → aprovação humana → validação → publicação no escopo aprovado → uso → métricas (acerto, tokens) → possível aposentadoria de agentes não usados.

### 2.5 Critérios de aceitação (futuros)
Reutiliza agente existente quando há um adequado (≥ 95% nos casos de teste) · não cria agente sem justificativa registrada · agente criado sem aprovação nunca persiste · tentativa de agente se autoconceder permissão é bloqueada · todo agente publicado tem versão, escopo e critérios de validação aprovados.
