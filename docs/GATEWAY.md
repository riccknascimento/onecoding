# Agent Gateway (OneNode) — ambiente independente e provisionamento

> Fase 1 · 2026-11-17. Protocolo e operações: `ARCHITECTURE.md` §5. Segurança: `SECURITY.md`. Aqui: o que é de quem, onde cada dado mora e como um Gateway nasce.
> Estado: **nada disto está implementado** além da detecção local somente leitura (`crates/one-adapters`).

## 1. Princípio: um Gateway = um ambiente completo e isolado

O OneAgent.app (Mac) é só o cliente/painel. Cada cliente/servidor tem **seu** Gateway, e o Gateway é dono de tudo o que toca o código daquele ambiente. O Mac não vira depósito dos clientes.

| Pertence ao **Gateway** (na máquina do cliente) | Pertence ao **Mac** (OneAgent.app) |
|---|---|
| Repositórios, clones, configuração e credenciais Git | Registro do Gateway: nome, identidade pública (fingerprint), endereço, política |
| Codex CLI, Claude Code, Cursor CLI e **seus logins** | Conversas com o orquestrador e histórico de tarefas (resumos) |
| Modelos disponíveis por CLI (detectados lá) | Metadados de projetos (nome, caminho **no Gateway**) |
| Skills, agents, commands, plugins e MCPs instalados | OneContext: metadados, documentação consolidada, resumos, **referências** a evidências remotas |
| Projetos e workspaces autorizados | Credencial do orquestrador (Keychain do Mac) |
| Logs completos, artefatos, diffs, resultados | Chave privada do Hub para falar com cada Gateway (Keychain; uma por Gateway) |
| Política local (diretórios, motores, modos, MCPs) e limites de recursos | Política "central" por Gateway (o Gateway reaplica e pode restringir mais, nunca menos) |

Regras:
1. **Sem clones, dependências ou logs completos de clientes no Mac.** O Mac guarda só o que o OneContext precisa (§4) e o resumo de cada execução.
2. **Credenciais nunca sincronizam** entre Gateways nem entre Gateway e Mac: cada Gateway tem seus próprios logins das CLIs, chaves Git/SSH e tokens. O Hub não lê nem copia `~/.codex`, `~/.claude`, `~/.cursor`, chaves SSH ou `.env`.
3. **Nenhum Gateway enxerga outro.** Não há canal Gateway↔Gateway; o Hub é o único ponto de contato e não repassa dados de um cliente a outro (o OneContext é segregado por Gateway/projeto).
4. O Gateway **não tem privilégio administrativo permanente**: roda como usuário comum, em diretório do usuário.

## 2. Layout no Gateway (proposta)

```
~/.oneagent/                      (ou $XDG_DATA_HOME/oneagent)
├── bin/one-node                  binário verificado (releases)
├── config/gateway.toml           política local, limites, workspaces autorizados
├── identity/                     chave do Gateway (0600), fingerprint
├── state/gateway.db              SQLite local (tarefas em andamento, índice de evidências)
├── runs/<run_id>/                log completo, eventos brutos, diffs, artefatos (retenção configurável)
├── worktrees/<run_id>/           worktrees isolados de execuções em modo write
└── inventory/                    inventário de capacidades (ver CAPABILITIES.md)
```

Permissões 0700 no diretório, 0600 nos segredos. Serviço de usuário (launchd/`systemd --user`), sem `sudo`.

## 3. Política e limites de recursos (por Gateway)

Definidos no Gateway (`gateway.toml`) e espelhados no Hub; vale o **mais restritivo**.

| Item | Padrão proposto |
|---|---|
| Workspaces autorizados | Nenhum até o usuário autorizar (caminho canônico, sem symlink para fora) |
| Motores/modelos permitidos | Lista explícita |
| Modo padrão / escrita | `read_only`; escrita só em worktree isolada e com aprovação |
| Execuções simultâneas | 1 (MVP); configurável |
| Timeout por tarefa / sem progresso | 30 min / 5 min |
| Memória / CPU por tarefa | Limite por árvore de processos (cgroups no Linux; limite e monitor no macOS) |
| Tamanho de saída | Truncar eventos no Hub; íntegra fica em `runs/` |
| Disco de `runs/` | Cota + retenção (ex.: 14 dias, mantendo o que foi referenciado pelo OneContext) |
| Rede | Negada/restrita quando a sandbox da CLI permitir; destinos novos exigem aprovação |
| Variáveis de ambiente do filho | Allowlist (padrão do `one-adapters::SystemHost`) |

## 4. O que fica no Mac × evidências remotas

Cada `Run` no Mac guarda: resumo estruturado, status, código de saída, duração, **`evidence_refs`** (ex.: `gw://apliflow/runs/<id>/log`), hashes de arquivos citados e a lista de arquivos tocados. Quando o usuário ou o orquestrador precisa da evidência, o Hub a **busca sob demanda** no Gateway (trecho, não o arquivo inteiro) e a mostra sem persistir cópia completa. O que o OneContext mantém está em `CONTEXT.md`.

## 5. Identidade e confiança

- Cada Gateway gera um par de chaves (Ed25519) na instalação; a chave pública e o **fingerprint** vão para o Hub no pareamento. O Hub tem uma chave própria **por Gateway** (revogável individualmente).
- Todas as mensagens `task.*` vêm assinadas pelo Hub e com `expires_at`; o Gateway rejeita expiradas/repetidas (nonce).
- O Gateway só aceita conexão do Hub pareado. Sem porta pública (ver §6).

## 6. Provisionamento remoto — "Adicionar Gateway" (futuro, Fase 8; não implementar antes)

Fluxo: **OneAgent → Adicionar Gateway → SSH → instalar OneNode → registrar identidade → verificar conexão.**

1. **Entrada do usuário:** nome, host, porta, usuário SSH. Autenticação por **chave** (agente SSH/chave dedicada); senha só em último caso e nunca guardada. O OneAgent não pede nem armazena a senha de root; instalação **sem sudo**.
2. **Identidade do host:** primeira conexão exibe o fingerprint da chave do host para o usuário confirmar (modelo `known_hosts` próprio do app, `StrictHostKeyChecking` equivalente). Mudança de chave = bloqueio e alerta, nunca aceitação silenciosa.
3. **Pré-verificações (somente leitura):** SO/arquitetura, versão de glibc/macOS, espaço em disco, usuário não-root, `systemd --user`/launchd disponível, portas.
4. **Obtenção da release:** o **Mac** baixa a release do GitHub (tag fixada) e verifica **checksum e assinatura** (chave pública de release embutida no app) antes de enviar por SSH (SCP/SFTP) ao host. O servidor do cliente não executa `curl | sh`.
5. **Instalação idempotente:** diretório versionado `~/.oneagent/releases/<versão>/`, link `current`, serviço de usuário. Reexecutar não duplica: compara versão/hash e só age no que difere; falha no meio faz rollback para o `current` anterior.
6. **Registro de identidade:** Gateway gera chaves (§5); o Hub recebe a pública, confirma o fingerprint **por canal já autenticado (a sessão SSH)** e emite o par do Hub. Código de pareamento de uso único como segunda verificação.
7. **Verificação:** `gateway.hello` (versão, SO, protocolo) + `gateway.capabilities` (CLIs, modelos, inventário). O Gateway aparece como `online` só após handshake assinado.
8. **Transporte em operação:** túnel SSH para loopback do Gateway (ou conexão de saída do Gateway) + mTLS; sem porta pública (D-007).
9. **Atualização e remoção:** mesma rota (release verificada, troca atômica de `current`); desinstalar remove serviço, binários e, se o usuário pedir, `runs/` e identidade.
10. **Auditoria:** cada passo do provisionamento vira evento de auditoria; nada é instalado sem confirmação explícita do usuário no resumo do plano ("vou instalar X em Y, criar o serviço Z").

Critérios de aceitação (futuros): host com chave diferente bloqueia; release com checksum errado não é enviada; reexecução não altera nada; queda no meio restaura o estado anterior; nenhum segredo do cliente chega ao Mac.

## 7. Detecção local (o que existe hoje)

`one-adapters` consulta, sem shell e com timeout, `--version`, status de login e listagem de modelos de `codex`, `claude` e `cursor-agent`, procurando os binários em `~/.local/bin`, `~/.nvm/versions/node/*/bin`, Homebrew e `PATH` (app aberto pelo Finder não herda o PATH do terminal). Só as CLIs oficiais são executadas; e-mail/organização retornados pelas CLIs **não** são lidos nem guardados. Modelos: Codex pelo catálogo (`debug models`, só visibilidade `list`), Cursor por `--list-models`, Claude por apelidos **não confirmados** (D-013).
