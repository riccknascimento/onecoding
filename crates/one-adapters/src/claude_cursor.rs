//! Adapters do Claude Code e do Cursor CLI (mesmo contrato do Codex, `engine.rs`).
//!
//! Os dois ficam BLOQUEADOS para execução (`execution_block`): o teste real mostrou que o isolamento
//! de leitura não é equivalente ao do Codex. O adapter continua pronto (argumentos, parser, testes) para
//! ser habilitado quando o bloqueio for removido por decisão explícita do usuário.
//!
//! Formatos observados em 2026-11-18 com um teste real por motor (fixtures em `tests/fixtures/`).

use std::collections::HashMap;

use one_protocol::{EngineKind, ErrorKind, EventPayload, ExecMode, UsageInfo};
use serde_json::Value;

use crate::engine::{
    redact, truncate, valid_model, Confinement, EngineAdapter, EngineParser, ExecRequest,
    ParsedSummary,
};

/// Motivo do bloqueio do Claude Code. Os plugins do usuário continuam ativos no modo restrito e podem
/// registrar hooks que rodam fora do isolamento de ferramentas.
pub const CLAUDE_BLOCK: &str = "Claude Code bloqueado: com --safe-mode os plugins ainda aparecem na inicialização e não há prova de que os hooks ficaram desligados. Aguarda decisão do usuário.";

/// Motivo do bloqueio do Cursor CLI. No teste real, a ferramenta de leitura acessou arquivo fora do
/// workspace apesar de `--sandbox enabled`.
pub const CURSOR_BLOCK: &str = "Cursor CLI bloqueado: no teste real a leitura acessou arquivo fora do workspace, apesar de --sandbox enabled. Isolamento não verificado.";

/// Ferramentas de leitura permitidas no modo restrito. Qualquer outra (Bash, escrita, web, MCP) é negada.
const CLAUDE_TOOLS: &str = "Read,Grep,Glob";
const CLAUDE_DENIED: &str = "Bash,Edit,Write,NotebookEdit,WebFetch,WebSearch";
/// Teto de custo por execução (USD), aplicado pelo próprio Claude Code.
const CLAUDE_BUDGET_USD: &str = "0.30";

fn read_only(req: &ExecRequest<'_>) -> Result<(), String> {
    if req.mode != ExecMode::ReadOnly {
        return Err("somente leitura por enquanto: o modo de escrita ainda não é suportado".into());
    }
    Ok(())
}

// ───────────────────────── Claude Code ─────────────────────────

pub struct ClaudeAdapter;

impl EngineAdapter for ClaudeAdapter {
    fn engine(&self) -> EngineKind {
        EngineKind::Claude
    }

    fn confinement(&self) -> Confinement {
        Confinement::ToolRestricted
    }

    fn execution_block(&self) -> Option<&'static str> {
        Some(CLAUDE_BLOCK)
    }

    fn args(&self, req: &ExecRequest<'_>) -> Result<Vec<String>, String> {
        read_only(req)?;
        if req.confinement != Confinement::ToolRestricted {
            return Err("o Claude Code só roda com confinamento por ferramenta".into());
        }
        let mut a: Vec<String> = [
            "-p",
            "--output-format",
            "stream-json",
            "--verbose",
            // ignora configurações de usuário/projeto/local e MCP; confina ferramentas de arquivo ao workspace
            "--restricted",
            // desliga plugins, hooks, MCP e comandos do usuário; preserva login, ferramentas e permissões
            "--safe-mode",
            "--tools",
            CLAUDE_TOOLS,
            "--disallowedTools",
            CLAUDE_DENIED,
            "--permission-mode",
            "dontAsk",
            "--strict-mcp-config",
            "--no-session-persistence",
            "--max-budget-usd",
            CLAUDE_BUDGET_USD,
        ]
        .map(String::from)
        .to_vec();
        if let Some(m) = req.model {
            if !valid_model(m) {
                return Err("nome de modelo inválido".into());
            }
            a.push("--model".into());
            a.push(m.into());
        }
        // o prompt NÃO entra nos argumentos: vai pelo stdin (`claude -p` lê stdin quando não há prompt)
        Ok(a)
    }

    fn parser(&self) -> Box<dyn EngineParser> {
        Box::new(ClaudeParser::default())
    }
}

/// Eventos do `claude -p --output-format stream-json --verbose`.
/// Nunca guarda o conteúdo devolvido por ferramentas (`tool_result`): é o conteúdo dos arquivos lidos.
#[derive(Default)]
pub struct ClaudeParser {
    sum: ParsedSummary,
    tool_names: HashMap<String, String>,
}

fn int(v: &Value, k: &str) -> Option<u32> {
    v.get(k)?.as_u64().and_then(|n| u32::try_from(n).ok())
}

impl EngineParser for ClaudeParser {
    fn feed(&mut self, line: &str) -> Vec<EventPayload> {
        let Ok(v) = serde_json::from_str::<Value>(line.trim()) else {
            return vec![];
        };
        match v.get("type").and_then(|t| t.as_str()).unwrap_or("") {
            "system" => match v.get("subtype").and_then(|s| s.as_str()) {
                Some("init") => {
                    self.sum.model_reported =
                        v.get("model").and_then(|m| m.as_str()).map(String::from);
                    self.sum.session_id = v
                        .get("session_id")
                        .and_then(|m| m.as_str())
                        .map(String::from);
                    vec![]
                }
                Some("permission_denied") => vec![EventPayload::Warning {
                    message: "uso de ferramenta negado pelo modo restrito".into(),
                }],
                _ => vec![],
            },
            "assistant" => {
                let mut out = vec![];
                for c in content(&v) {
                    if c.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                        let name = c
                            .get("name")
                            .and_then(|n| n.as_str())
                            .unwrap_or("tool")
                            .to_string();
                        if let Some(id) = c.get("id").and_then(|i| i.as_str()) {
                            self.tool_names.insert(id.to_string(), name.clone());
                        }
                        self.sum.tool_calls += 1;
                        // o nome da ferramenta basta; os argumentos (caminhos) não são guardados
                        out.push(EventPayload::ToolStarted {
                            name,
                            summary: String::new(),
                        });
                    }
                }
                out
            }
            "user" => {
                let mut out = vec![];
                for c in content(&v) {
                    if c.get("type").and_then(|t| t.as_str()) == Some("tool_result") {
                        let id = c.get("tool_use_id").and_then(|i| i.as_str()).unwrap_or("");
                        let name = self.tool_names.remove(id).unwrap_or_else(|| "tool".into());
                        let ok = !c.get("is_error").and_then(|e| e.as_bool()).unwrap_or(false);
                        out.push(EventPayload::ToolFinished {
                            name,
                            ok,
                            exit_code: None,
                        });
                    }
                }
                out
            }
            "result" => {
                let u = v.get("usage").cloned().unwrap_or_default();
                let cache_read = int(&u, "cache_read_input_tokens");
                let cache_write = int(&u, "cache_creation_input_tokens");
                let input = int(&u, "input_tokens").map(|i| {
                    i.saturating_add(cache_read.unwrap_or(0))
                        .saturating_add(cache_write.unwrap_or(0))
                });
                self.sum.usage = UsageInfo {
                    input_tokens: input,
                    output_tokens: int(&u, "output_tokens"),
                    cached_tokens: cache_read,
                    reasoning_tokens: u
                        .pointer("/output_tokens_details/thinking_tokens")
                        .and_then(|t| t.as_u64())
                        .and_then(|t| u32::try_from(t).ok()),
                };
                self.sum.turn_completed = true;
                let is_error = v.get("is_error").and_then(|e| e.as_bool()).unwrap_or(false);
                let text = v.get("result").and_then(|r| r.as_str()).unwrap_or("");
                let mut out = vec![EventPayload::Usage {
                    usage: self.sum.usage.clone(),
                }];
                if is_error || v.get("subtype").and_then(|s| s.as_str()) != Some("success") {
                    let msg = redact(&truncate(text, 500));
                    self.sum.failure = Some(msg.clone());
                    out.insert(
                        0,
                        EventPayload::Error {
                            kind: ErrorKind::ExecutorFailed,
                            message: msg,
                        },
                    );
                } else {
                    let answer = redact(&truncate(text, 8000));
                    self.sum.final_text = Some(answer.clone());
                    out.insert(0, EventPayload::Message { text: answer });
                }
                out
            }
            _ => vec![],
        }
    }

    fn summary(&self) -> ParsedSummary {
        self.sum.clone()
    }
}

fn content(v: &Value) -> Vec<Value> {
    v.pointer("/message/content")
        .and_then(|c| c.as_array())
        .cloned()
        .unwrap_or_default()
}

// ───────────────────────── Cursor CLI ─────────────────────────

pub struct CursorAdapter;

impl EngineAdapter for CursorAdapter {
    fn engine(&self) -> EngineKind {
        EngineKind::Cursor
    }

    fn confinement(&self) -> Confinement {
        Confinement::ToolRestricted
    }

    fn execution_block(&self) -> Option<&'static str> {
        Some(CURSOR_BLOCK)
    }

    fn args(&self, req: &ExecRequest<'_>) -> Result<Vec<String>, String> {
        read_only(req)?;
        if req.confinement != Confinement::ToolRestricted {
            return Err("o Cursor CLI só roda com confinamento por ferramenta".into());
        }
        // `--trust`: a pasta já foi autorizada no OneAgent; sem ele o modo não-interativo recusa.
        // `--mode ask` é Q&A somente leitura; nunca `--force`/`--yolo`.
        let mut a: Vec<String> = [
            "-p",
            "--trust",
            "--output-format",
            "stream-json",
            "--mode",
            "ask",
            "--sandbox",
            "enabled",
        ]
        .map(String::from)
        .to_vec();
        if let Some(m) = req.model {
            if !valid_model(m) {
                return Err("nome de modelo inválido".into());
            }
            a.push("--model".into());
            a.push(m.into());
        }
        Ok(a)
    }

    fn parser(&self) -> Box<dyn EngineParser> {
        Box::new(CursorParser::default())
    }
}

/// Eventos do `cursor-agent -p --output-format stream-json`. Eventos `thinking` (raciocínio) e
/// `assistant` (texto parcial) NÃO são guardados; o texto final vem do evento `result`.
#[derive(Default)]
pub struct CursorParser {
    sum: ParsedSummary,
    tool_names: HashMap<String, String>,
}

impl EngineParser for CursorParser {
    fn feed(&mut self, line: &str) -> Vec<EventPayload> {
        let Ok(v) = serde_json::from_str::<Value>(line.trim()) else {
            return vec![];
        };
        match v.get("type").and_then(|t| t.as_str()).unwrap_or("") {
            "system" => {
                self.sum.model_reported = v.get("model").and_then(|m| m.as_str()).map(String::from);
                self.sum.session_id = v
                    .get("session_id")
                    .and_then(|m| m.as_str())
                    .map(String::from);
                vec![]
            }
            "tool_call" => {
                let call_id = v
                    .get("call_id")
                    .and_then(|c| c.as_str())
                    .unwrap_or("")
                    .to_string();
                let tool = v.get("tool_call").cloned().unwrap_or_default();
                let name = tool_name(&tool);
                match v.get("subtype").and_then(|s| s.as_str()) {
                    Some("started") => {
                        self.tool_names.insert(call_id, name.clone());
                        self.sum.tool_calls += 1;
                        vec![EventPayload::ToolStarted {
                            name,
                            summary: String::new(),
                        }]
                    }
                    Some("completed") => {
                        let name = self.tool_names.remove(&call_id).unwrap_or(name);
                        let ok = tool
                            .pointer(&format!("/{}/result/success", tool_key(&tool)))
                            .is_some();
                        vec![EventPayload::ToolFinished {
                            name,
                            ok,
                            exit_code: None,
                        }]
                    }
                    _ => vec![],
                }
            }
            "result" => {
                let u = v.get("usage").cloned().unwrap_or_default();
                self.sum.usage = UsageInfo {
                    input_tokens: int(&u, "inputTokens"),
                    output_tokens: int(&u, "outputTokens"),
                    cached_tokens: int(&u, "cacheReadTokens"),
                    reasoning_tokens: int(&u, "reasoningTokens"),
                };
                self.sum.turn_completed = true;
                let is_error = v.get("is_error").and_then(|e| e.as_bool()).unwrap_or(false);
                let text = v.get("result").and_then(|r| r.as_str()).unwrap_or("");
                let mut out = vec![EventPayload::Usage {
                    usage: self.sum.usage.clone(),
                }];
                if is_error || v.get("subtype").and_then(|s| s.as_str()) != Some("success") {
                    let msg = redact(&truncate(text, 500));
                    self.sum.failure = Some(msg.clone());
                    out.insert(
                        0,
                        EventPayload::Error {
                            kind: ErrorKind::ExecutorFailed,
                            message: msg,
                        },
                    );
                } else {
                    let answer = redact(&truncate(text, 8000));
                    self.sum.final_text = Some(answer.clone());
                    out.insert(0, EventPayload::Message { text: answer });
                }
                out
            }
            _ => vec![],
        }
    }

    fn summary(&self) -> ParsedSummary {
        self.sum.clone()
    }
}

/// Chave do tipo de ferramenta (ex.: `readToolCall`), ignorando os campos comuns do evento.
fn tool_key(tool: &Value) -> String {
    tool.as_object()
        .and_then(|o| o.keys().find(|k| k.ends_with("ToolCall")).cloned())
        .unwrap_or_default()
}

fn tool_name(tool: &Value) -> String {
    let k = tool_key(tool);
    k.strip_suffix("ToolCall").unwrap_or("tool").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn fixture(engine: &str) -> String {
        std::fs::read_to_string(format!(
            "{}/tests/fixtures/{engine}/success.jsonl",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap()
    }

    fn req(conf: Confinement, model: Option<&str>) -> ExecRequest<'_> {
        ExecRequest {
            workspace: Path::new("/tmp/ws"),
            confinement: conf,
            model,
            mode: ExecMode::ReadOnly,
            prompt: "x",
        }
    }

    #[test]
    fn both_engines_are_blocked_and_codex_is_not() {
        assert!(ClaudeAdapter.execution_block().is_some());
        assert!(CursorAdapter.execution_block().is_some());
        assert!(crate::CodexAdapter.execution_block().is_none());
    }

    #[test]
    fn claude_args_are_restricted_read_only_and_prompt_is_on_stdin() {
        let a = ClaudeAdapter
            .args(&req(Confinement::ToolRestricted, Some("sonnet")))
            .unwrap();
        let j = a.join(" ");
        for must in [
            "-p",
            "--restricted",
            "--tools Read,Grep,Glob",
            "--strict-mcp-config",
            "--permission-mode dontAsk",
            "--model sonnet",
        ] {
            assert!(j.contains(must), "{must} em {j}");
        }
        for bad in [
            "bypassPermissions",
            "--dangerously",
            "--allowedTools",
            "--bare",
            "--add-dir",
        ] {
            assert!(!j.contains(bad), "{bad}");
        }
        assert!(j.contains("--disallowedTools Bash,Edit,Write"), "{j}");
        // sem confinamento por ferramenta, recusa
        assert!(ClaudeAdapter
            .args(&req(Confinement::WorkspaceOnly, None))
            .is_err());
        // escrita, recusa
        let mut w = req(Confinement::ToolRestricted, None);
        w.mode = ExecMode::Write;
        assert!(ClaudeAdapter.args(&w).is_err());
        assert!(ClaudeAdapter
            .args(&req(Confinement::ToolRestricted, Some("-x")))
            .is_err());
    }

    #[test]
    fn cursor_args_are_ask_mode_sandboxed_and_never_force() {
        let a = CursorAdapter
            .args(&req(Confinement::ToolRestricted, None))
            .unwrap();
        let j = a.join(" ");
        assert!(j.contains("--mode ask") && j.contains("--sandbox enabled"));
        for bad in ["--force", "--yolo", "-f", "bypass"] {
            assert!(!a.iter().any(|x| x == bad), "{bad}");
        }
        assert!(CursorAdapter
            .args(&req(Confinement::WorkspaceOnly, None))
            .is_err());
    }

    #[test]
    fn claude_stream_maps_to_normalized_events_without_tool_content() {
        let mut p = ClaudeParser::default();
        let ev: Vec<_> = fixture("claude").lines().flat_map(|l| p.feed(l)).collect();
        let s = p.summary();
        assert_eq!(s.model_reported.as_deref(), Some("claude-sonnet-5"));
        assert_eq!(s.tool_calls, 2);
        assert!(s.turn_completed && s.failure.is_none());
        assert_eq!(s.usage.output_tokens, Some(288));
        assert_eq!(s.usage.cached_tokens, Some(8250));
        assert!(s.final_text.unwrap().contains("ABACAXI-42"));
        assert!(
            ev.iter().any(|e| matches!(e, EventPayload::Warning { .. })),
            "negação de permissão"
        );
        let json = serde_json::to_string(&ev).unwrap();
        assert!(
            !json.contains("Projeto de teste") && !json.contains("CANARY-7731"),
            "conteúdo de arquivo vazou: {json}"
        );
    }

    #[test]
    fn cursor_stream_maps_to_normalized_events_without_thinking() {
        let mut p = CursorParser::default();
        let ev: Vec<_> = fixture("cursor").lines().flat_map(|l| p.feed(l)).collect();
        let s = p.summary();
        assert_eq!(s.model_reported.as_deref(), Some("Auto"));
        assert_eq!(s.tool_calls, 2);
        assert!(s.turn_completed && s.failure.is_none());
        assert_eq!(s.usage.input_tokens, Some(12092));
        assert!(
            ev.iter()
                .filter(|e| matches!(e, EventPayload::ToolFinished { ok: true, .. }))
                .count()
                == 2
        );
        let json = serde_json::to_string(&ev).unwrap();
        assert!(
            !json.contains("Projeto de teste") && !json.contains("Não devo alterar"),
            "vazou: {json}"
        );
    }

    #[test]
    fn garbage_and_unknown_events_are_ignored() {
        for mut p in [
            Box::new(ClaudeParser::default()) as Box<dyn EngineParser>,
            Box::new(CursorParser::default()),
        ] {
            for l in ["", "não é json", "{\"type\":\"novo\"}", "[1]"] {
                assert!(p.feed(l).is_empty(), "{l}");
            }
        }
    }
}
