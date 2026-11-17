# Isolamento de leitura da execução — diagnóstico e recomendação

> Fase 2A · 2026-11-17 · Codex CLI 0.169.0 · macOS 26.5.2 (arm64, SIP ativo).
> **[M]** medido nesta máquina · **[D]** documentação oficial lida · **[T]** fonte de terceiros · **[N]** não verificado.

## 1. Diagnóstico

O sandbox `read-only` do Codex **não restringe leitura**. **[M]** Com `codex sandbox cat <arquivo fora do workspace>` o conteúdo de um canário em `/tmp` foi lido; só a escrita foi bloqueada (`Operation not permitted`). Em execução real o agente leu `~/.codex/skills/…` fora do workspace. A documentação oficial não afirma o contrário: descreve `read-only` como "can inspect files" sem delimitar onde, e só trata de leitura restrita em **perfis de permissão** **[D]**. O próprio exemplo oficial "File access limited to workspace" precisa negar `:root` explicitamente.

Consequência: `-s read-only` garante "não escreve", **não** "só lê o workspace". Um repositório hostil pode induzir o agente a ler `~/.ssh`, `~/.codex/auth.json` ou outros projetos, e o conteúdo voltaria na resposta e no histórico.

## 2. Alternativas avaliadas

| # | Alternativa | O que foi medido ou lido | Veredito |
|---|---|---|---|
| A | `-s read-only` (padrão anterior) | **[M]** lê qualquer arquivo do usuário; escrita bloqueada | Só para desenvolvimento local confiável |
| B | **Perfil de permissão do Codex** (`default_permissions` + `[permissions.x.filesystem]`) | **[M]** com `:root="deny"`, `:minimal="read"`, `:tmpdir`/`:slash_tmp="deny"` e `:workspace_roots={"."="read"}`: workspace legível; canário em `/tmp`, canário no home, listagem do home, `~/.ssh`, `~/.codex/auth.json` e outro projeto **negados pelo SO**; escrita no workspace negada; rede negada (`curl` → 000). Funciona em `codex exec` sem `-s`. **[D]** recurso **Beta**; `-s`/`sandbox_mode` anulam o perfil; se o Codex não consegue impor a política, recusa rodar | **Adotado como padrão** |
| C | Seatbelt próprio (`sandbox-exec`) envolvendo o Codex | **[M]** um perfil próprio confina leitura de ferramentas simples. Mas o Codex dentro dele **perde a capacidade de executar qualquer comando** (`sandbox_apply: Operation not permitted`, inclusive para o arquivo permitido): a sandbox interna do Codex não se aplica dentro da externa. O único caminho seria desligar a sandbox interna (`danger-full-access`) e confiar só na externa: **[N]** (o teste foi bloqueado pelo classificador de permissões e não foi repetido) e contraria a regra de não usar modos irrestritos. **[T]** `sandbox-exec` está marcado como obsoleto, sem remoção anunciada e sem substituto oficial para CLIs | Rejeitada (improvisada e incompatível) |
| D | App Sandbox (entitlements) | **[T]** exige bundle com entitlements e não confina CLIs arbitrárias como processos filhos de forma útil | Não aplicável ao Codex |
| E | Usuário macOS dedicado | Isolamento pelas permissões Unix: o agente não lê `~` do usuário principal. Exige senha de administrador para criar a conta, logins separados das CLIs e gestão de permissões dos repositórios. **[N]** não testado (precisa de admin) | Recomendado para projetos de terceiros hostis, como camada extra |
| F | VM ou contêiner (Containerization/`container`, Tart, Lima) | **[T]** Containerization exige Apple silicon e macOS 26 (esta máquina atende). Isolamento forte de arquivos e rede; custo de RAM (16 GB), pastas compartilhadas e logins dentro do convidado. **[N]** não testado: a regra permanente do usuário proíbe instalar runtime de contêiner neste Mac | Caminho natural para VPS; fora do escopo local |
| G | Instruir o modelo a não ler fora | Não é controle. Observado: o agente já recusa por instrução, mas isso não é garantia | Insuficiente sozinho |

## 3. Recomendação por modo

### Modo 1 — desenvolvimento local confiável (seus próprios repositórios, sua máquina)
- Padrão: **perfil B** (leitura só no workspace). Ele já protege contra o caso mais provável: erro do agente ou texto malicioso dentro de uma dependência.
- Custo medido **[M]**: dentro do agente `git` (shim do Xcode CLT) e `node` (leitura de `/System/Library/OpenSSL`) falham, e ferramentas fora de `/usr/bin`, `/bin` e do workspace não funcionam. Para quem precisa delas, existe `Confinement::TrustedLocalRead` (`-s read-only`), que **não** está exposto na interface: habilitá-lo é decisão sua e assume que você aceita que o agente leia o que o seu usuário lê.
- Garantias verificáveis: escrita negada, leitura fora do workspace negada pelo SO, rede de comandos negada, aprovação e identidade do executável, sem MCP/plugins/hooks do usuário.

### Modo 2 — projetos de terceiros (código que você não controla)
- **Obrigatório** o perfil B, sem exceção para `TrustedLocalRead`, e recomendado adicionar **E (usuário dedicado)** ou **F (VM)** quando o projeto for hostil ou sensível.
- Não confiar em configuração do projeto: o Codex só carrega config de projeto confiável **[D]**; o OneAgent já passa `--ignore-user-config`.
- Lembrar que o **conteúdo do workspace é enviado ao provedor do modelo** por desenho (é o que o agente lê); isso não é vazamento pelo sandbox e não pode ser impedido por ele.

### Modo 3 — Gateways em VPS de clientes (futuro)
- Não herdar o desenho do macOS (Seatbelt). Em Linux, usar **usuário sem privilégios por cliente** com diretórios `0700` e/ou **contêiner ou VM por Gateway**; a documentação do Codex orienta que, dentro de contêiner, o isolamento seja do contêiner **[D]**.
- Antes de ativar: provar com canários, como nos testes desta fase, que um cliente não lê arquivos de outro, e que o Gateway não tem credenciais de outros clientes.
- **[N]** nada disto foi testado (sem acesso a VPS).

## 4. O que se pode e o que não se pode garantir

| Garantia | Verificável? | Evidência |
|---|---|---|
| O agente não escreve no workspace | **Sim** | Teste real: snapshot idêntico; `codex sandbox` nega escrita |
| Comandos do agente não leem fora do workspace (home, `/tmp`, outros projetos, `auth.json`, `~/.ssh`) | **Sim, por canário, no nível do SO** | `os_level_profile_denies_reads_outside_the_workspace` (sem custo de modelo) |
| Idem para `~/.ssh`, outro projeto e rede dos comandos | **Sim, na mesma suíte** | Mesmo teste (coberto desde a revisão final) |
| O comando do Codex enviado ao SO usa o perfil e não o `-s` legado | **Sim** | Testes de argumentos e do pipeline |
| Comandos do agente sem rede | **Parcial**: medido com `curl`; o tráfego do próprio Codex para o modelo é separado e permitido | `codex sandbox` |
| O processo do Codex em si (cliente da API, lê o próprio `auth.json`) está confinado | **Não**: só os comandos que ele dispara são sandboxed | Desenho do Codex |
| Dentro de `codex exec`, o SO nega leitura quando o agente **tenta** | **Não observado**: o agente recusa antes de tentar. A negação está provada por `codex sandbox` com o mesmo perfil | Experimentos desta fase |
| O perfil continua válido em versões futuras do Codex | **Não**: é Beta e pode mudar; a versão mínima não está documentada (funciona na 0.169.0) | Fixar versão e rodar o teste de SO a cada atualização |
| Ausência de escapes do Seatbelt ou do kernel | **Não** | Fora do alcance de testes |
| Prompt injection não influencia o agente | **Não**: só limita o dano (sem escrita, sem leitura externa, sem rede para comandos) | — |

**Como reexecutar as provas** (os testes são `#[ignore]` por usarem o Codex real e não rodam em `pnpm check`):
`cargo test -p one-exec --test real_codex -- --ignored --nocapture --test-threads=1`.
`os_level_profile_denies_reads_outside_the_workspace` **não gasta cota de modelo** (usa `codex sandbox`); `real_codex_read_only_investigation_in_a_temp_workspace` e `real_codex_cannot_read_outside_the_workspace` gastam cota (≈ 45 a 67 mil tokens de entrada, majoritariamente em cache). Medições feitas só à mão nesta fase, sem teste automatizado: Seatbelt externo quebrando o Codex (`sandbox_apply`), e a confusão inicial do canário em `/tmp`.

## 5. Pendências

1. Decisão sua: expor ou não `TrustedLocalRead` e, se sim, por workspace e com aviso.
2. Fixar a versão do Codex aprovada junto com a identidade e rodar `os_level_…` após cada atualização (já faz parte da identidade: um Codex atualizado exige nova aprovação).
3. Se a Fase 2B trouxer Claude e Cursor: avaliar o equivalente de cada um (permissões do Claude Code; `--sandbox` e `--mode ask|plan` do Cursor) com o mesmo método de canário. **[N]** nenhum foi medido.
4. Modo 3 depende de acesso a um ambiente de teste Linux, que não existe nesta fase.

## 6. Decisões de encerramento da Fase 2A (2026-11-17)

1. **O perfil restrito de leitura é o padrão e o único modo da interface.**
2. **`TrustedLocalRead` não é exposto na interface** nesta fase; permanece só na API do adaptador, sem caminho de ativação pelo usuário.
3. **Não se amplia permissão para consertar incompatibilidades.** Nenhum caminho (Xcode CLT, `/System/Library/OpenSSL`, `~`) é liberado para fazer `git` ou `node` funcionarem.
4. **Limitação registrada:** no modo restrito, `git` (shim do Xcode Command Line Tools) e `node` (leitura de `/System/Library/OpenSSL`) **falham dentro do agente**; ferramentas fora de `/usr/bin`, `/bin` e do workspace também. O agente investiga lendo e listando arquivos; não roda testes, build nem comandos Git.
5. **O perfil é Beta e não é garantia absoluta de isolamento do processo inteiro.** Ele confina os comandos que o Codex dispara. O processo do Codex (cliente da API, leitura do próprio `auth.json`) fica fora do confinamento; escapes de Seatbelt ou do kernel não são cobertos; o recurso pode mudar entre versões. Não deve ser tratado como substituto de isolamento por usuário, VM ou contêiner.
6. **Sem sandbox de Linux ou Windows** nesta fase: a verificação de ACL falha fechada nessas plataformas e nenhum equivalente do perfil foi implementado ou testado.

### Modos planejados (futuro; nada disto está implementado)

| Modo | Para quê | Isolamento previsto | Escrita |
|---|---|---|---|
| **Investigação Restrita** | Ler e entender projetos, inclusive de terceiros | **É o modo atual**: perfil de leitura só no workspace, sem rede para comandos, sem MCP/plugins/hooks do usuário, aprovação e identidade do executável | Negada |
| **Desenvolvimento Isolado** | Alterar código, rodar testes e build, usar `git` e `node` | Ambiente separado do restante do usuário: usuário macOS dedicado ou VM/contêiner, com cópia/worktree do projeto; ferramentas liberadas **dentro** desse ambiente, nunca ampliando o perfil restrito no Mac principal | Em worktree isolada, com aprovação por tarefa |
| **Administração Autorizada** | Operações com efeito externo (instalar, migrar, publicar, produção) | Aprovação humana explícita por ação (níveis 2 e 3 de `ORCHESTRATOR.md` §10), Gateway dedicado, registro de auditoria | Somente o autorizado, uma ação por vez |

Cada modo terá documento, testes de canário e fase próprios antes de ser habilitado. A ordem de entrega não está decidida.
