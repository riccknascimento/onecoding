//! Workspaces autorizados, tarefas, execuções e eventos (Fase 2A).
//! Tudo que vem de fora (UI, executor) entra por parâmetros SQL: nenhuma concatenação de texto.

use one_protocol::{
    ChatMessage, ChatMode, ChatRole, ErrorKind, EventPayload, ExecMode, RunEvent, RunInitiator,
    RunStatus, RunView, UsageInfo, Workspace,
};
use rusqlite::{params, OptionalExtension, Row};
use serde::de::DeserializeOwned;

use crate::{Result, Store, StoreError};

/// Limites para não acumular logs excessivos.
const MAX_EVENTS_PER_RUN: i64 = 2000;
const MAX_PAYLOAD_BYTES: usize = 16 * 1024;

pub struct NewRun<'a> {
    pub workspace_id: &'a str,
    pub engine: &'a str,
    pub model: Option<&'a str>,
    pub mode: ExecMode,
    pub prompt: &'a str,
}

#[derive(Default)]
pub struct FinishInfo {
    pub status: Option<RunStatus>,
    pub exit_code: Option<i32>,
    pub error_kind: Option<ErrorKind>,
    pub error_message: Option<String>,
    pub duration_ms: Option<u32>,
    pub usage: UsageInfo,
    pub tool_calls: u32,
    pub engine_session_id: Option<String>,
    pub model_reported: Option<String>,
    pub final_text: Option<String>,
}

fn enum_str<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|x| x.as_str().map(str::to_string))
        .unwrap_or_default()
}

fn parse_enum<T: DeserializeOwned>(s: &str) -> rusqlite::Result<T> {
    serde_json::from_value(serde_json::Value::String(s.to_string())).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })
}

const RUN_VIEW_SQL: &str = "SELECT r.id, r.task_id, t.gateway_id, t.project_id, p.name, t.engine, t.model, r.model_reported,
    t.mode, t.prompt, r.status, t.created_at, r.started_at, r.finished_at, r.duration_ms, r.exit_code,
    r.engine_session_id, r.input_tokens, r.output_tokens, r.cached_tokens, r.reasoning_tokens,
    r.tool_calls, r.attempts, r.error_kind, r.error_message, r.final_text, t.initiator
  FROM runs r JOIN tasks t ON t.id = r.task_id JOIN projects p ON p.id = t.project_id";

fn run_view(r: &Row<'_>) -> rusqlite::Result<RunView> {
    let opt_enum = |i: usize| -> rusqlite::Result<Option<ErrorKind>> {
        r.get::<_, Option<String>>(i)?
            .map(|s| parse_enum(&s))
            .transpose()
    };
    Ok(RunView {
        run_id: r.get(0)?,
        task_id: r.get(1)?,
        gateway_id: r.get(2)?,
        workspace_id: r.get(3)?,
        workspace_label: r.get(4)?,
        engine: parse_enum(&r.get::<_, String>(5)?)?,
        model_requested: r.get(6)?,
        model_reported: r.get(7)?,
        mode: parse_enum(&r.get::<_, String>(8)?)?,
        prompt: r.get(9)?,
        status: parse_enum(&r.get::<_, String>(10)?)?,
        created_at: r.get(11)?,
        started_at: r.get(12)?,
        finished_at: r.get(13)?,
        duration_ms: r.get(14)?,
        exit_code: r.get(15)?,
        engine_session_id: r.get(16)?,
        usage: UsageInfo {
            input_tokens: r.get(17)?,
            output_tokens: r.get(18)?,
            cached_tokens: r.get(19)?,
            reasoning_tokens: r.get(20)?,
        },
        tool_calls: r.get(21)?,
        attempts: r.get(22)?,
        error_kind: opt_enum(23)?,
        error_message: r.get(24)?,
        final_text: r.get(25)?,
        initiator: parse_enum(&r.get::<_, String>(26)?)?,
    })
}

impl Store {
    // ───────── conversas do orquestrador ─────────

    pub fn conversation_create(&self, mode: ChatMode, title: &str) -> Result<String> {
        let t: String = title.chars().take(60).collect();
        self.conn.execute(
            "INSERT INTO conversations(id, title, mode) VALUES ('cv_' || lower(hex(randomblob(8))), ?1, ?2)",
            params![t, enum_str(&mode)],
        )?;
        Ok(self.conn.query_row(
            "SELECT id FROM conversations WHERE rowid = last_insert_rowid()",
            [],
            |r| r.get(0),
        )?)
    }

    pub fn conversation_mode(&self, id: &str) -> Result<Option<ChatMode>> {
        let m: Option<String> = self
            .conn
            .query_row("SELECT mode FROM conversations WHERE id = ?1", [id], |r| {
                r.get(0)
            })
            .optional()?;
        Ok(m.map(|s| parse_enum(&s)).transpose()?)
    }

    pub fn message_append(&self, conversation_id: &str, role: ChatRole, text: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO messages(id, conversation_id, role, body) VALUES ('ms_' || lower(hex(randomblob(8))), ?1, ?2, ?3)",
            params![conversation_id, enum_str(&role), text],
        )?;
        Ok(())
    }

    pub fn messages_list(&self, conversation_id: &str) -> Result<Vec<ChatMessage>> {
        let mut st = self
            .conn
            .prepare("SELECT role, body, created_at FROM messages WHERE conversation_id = ?1 AND role IN ('user','orchestrator') ORDER BY rowid")?;
        let rows = st.query_map([conversation_id], |r| {
            Ok(ChatMessage {
                role: parse_enum(&r.get::<_, String>(0)?)?,
                text: r.get(1)?,
                ts: r.get(2)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Registra uma chamada de ferramenta, resultado, limite ou erro do provedor (texto já truncado e redigido).
    pub fn orch_event_append(
        &self,
        conversation_id: &str,
        kind: &str,
        payload: &str,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO orchestrator_events(conversation_id, seq, kind, payload)
             VALUES (?1, (SELECT coalesce(max(seq),0)+1 FROM orchestrator_events WHERE conversation_id = ?1), ?2, ?3)",
            params![conversation_id, kind, payload],
        )?;
        Ok(())
    }

    pub fn orch_events(&self, conversation_id: &str) -> Result<Vec<(String, String)>> {
        let mut st = self.conn.prepare(
            "SELECT kind, payload FROM orchestrator_events WHERE conversation_id = ?1 ORDER BY seq",
        )?;
        let rows = st.query_map([conversation_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Conversa à qual a execução pertence (para checar que o orquestrador só vê as suas).
    pub fn run_conversation(&self, run_id: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT t.conversation_id FROM runs r JOIN tasks t ON t.id = r.task_id WHERE r.id = ?1",
                [run_id],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten())
    }

    /// Quantas execuções o orquestrador já iniciou nesta conversa.
    pub fn orchestrator_run_count(&self, conversation_id: &str) -> Result<u32> {
        Ok(self.conn.query_row(
            "SELECT count(*) FROM tasks WHERE conversation_id = ?1 AND initiator = 'orchestrator'",
            [conversation_id],
            |r| r.get(0),
        )?)
    }

    /// Trilha de auditoria (append-only). `subject` não deve conter segredos.
    pub fn audit_append(
        &self,
        actor: &str,
        action: &str,
        subject: &str,
        outcome: &str,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO audit_log(actor, action, subject, outcome) VALUES (?1, ?2, ?3, ?4)",
            params![actor, action, subject, outcome],
        )?;
        Ok(())
    }

    /// O Gateway local existe sempre (FK de projetos e tarefas).
    pub fn ensure_local_gateway(&self) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO gateways(id, name, kind) VALUES ('local', 'Este Mac', 'local')",
            [],
        )?;
        Ok(())
    }

    /// Registra um workspace autorizado (caminho já canônico e validado pelo chamador). Idempotente.
    pub fn workspace_add(
        &self,
        gateway_id: &str,
        canonical_path: &str,
        label: &str,
    ) -> Result<Workspace> {
        self.conn.execute(
            "INSERT OR IGNORE INTO projects(id, name, gateway_id, path) VALUES ('ws_' || lower(hex(randomblob(8))), ?1, ?2, ?3)",
            params![label, gateway_id, canonical_path],
        )?;
        let w = self.conn.query_row(
            "SELECT id, gateway_id, name, path FROM projects WHERE gateway_id = ?1 AND path = ?2",
            params![gateway_id, canonical_path],
            |r| {
                Ok(Workspace {
                    id: r.get(0)?,
                    gateway_id: r.get(1)?,
                    label: r.get(2)?,
                    path: r.get(3)?,
                })
            },
        )?;
        Ok(w)
    }

    pub fn workspace_get(&self, id: &str) -> Result<Option<Workspace>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, gateway_id, name, path FROM projects WHERE id = ?1",
                [id],
                |r| {
                    Ok(Workspace {
                        id: r.get(0)?,
                        gateway_id: r.get(1)?,
                        label: r.get(2)?,
                        path: r.get(3)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn workspace_list(&self, gateway_id: &str) -> Result<Vec<Workspace>> {
        let mut st = self
            .conn
            .prepare("SELECT id, gateway_id, name, path FROM projects WHERE gateway_id = ?1 ORDER BY name, path")?;
        let rows = st.query_map([gateway_id], |r| {
            Ok(Workspace {
                id: r.get(0)?,
                gateway_id: r.get(1)?,
                label: r.get(2)?,
                path: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Cria conversa, tarefa e execução (`queued`) do Modo direto (iniciador: usuário).
    pub fn run_create(&self, n: &NewRun<'_>) -> Result<String> {
        self.run_create_for(n, None, RunInitiator::User)
    }

    /// Cria tarefa e execução (`queued`) numa transação. Com `conversation_id`, a tarefa entra nessa
    /// conversa (ex.: a do orquestrador); sem ele, cria uma conversa do Modo direto.
    pub fn run_create_for(
        &self,
        n: &NewRun<'_>,
        conversation_id: Option<&str>,
        initiator: RunInitiator,
    ) -> Result<String> {
        let ws = self
            .workspace_get(n.workspace_id)?
            .ok_or_else(|| StoreError::Integrity("workspace inexistente".into()))?;
        let title: String = n.prompt.chars().take(60).collect();
        let tx = self.conn.unchecked_transaction()?;
        let conv: String = match conversation_id {
            Some(c) => c.to_string(),
            None => {
                tx.execute(
                    "INSERT INTO conversations(id, project_id, title) VALUES ('cv_' || lower(hex(randomblob(8))), ?1, ?2)",
                    params![ws.id, title],
                )?;
                tx.query_row(
                    "SELECT id FROM conversations WHERE rowid = last_insert_rowid()",
                    [],
                    |r| r.get(0),
                )?
            }
        };
        tx.execute(
            "INSERT INTO tasks(id, conversation_id, gateway_id, project_id, engine, model, mode, prompt, initiator)
             VALUES ('tk_' || lower(hex(randomblob(8))), ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![conv, ws.gateway_id, ws.id, n.engine, n.model, enum_str(&n.mode), n.prompt, enum_str(&initiator)],
        )?;
        let task: String = tx.query_row(
            "SELECT id FROM tasks WHERE rowid = last_insert_rowid()",
            [],
            |r| r.get(0),
        )?;
        tx.execute(
            "INSERT INTO runs(id, task_id, status) VALUES ('rn_' || lower(hex(randomblob(8))), ?1, 'queued')",
            [&task],
        )?;
        let run: String = tx.query_row(
            "SELECT id FROM runs WHERE rowid = last_insert_rowid()",
            [],
            |r| r.get(0),
        )?;
        tx.commit()?;
        Ok(run)
    }

    pub fn run_mark_running(&self, run_id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE runs SET status = 'running', started_at = strftime('%Y-%m-%dT%H:%M:%SZ','now') WHERE id = ?1 AND status = 'queued'",
            [run_id],
        )?;
        Ok(())
    }

    /// Acrescenta um evento (seq automático). `None` quando o limite por execução já foi atingido.
    pub fn event_append(&self, run_id: &str, payload: &EventPayload) -> Result<Option<RunEvent>> {
        let json =
            serde_json::to_string(payload).map_err(|e| StoreError::Integrity(e.to_string()))?;
        if json.len() > MAX_PAYLOAD_BYTES {
            return Err(StoreError::Integrity(
                "evento acima do limite de tamanho".into(),
            ));
        }
        let tx = self.conn.unchecked_transaction()?;
        let seq: i64 = tx.query_row(
            "SELECT coalesce(max(seq), 0) FROM run_events WHERE run_id = ?1",
            [run_id],
            |r| r.get(0),
        )?;
        if seq >= MAX_EVENTS_PER_RUN {
            return Ok(None);
        }
        tx.execute(
            "INSERT INTO run_events(run_id, ts, kind, payload, seq) VALUES (?1, strftime('%Y-%m-%dT%H:%M:%SZ','now'), ?2, ?3, ?4)",
            params![run_id, event_kind(payload), json, seq + 1],
        )?;
        let ts: String = tx.query_row(
            "SELECT ts FROM run_events WHERE rowid = last_insert_rowid()",
            [],
            |r| r.get(0),
        )?;
        tx.commit()?;
        Ok(Some(RunEvent {
            run_id: run_id.into(),
            seq: (seq + 1) as u32,
            ts,
            payload: payload.clone(),
        }))
    }

    /// Finaliza a execução. Só atualiza se ainda estiver ativa (não sobrescreve um estado final).
    pub fn run_finish(&self, run_id: &str, f: &FinishInfo) -> Result<()> {
        self.conn.execute(
            "UPDATE runs SET status = ?2, finished_at = strftime('%Y-%m-%dT%H:%M:%SZ','now'), exit_code = ?3,
               error_kind = ?4, error_message = ?5, duration_ms = ?6, input_tokens = ?7, output_tokens = ?8,
               cached_tokens = ?9, reasoning_tokens = ?10, tool_calls = ?11, engine_session_id = ?12,
               model_reported = ?13, final_text = ?14
             WHERE id = ?1 AND status IN ('queued','running','awaiting_approval')",
            params![
                run_id,
                enum_str(&f.status.unwrap_or(RunStatus::Failed)),
                f.exit_code,
                f.error_kind.map(|k| enum_str(&k)),
                f.error_message,
                f.duration_ms,
                f.usage.input_tokens,
                f.usage.output_tokens,
                f.usage.cached_tokens,
                f.usage.reasoning_tokens,
                f.tool_calls,
                f.engine_session_id,
                f.model_reported,
                f.final_text,
            ],
        )?;
        Ok(())
    }

    pub fn run_get(&self, run_id: &str) -> Result<Option<RunView>> {
        Ok(self
            .conn
            .query_row(
                &format!("{RUN_VIEW_SQL} WHERE r.id = ?1"),
                [run_id],
                run_view,
            )
            .optional()?)
    }

    pub fn runs_list(&self, limit: u32) -> Result<Vec<RunView>> {
        let mut st = self
            .conn
            .prepare(&format!("{RUN_VIEW_SQL} ORDER BY r.rowid DESC LIMIT ?1"))?;
        let rows = st.query_map([limit.min(200)], run_view)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn events_after(&self, run_id: &str, after_seq: u32) -> Result<Vec<RunEvent>> {
        let mut st = self
            .conn
            .prepare("SELECT seq, ts, payload FROM run_events WHERE run_id = ?1 AND seq > ?2 ORDER BY seq LIMIT 500")?;
        let rows = st.query_map(params![run_id, after_seq], |r| {
            let json: String = r.get(2)?;
            let payload: EventPayload = serde_json::from_str(&json).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    2,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?;
            Ok(RunEvent {
                run_id: run_id.into(),
                seq: r.get(0)?,
                ts: r.get(1)?,
                payload,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Na abertura do app: execuções que ficaram ativas (app fechado ou caiu) viram `failed/interrupted`.
    pub fn reconcile_interrupted(&self) -> Result<usize> {
        Ok(self.conn.execute(
            "UPDATE runs SET status = 'failed', error_kind = 'interrupted',
               error_message = 'O app foi fechado durante a execução.',
               finished_at = strftime('%Y-%m-%dT%H:%M:%SZ','now')
             WHERE status IN ('queued','running','awaiting_approval')",
            [],
        )?)
    }
}

fn event_kind(p: &EventPayload) -> &'static str {
    match p {
        EventPayload::Status { .. } => "status",
        EventPayload::Message { .. } => "message",
        EventPayload::ToolStarted { .. } => "tool_started",
        EventPayload::ToolFinished { .. } => "tool_finished",
        EventPayload::Usage { .. } => "usage",
        EventPayload::Warning { .. } => "warning",
        EventPayload::Error { .. } => "error",
    }
}
