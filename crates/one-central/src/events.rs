//! Tempo real do OneHub: log de eventos por conversa, snapshot inicial e stream SSE.
//!
//! Regras:
//! - Cada mudança de estado grava o próprio evento na MESMA transação. Sem evento órfão e sem estado sem evento.
//! - A sequência sai de `conversations.last_event_seq` sob o lock da conversa; o stream só entrega números
//!   já confirmados, então quem reconecta com `Last-Event-ID` nunca perde nem repete evento.
//! - O stream revalida a sessão e a concessão OneNode × projeto a cada ciclo: revogação encerra a conexão.
//! - O estado do navegador é reconstruído por upsert (mensagem por id, job por id, evento por seq), então
//!   um evento repetido não duplica nada.

use std::collections::VecDeque;
use std::convert::Infallible;
use std::time::{Duration, Instant};

use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::response::sse::{Event as SseEvent, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use deadpool_postgres::GenericClient;
use futures_util::stream;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::routes::{db, owned_conversation, ApiError, AppState, AuthUser};

/// Eventos enviados por consulta ao banco.
const BATCH: i64 = 200;
/// Janela de eventos devolvida no snapshot inicial (sufixo do log).
const SNAPSHOT_EVENTS: i64 = 2000;
const SNAPSHOT_MESSAGES: i64 = 1000;
const POLL_EVERY: Duration = Duration::from_secs(1);
const PING_EVERY: Duration = Duration::from_secs(10);

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/conversations/{id}/state", get(snapshot))
        .route("/v1/conversations/{id}/events", get(stream_events))
}

/// Grava um evento da conversa. Chamar sempre dentro da transação da mudança de estado.
pub async fn emit<C: GenericClient>(
    c: &C,
    conv: Uuid,
    kind: &str,
    payload: Value,
) -> Result<i64, ApiError> {
    let seq: i64 = c
        .query_one(
            "UPDATE conversations SET last_event_seq = last_event_seq + 1 WHERE id = $1 RETURNING last_event_seq",
            &[&conv],
        )
        .await?
        .get(0);
    c.execute(
        "INSERT INTO conversation_events(conversation_id, seq, kind, payload) VALUES ($1, $2, $3, $4)",
        &[&conv, &seq, &kind, &payload],
    )
    .await?;
    Ok(seq)
}

/// Emite o estado atual de um job (status, erro e pedido de cancelamento).
pub async fn emit_job<C: GenericClient>(c: &C, job: Uuid) -> Result<(), ApiError> {
    let r = c
        .query_one(
            "SELECT conversation_id, status, error, cancel_requested_at IS NOT NULL FROM jobs WHERE id = $1",
            &[&job],
        )
        .await?;
    emit(
        c,
        r.get(0),
        "job.status",
        json!({
            "job_id": job,
            "status": r.get::<_, String>(1),
            "error": r.get::<_, Option<String>>(2),
            "cancel_requested": r.get::<_, bool>(3),
        }),
    )
    .await?;
    Ok(())
}

/// Emite o estado atual de uma tarefa, quando ela pertence a uma conversa.
pub async fn emit_task<C: GenericClient>(c: &C, task: Uuid) -> Result<(), ApiError> {
    if let Some(r) = c
        .query_opt(
            "SELECT conversation_id, status, tool, job_id FROM tasks WHERE id = $1",
            &[&task],
        )
        .await?
    {
        if let Some(conv) = r.get::<_, Option<Uuid>>(0) {
            emit(
                c,
                conv,
                "task.status",
                json!({
                    "task_id": task,
                    "status": r.get::<_, String>(1),
                    "tool": r.get::<_, Option<String>>(2),
                    "job_id": r.get::<_, Option<Uuid>>(3),
                }),
            )
            .await?;
        }
    }
    Ok(())
}

/// Atualiza a mensagem do assistente que anunciou o job quando ele termina, e emite a mudança.
/// Estados em andamento (queued/running) não mexem na mensagem.
pub async fn sync_job_message<C: GenericClient>(c: &C, job: Uuid) -> Result<(), ApiError> {
    let r = c
        .query_one(
            "SELECT status, answer, error FROM jobs WHERE id = $1",
            &[&job],
        )
        .await?;
    let (body, state): (String, &str) = match r.get::<_, String>(0).as_str() {
        "succeeded" => (
            r.get::<_, Option<String>>(1).unwrap_or_default(),
            "completed",
        ),
        "failed" => (
            r.get::<_, Option<String>>(2)
                .unwrap_or_else(|| "A análise falhou.".into()),
            "failed",
        ),
        "cancelled" => ("Análise cancelada.".into(), "cancelled"),
        _ => return Ok(()),
    };
    if let Some(row) = c
        .query_opt(
            "UPDATE messages SET body = $2, state = $3 WHERE job_id = $1 AND role = 'assistant' RETURNING id, conversation_id",
            &[&job, &body, &state],
        )
        .await?
    {
        emit(
            c,
            row.get(1),
            "message.updated",
            json!({ "id": row.get::<_, Uuid>(0), "body": body, "state": state }),
        )
        .await?;
    }
    Ok(())
}

fn message_payload(r: &tokio_postgres::Row) -> Value {
    json!({
        "id": r.get::<_, Uuid>(0),
        "role": r.get::<_, String>(1),
        "body": r.get::<_, String>(2),
        "created_at": r.get::<_, DateTime<Utc>>(3),
        "state": r.get::<_, String>(4),
        "task_id": r.get::<_, Option<Uuid>>(5),
        "job_id": r.get::<_, Option<Uuid>>(6),
    })
}

/// Snapshot inicial. A ordem importa: a sequência é lida ANTES dos dados. Assim, qualquer mudança que
/// já apareça nos dados e tenha sequência maior é reentregue pelo stream (upsert torna isso inofensivo).
async fn snapshot(
    State(s): State<AppState>,
    user: AuthUser,
    Path(conv): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    let client = db(&s).await?;
    owned_conversation(&client, &user, conv).await?;
    let last_seq: i64 = client
        .query_one(
            "SELECT last_event_seq FROM conversations WHERE id = $1",
            &[&conv],
        )
        .await?
        .get(0);
    let mut messages: Vec<Value> = client
        .query(
            "SELECT id, role, body, created_at, state, task_id, job_id FROM messages \
             WHERE conversation_id = $1 ORDER BY created_at DESC, id DESC LIMIT $2",
            &[&conv, &SNAPSHOT_MESSAGES],
        )
        .await?
        .iter()
        .map(message_payload)
        .collect();
    messages.reverse();
    let mut events: Vec<Value> = client
        .query(
            "SELECT seq, kind, payload, created_at FROM conversation_events \
             WHERE conversation_id = $1 AND seq <= $2 ORDER BY seq DESC LIMIT $3",
            &[&conv, &last_seq, &SNAPSHOT_EVENTS],
        )
        .await?
        .iter()
        .map(|r| {
            json!({
                "seq": r.get::<_, i64>(0),
                "kind": r.get::<_, String>(1),
                "payload": r.get::<_, Value>(2),
                "created_at": r.get::<_, DateTime<Utc>>(3),
            })
        })
        .collect();
    events.reverse();
    let jobs: Vec<Value> = client
        .query(
            "SELECT id, status, question, error, cancel_requested_at IS NOT NULL FROM jobs \
             WHERE conversation_id = $1 ORDER BY created_at DESC LIMIT 50",
            &[&conv],
        )
        .await?
        .iter()
        .map(|r| {
            json!({
                "id": r.get::<_, Uuid>(0),
                "status": r.get::<_, String>(1),
                "question": r.get::<_, String>(2),
                "error": r.get::<_, Option<String>>(3),
                "cancel_requested": r.get::<_, bool>(4),
            })
        })
        .collect();
    let tasks: Vec<Value> = client
        .query(
            "SELECT id, status, tool, job_id FROM tasks WHERE conversation_id = $1 ORDER BY created_at DESC LIMIT 50",
            &[&conv],
        )
        .await?
        .iter()
        .map(|r| {
            json!({
                "id": r.get::<_, Uuid>(0),
                "status": r.get::<_, String>(1),
                "tool": r.get::<_, Option<String>>(2),
                "job_id": r.get::<_, Option<Uuid>>(3),
            })
        })
        .collect();
    Ok(Json(json!({
        "last_seq": last_seq,
        "messages": messages,
        "events": events,
        "jobs": jobs,
        "tasks": tasks,
    })))
}

#[derive(Deserialize)]
pub struct StreamQuery {
    /// Usado só na primeira conexão (o navegador ainda não tem `Last-Event-ID`).
    pub after: Option<i64>,
}

/// Estado de uma conexão SSE. Não guarda conexão com o banco: cada ciclo pega a sua e a devolve.
struct Feed {
    state: AppState,
    user_id: Uuid,
    session: Vec<u8>,
    conv: Uuid,
    cursor: i64,
    queue: VecDeque<(i64, String, Value)>,
    last_ping: Instant,
    closed: bool,
}

enum Cycle {
    Rows(Vec<(i64, String, Value)>),
    Idle,
    /// Acesso encerrado (sessão, conversa ou concessão). A conexão termina com aviso.
    Ended(&'static str),
}

/// Acesso ainda válido: sessão, conversa do dono e concessão OneNode × projeto.
async fn access_ended(f: &Feed) -> Result<Option<&'static str>, ApiError> {
    let client = db(&f.state).await?;
    let session_ok = client
        .query_opt(
            "SELECT 1 FROM sessions s JOIN users u ON u.id = s.user_id \
             WHERE s.token_hash = $1 AND s.user_id = $2 AND s.revoked_at IS NULL AND s.expires_at > now() AND NOT u.disabled",
            &[&f.session, &f.user_id],
        )
        .await?
        .is_some();
    if !session_ok {
        return Ok(Some("session_ended"));
    }
    let scope = client
        .query_opt(
            "SELECT node_id, project_id FROM conversations WHERE id = $1 AND owner_id = $2",
            &[&f.conv, &f.user_id],
        )
        .await?;
    let Some(scope) = scope else {
        return Ok(Some("access_ended"));
    };
    if let (Some(n), Some(p)) = (
        scope.get::<_, Option<Uuid>>(0),
        scope.get::<_, Option<Uuid>>(1),
    ) {
        let active = client
            .query_opt(
                "SELECT 1 FROM node_project_grants g JOIN onenodes o ON o.id = g.node_id \
                 WHERE g.node_id = $1 AND g.project_id = $2 AND g.revoked_at IS NULL AND o.revoked_at IS NULL",
                &[&n, &p],
            )
            .await?
            .is_some();
        if !active {
            return Ok(Some("grant_revoked"));
        }
    }
    Ok(None)
}

async fn cycle(f: &Feed) -> Result<Cycle, ApiError> {
    if let Some(reason) = access_ended(f).await? {
        return Ok(Cycle::Ended(reason));
    }
    let client = db(&f.state).await?;
    let rows = client
        .query(
            "SELECT seq, kind, payload FROM conversation_events WHERE conversation_id = $1 AND seq > $2 ORDER BY seq LIMIT $3",
            &[&f.conv, &f.cursor, &BATCH],
        )
        .await?;
    if rows.is_empty() {
        return Ok(Cycle::Idle);
    }
    Ok(Cycle::Rows(
        rows.iter()
            .map(|r| (r.get(0), r.get(1), r.get(2)))
            .collect(),
    ))
}

fn event_of((seq, kind, payload): (i64, String, Value)) -> SseEvent {
    SseEvent::default()
        .id(seq.to_string())
        .event(kind)
        .data(payload.to_string())
}

/// Uma próxima mensagem do stream. Sem eventos novos, espera um ciclo e emite `ping` quando vencer o prazo.
async fn next_message(mut f: Feed) -> Option<(Result<SseEvent, Infallible>, Feed)> {
    loop {
        if f.closed {
            return None;
        }
        if let Some(row) = f.queue.pop_front() {
            // revalida antes de cada entrega: uma revogação no meio do lote não deixa eventos passarem
            match access_ended(&f).await {
                Ok(None) => {
                    f.cursor = row.0;
                    return Some((Ok(event_of(row)), f));
                }
                Ok(Some(reason)) => {
                    f.closed = true;
                    f.queue.clear();
                    let ev = SseEvent::default()
                        .event("closed")
                        .data(json!({ "reason": reason }).to_string());
                    return Some((Ok(ev), f));
                }
                Err(_) => {
                    // falha transitória: devolve o evento à fila e tenta de novo
                    f.queue.push_front(row);
                    tokio::time::sleep(POLL_EVERY).await;
                    continue;
                }
            }
        }
        match cycle(&f).await {
            Ok(Cycle::Rows(rows)) => f.queue.extend(rows),
            Ok(Cycle::Ended(reason)) => {
                f.closed = true;
                let ev = SseEvent::default()
                    .event("closed")
                    .data(json!({ "reason": reason }).to_string());
                return Some((Ok(ev), f));
            }
            // Falha transitória do banco: não derruba a conexão; tenta de novo no próximo ciclo.
            Ok(Cycle::Idle) | Err(_) => {}
        }
        if f.queue.is_empty() {
            if f.last_ping.elapsed() >= PING_EVERY {
                f.last_ping = Instant::now();
                return Some((Ok(SseEvent::default().event("ping").data("1")), f));
            }
            tokio::time::sleep(POLL_EVERY).await;
        }
    }
}

async fn stream_events(
    State(s): State<AppState>,
    user: AuthUser,
    Path(conv): Path<Uuid>,
    Query(q): Query<StreamQuery>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let client = db(&s).await?;
    owned_conversation(&client, &user, conv).await?;
    let scope = client
        .query_one(
            "SELECT node_id, project_id, last_event_seq FROM conversations WHERE id = $1",
            &[&conv],
        )
        .await?;
    if let (Some(n), Some(p)) = (
        scope.get::<_, Option<Uuid>>(0),
        scope.get::<_, Option<Uuid>>(1),
    ) {
        crate::routes::check_grant(&client, n, p).await?;
    }
    let cursor = headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<i64>().ok())
        .or(q.after)
        .unwrap_or_else(|| scope.get::<_, i64>(2))
        // cursor acima da sequência atual (adulterado ou de outra conversa) não pode pular eventos
        .clamp(0, scope.get::<_, i64>(2));
    drop(client);
    let feed = Feed {
        state: s,
        user_id: user.id,
        session: user.session,
        conv,
        cursor,
        queue: VecDeque::new(),
        last_ping: Instant::now(),
        closed: false,
    };
    Ok(Sse::new(stream::unfold(feed, next_message)).into_response())
}
