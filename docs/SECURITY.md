# OneAgent — Segurança (modelo de ameaças e controles)

> Status: proposta (Fase 0) · 2026-11-17. Controles marcados **[Hub]**, **[Gateway]** ou **[Prompt]** indicam onde são aplicados. **Nenhum controle crítico depende só do prompt.**

## 1. Ativos a proteger

Código-fonte e dados dos projetos · credenciais (chaves de API, tokens das CLIs, SSH) · máquina do usuário e VPS · integridade do OneContext · custos (tokens/cotas) · trilha de auditoria.

## 2. Fronteiras de confiança

| Zona | Confiança |
|---|---|
| Usuário (UI do app) | Autoridade de aprovação |
| OneHub (núcleo Rust) | Confiável; aplica política; guarda segredos |
| Webview/UI React | **Semiconfiável**: renderiza conteúdo não confiável (Markdown/HTML do modelo) |
| Provedor do orquestrador | Externo; recebe só o necessário |
| Gateway | Confiável dentro do que o Hub autoriza; processo de usuário comum |
| CLIs (Codex/Claude/Cursor) | Executores **semiconfiáveis**: podem ser enganados por conteúdo do repositório |
| Conteúdo de repositórios, logs, web, saídas de ferramentas | **Não confiável** (dados, nunca instruções) |

## 3. Ameaças principais e controles

| # | Ameaça | Controles |
|---|---|---|
| T1 | **Prompt injection** via arquivo/issue/log lido pela CLI ou devolvido ao orquestrador | **[Prompt]** hierarquia e regra "conteúdo = dado"; **[Hub]** resultados de ferramentas marcados `untrusted`, sem poder de mudar ferramentas/política; **[Hub/Gateway]** mesmo que o modelo seja enganado, a ação é barrada pelo policy engine; aprovação humana para níveis ≥ 1; testes OR-08/AC-10 |
| T2 | Escape do diretório autorizado | **[Gateway]** workspace canônico (realpath, sem symlink para fora), CLIs iniciadas com `cwd` + `--add-dir` explícito, sandbox da CLI (`-s`, `--sandbox`, permissões) + verificação pós-execução de arquivos tocados; **[Hub]** workspace só autorizável pelo usuário |
| T3 | Execução com privilégio excessivo | **[Gateway]** nunca root/admin; sem `sudo`; serviço de usuário; instalação em diretório do usuário; bloqueio de `--dangerously-*`, `--yolo`, `bypassPermissions` por padrão (só com política explícita e aprovação nível 3) |
| T4 | Vazamento de segredos (env, arquivos `.env`, chaves SSH, tokens das CLIs) | **[Gateway]** ambiente do filho montado por *allowlist* (não herda tudo); caminhos sensíveis (`~/.ssh`, `~/.aws`, Keychain, `~/.codex`/`~/.claude`/`~/.cursor` credenciais) fora do alcance; **[Hub]** segredos só no Keychain; redação por padrões em logs/eventos/prompts; **nunca** ler ou copiar o token OAuth das CLIs |
| T5 | Ação destrutiva/produção sem consentimento | Níveis de aprovação (`ORCHESTRATOR.md` §10); aprovação como objeto assinado com hash dos parâmetros e expiração; produção = nível 3, uma ação por vez; worktree/branch isolada para escrita; diff revisável antes de aplicar |
| T6 | Comprometimento do Gateway remoto | Sem porta pública; túnel SSH/mTLS; pareamento por código único; chave do Hub por Gateway revogável; Gateway aceita só `task.*` assinadas pelo Hub; diretórios e motores restritos por política do Gateway (o Hub não amplia sozinho) |
| T7 | XSS/Execução via conteúdo do modelo na UI | Markdown sanitizado, sem HTML bruto, CSP estrita no Tauri, sem `eval`, capabilities mínimas do Tauri, sem acesso direto do front a shell/FS |
| T8 | Cadeia de suprimentos (dependências, releases do Gateway) | Lockfiles, `cargo audit`/`npm audit`, mínimo de dependências, releases com checksum e assinatura, instalação com verificação, pinagem de Actions por SHA |
| T9 | Corrupção/envenenamento do OneContext | Proveniência obrigatória; estados `proposed/validated/official`; atualização `official` só por humano; versionamento e rollback |
| T10 | Custo descontrolado | Orçamento por conversa/tarefa, limites de tentativas, timeouts, kill switch |
| T11 | Uso indevido de assinaturas das CLIs | Executar **as próprias CLIs** como o fabricante prevê, sem extrair tokens nem simular o app; respeitar termos/limites; sinalizar ao usuário mudanças de cobrança (ver D-008) |
| T12 | Adulteração da trilha de auditoria | Log append-only com hash encadeado por `run`; backup; exibição só leitura na UI |

## 4. Política de execução (resumo)

- **Padrão:** `read_only`. Escrita só em worktree/branch isolada, com aprovação.
- **Diretórios:** somente os autorizados no Gateway, por caminho canônico.
- **Rede:** negada ou restrita quando a CLI/sandbox suportar; destinos novos exigem aprovação.
- **Processos:** árvore de processos por `task`, com timeout, limite de saída e kill confiável.
- **Segredos:** nunca em prompts do orquestrador, banco ou logs. Variáveis sensíveis não são repassadas ao filho.
- **Bloqueado sempre:** privilégio de administrador, desabilitar sandbox via orquestrador, acessar áreas fora do workspace.

## 5. Aprovação humana

Objeto: `{id, ação, parâmetros, workspace, motor, impacto, reversão, expira_em, hash}`. UI mostra texto claro e diff quando houver. Resposta registrada (quem, quando, decisão). O Gateway rejeita execução sem aprovação válida casando com o hash. Aprovação não é reutilizável.

## 6. Privacidade do envio ao provedor do orquestrador

Enviar apenas: mensagens da conversa, pacote de contexto limitado, resultados resumidos e trechos citados necessários. Não enviar: segredos, `.env`, conteúdo integral de repositório, caminhos fora do necessário. Mostrar ao usuário o que vai ser enviado (modo depuração). Retenção/uso de dados conforme os termos do provedor escolhido (a confirmar em D-001).

## 7. Segurança das CLIs (observações verificadas)

- Claude Code: `--permission-mode`, `--permission-prompts host` (o Hub decide), `--restricted`; `--bare` ignora hooks/CLAUDE.md — útil para execuções reproduzíveis, mas muda comportamento esperado [V].
- Codex: `-s` (sandbox), `--ephemeral`, `app-server` com aprovações, `codex sandbox` para comandos isolados [V].
- Cursor: `--mode plan|ask` somente leitura, `--sandbox enabled`, `--trust`, `-f` (força escrita), `--approve-mcps` — **não usar `-f`/`--yolo`/`--approve-mcps` sem aprovação nível ≥ 2** [V].

## 8. Registro e auditoria

Eventos obrigatórios: tarefa criada/iniciada/terminada, aprovações pedidas/decididas, negativas da política, comandos e arquivos tocados (quando a CLI informa), mudanças de prompt/política, atualizações de contexto. Campos: `ts, component, task_id, run_id, actor, action, outcome, hash_prev`.

## 9. Testes de segurança do MVP

Escape de diretório (`../`, symlink, caminho absoluto) · injeção em arquivos de README/código/logs · tentativa de ler `~/.ssh` e `.env` · variável de ambiente sensível não propagada · XSS por Markdown malicioso · aprovação reutilizada/expirada/alterada · cancelamento mata filhos · varredura de segredos em logs/banco · remoção de regras do prompt não amplia permissões (OR-17).

## 10. Pontos em aberto

D-005 (sandbox no macOS: confiar nas sandboxes das CLIs vs. camada própria), D-006 (aprovação mid-run), D-007 (transporte remoto), D-008 (termos de uso das assinaturas).

## 11. Controles implementados na Fase 1

| Controle | Onde | Verificação |
|---|---|---|
| CSP estrita na webview (`default-src 'self'`, sem `eval`, `object-src 'none'`, `frame-ancestors 'none'`) | `apps/desktop/src-tauri/tauri.conf.json` | Revisão; teste de build |
| Capability mínima do Tauri (`core:default` e arrastar janela); sem shell, sem FS, sem HTTP da webview | `src-tauri/capabilities/default.json` | Revisão |
| UI só chama comandos de leitura (`store_status`, `detect_local_gateway`) | `src-tauri/src/lib.rs` | Revisão |
| Detecção sem shell (`Command` com argumentos), timeout e kill do processo | `one-adapters/src/host.rs` | `kills_on_timeout` |
| Ambiente do filho por allowlist (sem vazar variáveis) | `host.rs` | `does_not_leak_environment` |
| Saída de CLI limitada em 4 MiB e nunca persistida | `host.rs` | Revisão |
| E-mail e organização retornados por `claude auth status` não são lidos nem guardados | `parse.rs` | `claude_models_are_unconfirmed_aliases_and_email_is_never_kept` |
| Banco com `foreign_keys=ON`, WAL, `CHECK` em enums | `one-store` | `rejects_invalid_enum_values` |
| Trilha de auditoria append-only (triggers) | migração V1 | Teste de triggers a adicionar na Fase 3 |
| Banco mais novo que o app é recusado (evita downgrade silencioso) | `migrations.rs` | `rejects_database_newer_than_app` |
| Dados de demonstração marcados `demo: true` e com selo visível | `src/demo/`, componente `DemoBadge` | `marca como DEMO…` |
| Nenhuma credencial no código, no banco ou nos fixtures | — | Revisão; varredura de padrões na Fase 3 (AC-11) |

Pendências de segurança para a Fase 2–3: escrita fora do workspace bloqueada no Gateway; aprovação com hash; varredura de injeção nos resultados.

## 12. Correções da auditoria adversarial (Fase 1)

| Achado | Correção | Testes |
|---|---|---|
| **A1 (alta)** Binário não verificado executado ao detectar | Descoberta separada da execução (`trust.rs`): `inspect` só lê. `Host::run` só aceita `TrustedExe`, criado após comparar a identidade com a aprovada pelo usuário. Identidade = caminho canônico + SHA-256 + dono + interpretador do `#!`. Verifica dono (usuário ou root) e rejeita escrita por **grupo ou outros** no arquivo e em todos os diretórios até a raiz (exceto sticky). Estados `unknown`/`changed`/`rejected` nunca executam. Aprovação por **diálogo nativo aberto pelo Rust** (plugin não concedido à webview), com conferência da impressão digital e um diálogo por vez; persistida em `trusted_executables` e auditada | `unknown_executable_is_found_but_never_executed`, `real_binary_runs_only_after_approval_and_not_after_replacement`, `symlink_resolves…`, `rejects_world_writable…` (grupo e outros), `records_env_interpreter…`, `approve_requires_matching_fingerprint…` |
| **A2 (média)** Timeout deixava descendentes e podia travar | `proc.rs`: grupo de processos próprio, `kill(-pgid)` no fim de toda execução, leitura **não bloqueante sem threads**, prazo total, limite de saída, cancelamento e resultado distinto (`Exited`, `TimedOut`, `Cancelled`, `OutputLimit`, `SpawnFailed`). stderr descartado | `descendant_holding_stdout_does_not_block_and_is_killed`, `timeout_kills_whole_group_quickly`, `output_flood…`, `cancel_stops_the_group` |
| **A3 (média)** Tarefa podia ligar Gateway A a projeto de B | Migração **V2**: índice único `(id, gateway_id)` em `projects` e FK composta em `tasks`, com `foreign_keys=OFF` fora da transação, `foreign_key_check` antes do commit e rollback em falha; `integrity_check` antes e depois; backup `VACUUM INTO` antes de migrar; leitura de volta de `foreign_keys=1` na abertura | `upgrade_v1_to_v2…`, `upgrade_aborts_without_deleting_data…`, `failing_migration_rolls_back…` |

**Rodada 2 e 3 da revisão (ACL e diálogo):**

| Achado | Tratamento |
|---|---|
| Bits `0o755` não mostram ACLs (`chmod +a "everyone allow write"` abre a escrita) | `acl.rs`: `acl_get_file`/`acl_get_entry`/`acl_get_permset_mask_np` do libSystem, sem dependência nova; aplicado ao arquivo, a todos os diretórios do caminho e ao interpretador. Testado com ACLs reais (7 permissões de arquivo, 3 de diretório, várias entradas, ACL em usuário específico, deny e read inofensivos) |
| Revisão apontou que o laço de `acl_get_entry` pararia na 1ª entrada | **Falso positivo no macOS** (retorna 0 em sucesso, não 1 como em POSIX.1e): os testes com ACL real provam a detecção, e há teste com a entrada permissiva por último. O laço agora falha fechado em qualquer erro que não seja o fim da lista |
| Grupo `admin` com 2º usuário administrador: `/opt/homebrew/Cellar` é gravável por outra pessoa | Reprovação correta (não é falso positivo). Em vez de abrir exceção, interpretadores (`env bash`) são resolvidos com diretórios do sistema primeiro, tanto na verificação quanto no PATH do filho (`runtime_dirs`); também impede que um `bash` em pasta do usuário sombreie o do sistema |
| Caminho com `\n` ou caracteres bidirecionais falsificando o diálogo | `display_safe`: controles e formatação invisível viram `\u{XXXX}`, limite de 300 caracteres; teste com caminho hostil |
| Teste do ACL do Tauri | Webview simulado com o contexto real: os 3 comandos passam; diálogo, janela, eventos, versão e outras janelas são negados |

Endurecimento adicional: comandos Tauri sob ACL por comando (`AppManifest`; a capability concede só os três comandos e arrastar a janela); CSP sem `unsafe-inline`; `detect_local_gateway` serializado e limitado a uma detecção por 3 s (`Throttle`); caminho do SQLite não é mais enviado à webview.

**Riscos residuais (aceitos e declarados):**
- Janela entre a verificação e o `exec` (não há `fexecve` no macOS). Mitigação parcial: executa-se o caminho canônico já verificado.
- O hash cobre o arquivo lançado, não o pacote inteiro (módulos npm) nem o conteúdo do interpretador (só o caminho dele).
- ACLs: no macOS, qualquer entrada ALLOW com escrita/exclusão/troca de dono ou de permissões reprova o arquivo e cada diretório do caminho (sem resolver o beneficiário: conservador). Entradas DENY são ignoradas. ACL ilegível = reprovado. Em outras plataformas a verificação falha fechada (Linux: ACLs POSIX entram na fase do Gateway Linux).
- Um descendente que se separe do grupo (nova sessão) pode sobreviver; a saída para de ser coletada, mas o processo não é morto.
- `foreign key` não substitui autorização: a validação de acesso entre clientes continua sendo da aplicação (Fase 3).
- Aprovação depende de o usuário reconhecer o arquivo no diálogo.

## 13. Execução real (Fase 2A, Codex)

| Controle | Onde | Verificação |
|---|---|---|
| Só binário aprovado, com identidade **revalidada imediatamente antes do exec** (além da checagem no `start`) | `one-exec` + `acquire` | `identity_is_revalidated_immediately_before_exec` (host que troca a identidade na 2ª inspeção: o processo nem inicia) |
| A webview envia só `workspace_id`, motor, modelo e prompt. Caminho, flags e comandos não existem no contrato | `StartRunRequest`, comandos Tauri sob ACL | `run_commands_are_reachable_and_enforce_policy_through_the_webview` |
| Workspace só por seletor **nativo** de pasta; recusa raiz, pastas do sistema e a pasta pessoal (ou ancestral); revalida o caminho canônico a cada execução (troca por symlink é recusada) | `workspace.rs`, `start` | `rejects_root_system_home…`, `workspace_path_swapped_for_a_symlink_is_refused` |
| Somente leitura: `-s read-only`; `workspace-write`, `danger-full-access`, `--dangerously-*` e `--full-auto` nunca são pedidos; modo `write` é recusado pelo adaptador | `CodexAdapter` | `args_are_structured_read_only_and_never_dangerous` |
| `--ignore-user-config`: MCPs, plugins e hooks do `config.toml` do usuário não carregam (a autenticação continua) | `CodexAdapter` | idem |
| Prompt por **stdin**, nunca em argv; modelo validado (não vira flag) | adaptador + proc | `approved_cli_runs…` (`argv` sem o prompt), `write_mode_and_bad_models_are_refused` |
| Uma execução por workspace; cancelamento, prazo (10 min) e limite de saída (8 MiB) encerram o **grupo de processos** | `ExecService`, `proc.rs` | `one_run_per_workspace…`, `cancel_stops…`, `timeout_is_enforced…`, `output_flood…` |
| Não se persistem saídas brutas de comandos; textos e comandos passam por truncamento e redação de segredos conhecidos; teto de 2000 eventos e 16 KB por evento | `engine.rs`, `runs.rs` | `command_output_is_never_kept`, `redaction_masks_common_secrets`, `event_limits_and_sql_safety` |
| Estado final sempre gravado: guarda contra pânico e reconciliação de execuções ativas ao abrir o app | `Guard`, `reconcile_interrupted` | `history_survives_restart…` |
| Auditoria: autorização de workspace, início, cancelamento e pedidos bloqueados | `audit_log` | `unapproved_cli_is_blocked…` |

**Riscos residuais novos (declarados):**
- **Leitura confinada ao workspace (corrigido nesta fase, ver `ISOLATION.md`).** O `-s read-only` do Codex não restringia leitura (medido). O adaptador passa a usar, por padrão, o perfil de permissão oficial do Codex (Beta) com `:root` negado: leitura de `~`, `/tmp`, outros projetos, `~/.ssh` e `~/.codex/auth.json` é negada pelo SO (teste por canário, sem custo de modelo). Limites: recurso Beta; só os comandos do agente são confinados, não o processo do Codex; a negação dentro do `codex exec` não foi observada porque o agente recusa antes de tentar; `git` e `node` falham dentro do agente. `TrustedLocalRead` (leitura ampla) existe só na API do adaptador, não na interface.
- O `codex exec --json` **não informa o modelo** usado: `model_reported` fica vazio em vez de ser inferido.
- A redação cobre padrões comuns (chaves com prefixo, Bearer, JWT, `X_TOKEN=`); não é garantia contra qualquer segredo.
- O resumo de ferramentas (comando) é persistido truncado em 300 caracteres e pode conter caminhos locais.
- Aprovação do Codex e autorização de pasta continuam dependendo de você reconhecer o que o diálogo mostra.

**Janela entre a revalidação e o `exec` (achado 4 da revisão da Fase 2A, risco aceito).** A identidade é revalidada imediatamente antes de iniciar o processo e o caminho executado é o canônico verificado, mas o macOS não oferece `fexecve` e o `Command` resolve o arquivo de novo: o que a revalidação garante é "o arquivo era o aprovado há instantes", não "o objeto executado é o verificado". Para explorar a janela é preciso substituir o binário ou um diretório do caminho, e esses caminhos só são aceitos se pertencerem a você ou a root e não forem graváveis por grupo, outros ou ACL; ou seja, quem consegue já tem o seu nível de privilégio (e poderia alterar o próprio app ou o banco). Mitigações possíveis futuras: executar uma cópia privada imutável (inviável para pacotes npm com arquivos relativos) ou Gateway sob usuário dedicado.

**Workspace entre `start` e o exec:** o caminho é revalidado (canônico, pasta válida, igual ao autorizado **e o mesmo diretório no disco: dispositivo + inode capturados no `start`**, o que detecta a pasta ser removida e recriada no mesmo caminho) dentro da thread, imediatamente antes de iniciar, e o Codex recebe esse caminho canônico em `-C` e como diretório de trabalho. Resta a mesma janela mínima, com a mesma ressalva de privilégio.

Limite explícito: a identidade do diretório (inode) é fixada no início de cada execução, não no momento da autorização da pasta; recriar a pasta do projeto entre execuções (ex.: novo clone) continua válido de propósito. As permissões dos diretórios pais do workspace não são verificadas.

## 14. Limitações registradas no encerramento da Fase 2A

- **Git e Node não funcionam dentro do agente** no modo restrito (medido). Foi decisão deliberada **não** liberar caminhos para consertar isso (`ISOLATION.md` §6).
- O perfil de permissão do Codex é **Beta** e confina os **comandos** do agente, não o processo do Codex inteiro; não é garantia absoluta de isolamento.
- `TrustedLocalRead` não é exposto na interface e não há mecanismo de ativação para o usuário.
- Não há sandbox de Linux nem de Windows; a verificação de ACL falha fechada fora do macOS.
- Dentro do `codex exec` a negação pelo SO não foi observada (o agente recusa antes de tentar); a prova a nível de SO usa `codex sandbox` com o mesmo perfil.
- A execução por interface (aprovar o Codex, autorizar pasta, enviar tarefa e ver o histórico) **não tem confirmação registrada**: o banco real do usuário não contém aprovações, workspaces nem execuções.
