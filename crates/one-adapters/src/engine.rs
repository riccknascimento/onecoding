//! Contrato de adaptador de motor (Codex hoje; Claude e Cursor reutilizam o mesmo contrato).
//!
//! O adaptador só TRADUZ: monta os argumentos (estruturados, sem shell) e converte a saída do motor
//! em eventos normalizados. Quem executa, limita, cancela e persiste é o serviço de execução.

use std::path::Path;

use one_protocol::{EngineKind, EventPayload, ExecMode, UsageInfo};

/// Pedido já validado pelo serviço (workspace autorizado, modelo sanitizado, prompt limitado).
pub struct ExecRequest<'a> {
    pub workspace: &'a Path,
    pub confinement: Confinement,
    pub model: Option<&'a str>,
    pub mode: ExecMode,
    pub prompt: &'a str,
}

/// Até onde o agente pode LER. A escrita é sempre negada nesta fase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confinement {
    /// Só o workspace (mais caminhos mínimos de plataforma). Leitura de `~`, `/tmp`, outros projetos,
    /// `~/.ssh` e `~/.codex/auth.json` é negada pelo sandbox. Padrão.
    WorkspaceOnly,
    /// Sandbox `read-only` do Codex: NÃO restringe leitura (o agente lê qualquer arquivo do usuário).
    /// Só para desenvolvimento local confiável; a UI não expõe esta opção.
    TrustedLocalRead,
    /// Leitura limitada pelas ferramentas do próprio motor (sem isolamento de sistema operacional).
    /// Não equivale a `WorkspaceOnly`: plugins e hooks do usuário podem continuar ativos.
    ToolRestricted,
}

/// O que o motor informou ao final. Campos que o motor não informa ficam `None`/0 (nunca inventados).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ParsedSummary {
    /// Sessão interna do motor (ex.: thread do Codex).
    pub session_id: Option<String>,
    /// Modelo efetivamente informado pelo executor (o `codex exec --json` não informa).
    pub model_reported: Option<String>,
    pub usage: UsageInfo,
    pub tool_calls: u32,
    pub final_text: Option<String>,
    /// Mensagem de falha informada pelo motor, se houver.
    pub failure: Option<String>,
    pub turn_completed: bool,
}

/// Conversor incremental da saída (uma linha por vez).
pub trait EngineParser: Send {
    fn feed(&mut self, line: &str) -> Vec<EventPayload>;
    fn summary(&self) -> ParsedSummary;
}

pub trait EngineAdapter: Send + Sync {
    fn engine(&self) -> EngineKind;
    /// Confinamento de leitura que este motor sabe aplicar (o serviço o passa a `args`).
    fn confinement(&self) -> Confinement;
    /// `Some(motivo)` = motor bloqueado para execução (isolamento não verificado). `None` = liberado.
    fn execution_block(&self) -> Option<&'static str> {
        None
    }
    /// Argumentos estruturados. O prompt NÃO entra aqui: vai por stdin. `Err` = pedido não suportado.
    fn args(&self, req: &ExecRequest<'_>) -> Result<Vec<String>, String>;
    fn parser(&self) -> Box<dyn EngineParser>;
}

/// Nomes de modelo aceitos da UI: curtos, sem espaços e sem começar com `-` (não vira flag).
pub fn valid_model(m: &str) -> bool {
    !m.is_empty()
        && m.len() <= 64
        && !m.starts_with('-')
        && m.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '-' | '/'))
}

// ───────────── Sanitização do que é persistido/exibido ─────────────

pub fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max).collect();
        t.push('…');
        t
    }
}

/// Remove segredos óbvios antes de guardar ou mostrar: chaves com prefixos conhecidos, tokens JWT,
/// `Bearer …`, blocos de chave privada e atribuições `X_TOKEN=valor`. Não é prova de ausência de
/// segredos: por isso saídas brutas de comandos não são persistidas.
pub fn redact(s: &str) -> String {
    const PREFIXES: [&str; 8] = [
        "sk-",
        "ghp_",
        "gho_",
        "github_pat_",
        "AKIA",
        "xoxb-",
        "xoxp-",
        "eyJ",
    ];
    const SENSITIVE: [&str; 5] = ["TOKEN", "SECRET", "PASSWORD", "API_KEY", "APIKEY"];
    let mut out = String::with_capacity(s.len());
    for line in s.split_inclusive('\n') {
        if line.contains("PRIVATE KEY") {
            out.push_str("[REDACTED: chave privada]\n");
            continue;
        }
        let mut prev_bearer = false;
        for word in line.split_inclusive(char::is_whitespace) {
            let core = word.trim_end();
            let tail = &word[core.len()..];
            let mut w = core.to_string();
            if prev_bearer && core.len() >= 8 {
                w = "[REDACTED]".into();
            } else if let Some((k, v)) = core.split_once('=') {
                if !v.is_empty() && SENSITIVE.iter().any(|m| k.to_ascii_uppercase().contains(m)) {
                    w = format!("{k}=[REDACTED]");
                }
            }
            // qualquer trecho (separado por símbolos) que pareça uma chave conhecida
            for piece in core.split(|c: char| !(c.is_ascii_alphanumeric() || "_-.".contains(c))) {
                if piece.len() >= 20 && PREFIXES.iter().any(|p| piece.starts_with(p)) {
                    w = w.replace(piece, "[REDACTED]");
                }
            }
            prev_bearer = core.eq_ignore_ascii_case("bearer");
            out.push_str(&w);
            out.push_str(tail);
        }
    }
    out
}

// ───────────────────────── Codex ─────────────────────────

/// Leitura só no workspace (`-C`). `:tmpdir`/`:slash_tmp` negados (senão `/tmp` ficaria legível, medido).
/// Custo conhecido: ferramentas que leem fora disso (`git` via Xcode CLT, `node`/OpenSSL) falham no agente.
pub const WORKSPACE_ONLY_DEFAULT: &str = "default_permissions=\"oa_workspace_ro\"";
pub const WORKSPACE_ONLY_PROFILE: &str = "permissions.oa_workspace_ro.filesystem={\":root\"=\"deny\",\":minimal\"=\"read\",\":tmpdir\"=\"deny\",\":slash_tmp\"=\"deny\",\":workspace_roots\"={\".\"=\"read\"}}";

pub struct CodexAdapter;

impl EngineAdapter for CodexAdapter {
    fn engine(&self) -> EngineKind {
        EngineKind::Codex
    }

    fn confinement(&self) -> Confinement {
        Confinement::WorkspaceOnly
    }

    fn args(&self, req: &ExecRequest<'_>) -> Result<Vec<String>, String> {
        if req.mode != ExecMode::ReadOnly {
            return Err(
                "somente leitura por enquanto: o modo de escrita ainda não é suportado".into(),
            );
        }
        let mut a: Vec<String> = [
            "exec",
            "--json",
            // não carrega MCPs, plugins nem hooks do config.toml do usuário (a autenticação continua)
            "--ignore-user-config",
            // não grava rollout de sessão em ~/.codex
            "--ephemeral",
            "--skip-git-repo-check",
            "--color",
            "never",
        ]
        .map(String::from)
        .to_vec();
        match req.confinement {
            // Perfil de permissão do Codex (Beta): nega tudo, libera só o mínimo de plataforma e o
            // workspace, somente leitura. NÃO combinar com `-s`: o sandbox legado anularia o perfil.
            Confinement::WorkspaceOnly => {
                a.extend(["-c".into(), WORKSPACE_ONLY_DEFAULT.into()]);
                a.extend(["-c".into(), WORKSPACE_ONLY_PROFILE.into()]);
            }
            // `workspace-write`/`danger-full-access` nunca são pedidos.
            Confinement::TrustedLocalRead => a.extend(["-s".into(), "read-only".into()]),
            Confinement::ToolRestricted => {
                return Err("o Codex não usa confinamento por ferramenta".into())
            }
        }
        a.push("-C".into());
        a.push(req.workspace.display().to_string());
        if let Some(m) = req.model {
            if !valid_model(m) {
                return Err("nome de modelo inválido".into());
            }
            a.push("-m".into());
            a.push(m.into());
        }
        a.push("-".into()); // o prompt é lido do stdin
        Ok(a)
    }

    fn parser(&self) -> Box<dyn EngineParser> {
        Box::new(CodexParser::default())
    }
}

/// Eventos do `codex exec --json` (formato observado na versão 0.169.0).
#[derive(Default)]
pub struct CodexParser {
    sum: ParsedSummary,
}

fn u32_of(v: &serde_json::Value, k: &str) -> Option<u32> {
    v.get(k)?.as_u64().and_then(|n| u32::try_from(n).ok())
}

impl EngineParser for CodexParser {
    fn feed(&mut self, line: &str) -> Vec<EventPayload> {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line.trim()) else {
            return vec![]; // linha que não é JSON (ex.: aviso solto): ignorada
        };
        let ty = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
        let item = v.get("item");
        let item_ty = item
            .and_then(|i| i.get("type"))
            .and_then(|t| t.as_str())
            .unwrap_or("");
        match (ty, item_ty) {
            ("thread.started", _) => {
                self.sum.session_id = v
                    .get("thread_id")
                    .and_then(|t| t.as_str())
                    .map(String::from);
                vec![]
            }
            ("item.completed", "agent_message") => {
                let text = redact(&truncate(
                    item.and_then(|i| i.get("text"))
                        .and_then(|t| t.as_str())
                        .unwrap_or(""),
                    8000,
                ));
                self.sum.final_text = Some(text.clone());
                vec![EventPayload::Message { text }]
            }
            ("item.completed", "error") => vec![EventPayload::Warning {
                message: redact(&truncate(
                    item.and_then(|i| i.get("message"))
                        .and_then(|t| t.as_str())
                        .unwrap_or(""),
                    500,
                )),
            }],
            ("item.started", "command_execution") => {
                self.sum.tool_calls += 1;
                let cmd = item
                    .and_then(|i| i.get("command"))
                    .and_then(|c| c.as_str())
                    .unwrap_or("");
                vec![EventPayload::ToolStarted {
                    name: "shell".into(),
                    summary: redact(&truncate(cmd, 300)),
                }]
            }
            ("item.completed", "command_execution") => {
                let code = item
                    .and_then(|i| i.get("exit_code"))
                    .and_then(|c| c.as_i64())
                    .map(|c| c as i32);
                // a saída agregada do comando NÃO é guardada (pode conter conteúdo de arquivos e segredos)
                vec![EventPayload::ToolFinished {
                    name: "shell".into(),
                    ok: code == Some(0),
                    exit_code: code,
                }]
            }
            ("item.started", "mcp_tool_call" | "web_search" | "file_change") => {
                self.sum.tool_calls += 1;
                vec![EventPayload::ToolStarted {
                    name: item_ty.into(),
                    summary: String::new(),
                }]
            }
            ("item.completed", "file_change") => vec![EventPayload::Warning {
                message: "o motor reportou alteração de arquivo em execução somente leitura".into(),
            }],
            ("turn.completed", _) => {
                self.sum.turn_completed = true;
                let u = v.get("usage").cloned().unwrap_or_default();
                let add = |cur: Option<u32>, n: Option<u32>| match (cur, n) {
                    (Some(a), Some(b)) => Some(a.saturating_add(b)),
                    (a, b) => a.or(b),
                };
                let us = &mut self.sum.usage;
                us.input_tokens = add(us.input_tokens, u32_of(&u, "input_tokens"));
                us.output_tokens = add(us.output_tokens, u32_of(&u, "output_tokens"));
                us.cached_tokens = add(us.cached_tokens, u32_of(&u, "cached_input_tokens"));
                us.reasoning_tokens =
                    add(us.reasoning_tokens, u32_of(&u, "reasoning_output_tokens"));
                vec![EventPayload::Usage { usage: us.clone() }]
            }
            ("error", _) => {
                self.sum.failure = v
                    .get("message")
                    .and_then(|m| m.as_str())
                    .map(|m| redact(&truncate(m, 500)));
                vec![] // o `turn.failed` que segue gera o evento de erro
            }
            ("turn.failed", _) => {
                let m = v
                    .pointer("/error/message")
                    .and_then(|m| m.as_str())
                    .map(|m| redact(&truncate(m, 500)));
                self.sum.failure = m.clone().or(self.sum.failure.take());
                vec![EventPayload::Error {
                    kind: one_protocol::ErrorKind::ExecutorFailed,
                    message: self
                        .sum
                        .failure
                        .clone()
                        .unwrap_or_else(|| "o motor reportou falha".into()),
                }]
            }
            _ => vec![],
        }
    }

    fn summary(&self) -> ParsedSummary {
        self.sum.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(format!(
            "{}/tests/fixtures/codex/{name}",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap()
    }

    fn run(name: &str) -> (Vec<EventPayload>, ParsedSummary) {
        let mut p = CodexParser::default();
        let ev: Vec<_> = fixture(name).lines().flat_map(|l| p.feed(l)).collect();
        (ev, p.summary())
    }

    #[test]
    fn parses_real_success_stream() {
        let (ev, s) = run("success.jsonl");
        assert_eq!(
            s.session_id.as_deref(),
            Some("01a1ea72-6665-76f2-91bd-9907d3957816")
        );
        assert_eq!(s.tool_calls, 3);
        assert!(s.turn_completed && s.failure.is_none());
        assert_eq!(s.usage.input_tokens, Some(67838));
        assert_eq!(s.usage.cached_tokens, Some(56704));
        assert_eq!(s.usage.output_tokens, Some(137));
        assert_eq!(s.usage.reasoning_tokens, Some(0));
        assert_eq!(
            s.model_reported, None,
            "o Codex não informa o modelo: não se inventa"
        );
        assert!(s.final_text.unwrap().contains("Demo"));
        let tools_started = ev
            .iter()
            .filter(|e| matches!(e, EventPayload::ToolStarted { .. }))
            .count();
        let tools_done = ev
            .iter()
            .filter(|e| matches!(e, EventPayload::ToolFinished { ok: true, .. }))
            .count();
        assert_eq!((tools_started, tools_done), (3, 3));
        assert!(ev.iter().any(|e| matches!(e, EventPayload::Usage { .. })));
    }

    #[test]
    fn command_output_is_never_kept() {
        let (ev, _) = run("success.jsonl");
        let json = serde_json::to_string(&ev).unwrap();
        assert!(
            !json.contains("total 16") && !json.contains("Projeto de teste"),
            "saída bruta vazou: {json}"
        );
    }

    #[test]
    fn parses_real_failure_stream() {
        let (ev, s) = run("failure.jsonl");
        assert!(!s.turn_completed);
        assert!(s.failure.as_deref().unwrap().contains("not supported"));
        assert!(ev.iter().any(|e| matches!(
            e,
            EventPayload::Error {
                kind: one_protocol::ErrorKind::ExecutorFailed,
                ..
            }
        )));
        assert!(
            ev.iter().any(|e| matches!(e, EventPayload::Warning { .. })),
            "o aviso de metadados vira warning"
        );
    }

    #[test]
    fn tolerates_garbage_and_unknown_events() {
        let mut p = CodexParser::default();
        for l in [
            "",
            "não é json",
            "{\"type\":\"novo.evento\"}",
            "{\"type\":\"item.completed\",\"item\":{\"type\":\"reasoning\"}}",
            "[1,2]",
        ] {
            assert!(p.feed(l).is_empty(), "{l}");
        }
    }

    #[test]
    fn args_are_structured_read_only_and_never_dangerous() {
        let ws = Path::new("/tmp/ws");
        let a = CodexAdapter
            .args(&ExecRequest {
                workspace: ws,
                confinement: Confinement::WorkspaceOnly,
                model: Some("gpt-6.1-sol"),
                mode: ExecMode::ReadOnly,
                prompt: "$(rm -rf /); `x`",
            })
            .unwrap();
        let j = a.join(" ");
        assert!(
            j.contains("default_permissions=\"oa_workspace_ro\"")
                && j.contains("\":root\"=\"deny\"")
                && j.contains("\":tmpdir\"=\"deny\"")
                && j.contains("-C /tmp/ws")
                && j.contains("-m gpt-6.1-sol")
        );
        // com o perfil de permissão NÃO pode haver -s: o sandbox legado anularia o perfil
        assert!(!a.iter().any(|x| x == "-s"), "{j}");
        // o modo antigo existe só como opção explícita da API do adaptador
        let t = CodexAdapter
            .args(&ExecRequest {
                workspace: ws,
                confinement: Confinement::TrustedLocalRead,
                model: None,
                mode: ExecMode::ReadOnly,
                prompt: "x",
            })
            .unwrap()
            .join(" ");
        assert!(
            t.contains("-s read-only") && !t.contains("default_permissions"),
            "{t}"
        );
        assert!(j.contains("--ignore-user-config") && j.contains("--ephemeral"));
        for bad in [
            "dangerously",
            "full-auto",
            "workspace-write",
            "danger-full-access",
            "rm -rf",
            "`x`",
        ] {
            assert!(!j.contains(bad), "{bad} em {j}");
        }
        assert_eq!(a.last().unwrap(), "-", "prompt por stdin, nunca em argv");
    }

    #[test]
    fn write_mode_and_bad_models_are_refused() {
        let ws = Path::new("/tmp/ws");
        let r = |model, mode| {
            CodexAdapter.args(&ExecRequest {
                workspace: ws,
                confinement: Confinement::WorkspaceOnly,
                model,
                mode,
                prompt: "x",
            })
        };
        assert!(r(None, ExecMode::Write).is_err());
        let long = "a".repeat(65);
        for m in [
            "--dangerously-bypass-approvals-and-sandbox",
            "-s",
            "a b",
            "x;y",
            "",
            &long,
            "m$(id)",
        ] {
            assert!(r(Some(m), ExecMode::ReadOnly).is_err(), "{m}");
        }
        assert!(r(Some("gpt-5.6-sol"), ExecMode::ReadOnly).is_ok());
    }

    #[test]
    fn redaction_masks_common_secrets() {
        let s = redact("curl -H 'Authorization: Bearer abcdefghijklmnop' && export OPENAI_API_KEY=sk-abcdefghijklmnopqrstuvwxyz GH=ghp_abcdefghijklmnopqrstuvwxyz1234 normal=1");
        for leak in ["abcdefghijklmnop'", "sk-abcdefghij", "ghp_abcdefghij"] {
            assert!(!s.contains(leak), "{leak} em {s}");
        }
        assert!(s.contains("normal=1"));
        assert!(redact("-----BEGIN OPENSSH PRIVATE KEY-----\nabc\n").contains("REDACTED"));
        assert_eq!(truncate("áéíóú", 3), "áéí…");
    }
}
