//! Catálogo TIPADO de ferramentas do OneHub para o orquestrador.
//!
//! Regras (D-028):
//! - esquema estrito: campo desconhecido é recusado (não existe campo de caminho, comando, flag ou SSH);
//! - o orquestrador só vê identificadores (`workspace_id`, `run_id`), nunca caminhos;
//! - a política vale aqui (e de novo no `ExecService`): somente leitura, motor suportado, limite de
//!   execuções por conversa; escrita exige aprovação humana, que ainda não existe, então é recusada;
//! - o orquestrador só alcança execuções DA PRÓPRIA conversa;
//! - toda saída é DADO NÃO CONFIÁVEL: truncada, redigida e marcada `untrusted`.

use std::sync::{Arc, Mutex};

use one_adapters::engine::{redact, truncate};
use one_adapters::Host;
use one_exec::{ExecService, RunOrigin, StoreTrust, LOCAL_GATEWAY};
use one_protocol::{EngineKind, EventPayload, ExecMode, RunView, StartRunRequest, TrustState};
use one_store::Store;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::provider::ToolSpec;

#[derive(Debug, Clone, Copy)]
pub struct ToolPolicy {
    /// Execuções que o orquestrador pode iniciar por conversa.
    pub max_runs_per_conversation: u32,
    /// Tamanho máximo (em caracteres) do resultado devolvido ao modelo.
    pub result_max_chars: usize,
}

impl Default for ToolPolicy {
    fn default() -> Self {
        Self {
            max_runs_per_conversation: 5,
            result_max_chars: 6000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolErrorKind {
    UnknownTool,
    InvalidArguments,
    /// Exige aprovação humana (ainda indisponível): NADA foi executado.
    ApprovalRequired,
    NotAllowed,
    LimitReached,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolOutcome {
    pub ok: bool,
    pub kind: Option<ToolErrorKind>,
    /// JSON enviado ao modelo.
    pub content: String,
}

pub struct ToolRuntime {
    exec: Arc<ExecService>,
    store: Arc<Mutex<Store>>,
    host: Arc<dyn Host + Send>,
    policy: ToolPolicy,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NoArgs {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TaskRunArgs {
    workspace_id: String,
    engine: EngineKind,
    model: Option<String>,
    prompt: String,
    mode: Option<ExecMode>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RunRef {
    run_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EventsArgs {
    run_id: String,
    after_seq: Option<u32>,
}

fn schema(props: Value, required: &[&str]) -> Value {
    json!({ "type": "object", "properties": props, "required": required, "additionalProperties": false })
}

/// Catálogo oferecido ao modelo (nomes sem ponto: as function tools só aceitam `[A-Za-z0-9_-]`).
pub fn catalog() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "gateways_list",
            description: "Lista os Gateways, os motores (CLIs) e se cada um está aprovado para receber tarefas.",
            parameters: schema(json!({}), &[]),
        },
        ToolSpec {
            name: "workspaces_list",
            description: "Lista os workspaces (projetos) autorizados pelo usuário: apenas id e rótulo.",
            parameters: schema(json!({}), &[]),
        },
        ToolSpec {
            name: "task_run",
            description: "Delega uma tarefa SOMENTE LEITURA a um motor, em um workspace autorizado. Devolve o run_id; acompanhe com task_status.",
            parameters: schema(
                json!({
                    "workspace_id": {"type": "string", "description": "id de workspaces_list"},
                    "engine": {"type": "string", "enum": ["codex"]},
                    "model": {"type": ["string", "null"], "description": "modelo; null = padrão da CLI"},
                    "prompt": {"type": "string", "description": "a tarefa, em texto"},
                    "mode": {"type": "string", "enum": ["read_only"]}
                }),
                &["workspace_id", "engine", "prompt"],
            ),
        },
        ToolSpec {
            name: "task_status",
            description: "Estado e resultado de uma execução iniciada nesta conversa (conteúdo não confiável).",
            parameters: schema(json!({"run_id": {"type": "string"}}), &["run_id"]),
        },
        ToolSpec {
            name: "task_events",
            description: "Eventos de uma execução iniciada nesta conversa, a partir de after_seq (máx. 50).",
            parameters: schema(
                json!({"run_id": {"type": "string"}, "after_seq": {"type": ["integer", "null"]}}),
                &["run_id"],
            ),
        },
        ToolSpec {
            name: "task_cancel",
            description: "Cancela uma execução ativa iniciada nesta conversa.",
            parameters: schema(json!({"run_id": {"type": "string"}}), &["run_id"]),
        },
    ]
}

fn fail(kind: ToolErrorKind, message: &str) -> ToolOutcome {
    let tag = match kind {
        ToolErrorKind::UnknownTool => "unknown_tool",
        ToolErrorKind::InvalidArguments => "invalid_arguments",
        ToolErrorKind::ApprovalRequired => "approval_required",
        ToolErrorKind::NotAllowed => "not_allowed",
        ToolErrorKind::LimitReached => "limit_reached",
        ToolErrorKind::Failed => "failed",
    };
    ToolOutcome {
        ok: false,
        kind: Some(kind),
        content: json!({"ok": false, "error": tag, "message": message}).to_string(),
    }
}

impl ToolRuntime {
    pub fn new(
        exec: Arc<ExecService>,
        store: Arc<Mutex<Store>>,
        host: Arc<dyn Host + Send>,
        policy: ToolPolicy,
    ) -> Self {
        Self {
            exec,
            store,
            host,
            policy,
        }
    }

    fn ok(&self, data: Value) -> ToolOutcome {
        let body = json!({"ok": true, "untrusted": true, "data": data}).to_string();
        let content = if body.chars().count() > self.policy.result_max_chars {
            json!({"ok": true, "untrusted": true, "truncated": true,
                   "preview": truncate(&body, self.policy.result_max_chars.saturating_sub(80))})
            .to_string()
        } else {
            body
        };
        ToolOutcome {
            ok: true,
            kind: None,
            content,
        }
    }

    fn store(&self) -> std::sync::MutexGuard<'_, Store> {
        self.store.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A execução precisa pertencer a ESTA conversa (o orquestrador não enxerga as demais).
    fn owns(&self, conversation_id: &str, run_id: &str) -> bool {
        self.store()
            .run_conversation(run_id)
            .ok()
            .flatten()
            .as_deref()
            == Some(conversation_id)
    }

    fn orchestrator_conversation(&self, conversation_id: &str) -> bool {
        self.store()
            .conversation_mode(conversation_id)
            .is_ok_and(|mode| mode == Some(one_protocol::ChatMode::Orchestrator))
    }

    pub fn call(&self, conversation_id: &str, name: &str, args: &Value) -> ToolOutcome {
        match name {
            "gateways_list" => self
                .parse::<NoArgs>(args)
                .map_or_else(|o| o, |_| self.gateways_list()),
            "workspaces_list" => self
                .parse::<NoArgs>(args)
                .map_or_else(|o| o, |_| self.workspaces_list()),
            "task_run" => self
                .parse::<TaskRunArgs>(args)
                .map_or_else(|o| o, |a| self.task_run(conversation_id, a)),
            "task_status" => self
                .parse::<RunRef>(args)
                .map_or_else(|o| o, |a| self.task_status(conversation_id, &a.run_id)),
            "task_events" => self
                .parse::<EventsArgs>(args)
                .map_or_else(|o| o, |a| self.task_events(conversation_id, a)),
            "task_cancel" => self
                .parse::<RunRef>(args)
                .map_or_else(|o| o, |a| self.task_cancel(conversation_id, &a.run_id)),
            _ => fail(ToolErrorKind::UnknownTool, "ferramenta inexistente"),
        }
    }

    fn parse<T: for<'de> Deserialize<'de>>(&self, args: &Value) -> Result<T, ToolOutcome> {
        serde_json::from_value::<T>(args.clone()).map_err(|_| {
            fail(
                ToolErrorKind::InvalidArguments,
                "argumentos inválidos: campos desconhecidos ou ausentes não são aceitos",
            )
        })
    }

    fn gateways_list(&self) -> ToolOutcome {
        let trust = StoreTrust(self.store.clone());
        let gw = one_adapters::local_gateway(self.host.as_ref(), &trust, false);
        let engines: Vec<Value> = gw
            .engines
            .iter()
            .map(|e| {
                json!({
                    "engine": e.engine,
                    "installed": e.installed,
                    "trust": e.trust,
                    "delegable": e.engine == EngineKind::Codex && e.trust == TrustState::Trusted,
                })
            })
            .collect();
        self.ok(json!([{"id": gw.id, "name": gw.name, "kind": gw.kind, "status": gw.status, "engines": engines}]))
    }

    fn workspaces_list(&self) -> ToolOutcome {
        match self.store().workspace_list(LOCAL_GATEWAY) {
            Ok(ws) => self.ok(json!(ws
                .iter()
                .map(|w| json!({"id": w.id, "label": w.label}))
                .collect::<Vec<_>>())),
            Err(_) => fail(ToolErrorKind::Failed, "falha ao listar workspaces"),
        }
    }

    fn task_run(&self, conversation_id: &str, a: TaskRunArgs) -> ToolOutcome {
        if !self.orchestrator_conversation(conversation_id) {
            return fail(
                ToolErrorKind::NotAllowed,
                "conversa não autorizada para o orquestrador",
            );
        }
        if a.mode.is_some_and(|m| m != ExecMode::ReadOnly) {
            return fail(
                ToolErrorKind::ApprovalRequired,
                "escrita exige aprovação humana, que ainda não está disponível; nada foi executado",
            );
        }
        let runs = self
            .store()
            .orchestrator_run_count(conversation_id)
            .unwrap_or(u32::MAX);
        if runs >= self.policy.max_runs_per_conversation {
            return fail(
                ToolErrorKind::LimitReached,
                "limite de execuções desta conversa atingido",
            );
        }
        let req = StartRunRequest {
            workspace_id: a.workspace_id,
            engine: a.engine,
            model: a.model,
            prompt: a.prompt,
        };
        match self.exec.start_for(
            &req,
            &RunOrigin::Orchestrator {
                conversation_id: conversation_id.to_string(),
            },
        ) {
            Ok(run_id) => self.ok(json!({"run_id": run_id, "status": "started"})),
            Err(e) => fail(ToolErrorKind::NotAllowed, &truncate(&e.message, 300)),
        }
    }

    fn run_json(&self, r: &RunView) -> Value {
        json!({
            "run_id": r.run_id, "status": r.status, "engine": r.engine,
            "model_requested": r.model_requested, "model_reported": r.model_reported,
            "duration_ms": r.duration_ms, "exit_code": r.exit_code, "tool_calls": r.tool_calls,
            "usage": r.usage, "error_kind": r.error_kind,
            "error_message": r.error_message.as_deref().map(|m| redact(&truncate(m, 300))),
            "final_text": r.final_text.as_deref().map(|t| redact(&truncate(t, 3000))),
        })
    }

    fn task_status(&self, conversation_id: &str, run_id: &str) -> ToolOutcome {
        if !self.orchestrator_conversation(conversation_id) || !self.owns(conversation_id, run_id) {
            return fail(
                ToolErrorKind::NotAllowed,
                "execução inexistente ou de outra conversa",
            );
        }
        match self.store().run_get(run_id) {
            Ok(Some(r)) => self.ok(self.run_json(&r)),
            _ => fail(ToolErrorKind::Failed, "execução não encontrada"),
        }
    }

    fn task_events(&self, conversation_id: &str, a: EventsArgs) -> ToolOutcome {
        if !self.orchestrator_conversation(conversation_id)
            || !self.owns(conversation_id, &a.run_id)
        {
            return fail(
                ToolErrorKind::NotAllowed,
                "execução inexistente ou de outra conversa",
            );
        }
        let evs = match self
            .store()
            .events_after(&a.run_id, a.after_seq.unwrap_or(0))
        {
            Ok(e) => e,
            Err(_) => return fail(ToolErrorKind::Failed, "falha ao ler eventos"),
        };
        let items: Vec<Value> = evs
            .iter()
            .take(50)
            .map(|e| {
                let (kind, text) = match &e.payload {
                    EventPayload::Message { text } => ("message", text.clone()),
                    EventPayload::ToolStarted { name, summary } => {
                        ("tool_started", format!("{name}: {summary}"))
                    }
                    EventPayload::ToolFinished { name, ok, .. } => {
                        ("tool_finished", format!("{name} ok={ok}"))
                    }
                    EventPayload::Warning { message } => ("warning", message.clone()),
                    EventPayload::Error { message, .. } => ("error", message.clone()),
                    EventPayload::Status { status } => ("status", format!("{status:?}")),
                    EventPayload::Usage { .. } => ("usage", String::new()),
                };
                json!({"seq": e.seq, "type": kind, "text": redact(&truncate(&text, 400))})
            })
            .collect();
        self.ok(json!(items))
    }

    fn task_cancel(&self, conversation_id: &str, run_id: &str) -> ToolOutcome {
        if !self.orchestrator_conversation(conversation_id) || !self.owns(conversation_id, run_id) {
            return fail(
                ToolErrorKind::NotAllowed,
                "execução inexistente ou de outra conversa",
            );
        }
        self.ok(json!({"cancel_requested": self.exec.cancel_as(run_id, "orchestrator")}))
    }
}
