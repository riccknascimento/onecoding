//! OneExecutor no Central: registro de executores, jobs de análise com lease e ferramentas remotas
//! pedidas pelo executor, sempre dentro do OneNode e do projeto do job. O executor roda fora do Central
//! (hoje no Mac, com o Codex autenticado lá). O Central nunca recebe credenciais de IA.

use axum::extract::{FromRequestParts, Path, State};
use serde::{Deserialize, Serialize};
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use uuid::Uuid;

use deadpool_postgres::GenericClient;

use crate::auth::{new_secret, sha256};
use crate::events::{emit, emit_job, emit_task, sync_job_message};
use crate::routes::{
    audit, bearer, check_grant, db, owned_conversation, require_admin, text_len_ok, validate_tool_args, ApiError,
    AppState, AuthUser, Db,
};
use one_registry::agent::{AgentRegistry, KNOWN_ENGINES};
use one_registry::delegation::{EngineState, Environment, Status};
use one_registry::orchestrate;
use one_registry::entry::Permission;

pub const JOB_LEASE_SECONDS: f64 = 600.0;
pub const EXECUTOR_ONLINE_WINDOW: i64 = 90;
/// Pedidos de análise em andamento por usuário (evita laços e fila sem limite).
pub const MAX_OPEN_JOBS_PER_USER: i64 = 5;
/// Ferramentas remotas que um mesmo job pode pedir.
pub const MAX_TOOL_REQUESTS_PER_JOB: i64 = 40;
const MAX_EVENT_BYTES: usize = 16 * 1024;
const MAX_ANSWER_BYTES: usize = 64 * 1024;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/executors", get(list_executors).post(create_executor))
        .route("/v1/executors/{id}/revoke", post(revoke_executor))
        .route("/v1/jobs/{id}", get(get_job))
        .route("/v1/jobs/{id}/cancel", post(cancel_job))
        .route("/v1/delegations", post(create_delegation))
        .route("/executor/v1/pair", post(pair_executor))
        .route("/executor/v1/heartbeat", post(heartbeat))
        .route("/executor/v1/jobs/next", post(next_job))
        .route("/executor/v1/jobs/{id}/tools", post(request_tool))
        .route("/executor/v1/jobs/{id}/tasks/{task}", get(tool_status))
        .route("/executor/v1/jobs/{id}/events", post(job_event))
        .route("/executor/v1/jobs/{id}/heartbeat", post(job_heartbeat))
        .route("/executor/v1/jobs/{id}/finish", post(finish_job))
}

/// Executor autenticado pelo próprio token (emitido no pareamento). Revogado = recusado.
pub struct ExecutorAuth {
    pub id: Uuid,
}

impl FromRequestParts<AppState> for ExecutorAuth {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let token = bearer(parts).ok_or(ApiError::Unauthorized)?;
        let client = db(state).await?;
        let row = client
            .query_opt(
                "SELECT id FROM executors WHERE token_hash = $1 AND revoked_at IS NULL",
                &[&sha256(&token)],
            )
            .await?
            .ok_or(ApiError::Unauthorized)?;
        Ok(ExecutorAuth { id: row.get(0) })
    }
}

fn parse_uuid(v: Option<&Value>) -> Option<Uuid> {
    v.and_then(Value::as_str)
        .and_then(|s| Uuid::parse_str(s).ok())
}

async fn create_executor(
    State(s): State<AppState>,
    admin: AuthUser,
    Json(b): Json<Value>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    require_admin(&admin)?;
    let name = b
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if !text_len_ok(&name, 1, 120) {
        return Err(ApiError::BadRequest("invalid_name"));
    }
    let code = new_secret();
    let client = db(&s).await?;
    let row = client
        .query_one(
            "INSERT INTO executors(name, pairing_hash, pairing_expires) VALUES ($1, $2, now() + interval '15 minutes') RETURNING id, pairing_expires",
            &[&name, &sha256(&code)],
        )
        .await?;
    let id: Uuid = row.get(0);
    audit(
        &client,
        &admin.email,
        "executor_create",
        &id.to_string(),
        "ok",
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(
            json!({ "id": id, "pairing_code": code, "pairing_expires_at": row.get::<_, DateTime<Utc>>(1) }),
        ),
    ))
}

async fn pair_executor(
    State(s): State<AppState>,
    Json(b): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let id = parse_uuid(b.get("executor_id")).ok_or(ApiError::BadRequest("invalid_executor_id"))?;
    let code = b.get("code").and_then(Value::as_str).unwrap_or("");
    let token = new_secret();
    let client = db(&s).await?;
    let row = client
        .query_opt(
            "UPDATE executors SET pairing_hash = NULL, pairing_expires = NULL, token_hash = $3, paired_at = now(), last_seen_at = now() \
             WHERE id = $1 AND pairing_hash = $2 AND pairing_expires > now() AND revoked_at IS NULL RETURNING id",
            &[&id, &sha256(code), &sha256(&token)],
        )
        .await?
        .ok_or(ApiError::Unauthorized)?;
    let eid: Uuid = row.get(0);
    audit(&client, "executor", "pair", &eid.to_string(), "ok").await?;
    Ok(Json(json!({ "executor_id": eid, "token": token })))
}

/// Estado de um motor, reportado pelo próprio executor. Só estado: nunca credencial, token ou caminho.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EngineReport {
    pub name: String,
    pub installed: bool,
    pub authenticated: bool,
    pub approved: bool,
    #[serde(default)]
    pub version: Option<String>,
}

const MAX_ENGINE_REPORTS: usize = 8;

fn valid_reports(reports: &[EngineReport]) -> bool {
    reports.len() <= MAX_ENGINE_REPORTS
        && reports.iter().all(|r| {
            KNOWN_ENGINES.contains(&r.name.as_str())
                && r.version.as_ref().is_none_or(|v| v.len() <= 40 && v.chars().all(|c| c.is_ascii_graphic() || c == ' '))
        })
}

async fn heartbeat(
    State(s): State<AppState>,
    ex: ExecutorAuth,
    report: Option<Json<Vec<EngineReport>>>,
) -> Result<Json<Value>, ApiError> {
    let client = db(&s).await?;
    match report {
        Some(Json(reports)) => {
            if !valid_reports(&reports) {
                return Err(ApiError::BadRequest("engines_invalid"));
            }
            let body = serde_json::to_value(&reports).map_err(|_| ApiError::Internal)?;
            client
                .execute(
                    "UPDATE executors SET last_seen_at = now(), engines = $2, engines_reported_at = now() WHERE id = $1",
                    &[&ex.id, &body],
                )
                .await?;
        }
        None => {
            client
                .execute("UPDATE executors SET last_seen_at = now() WHERE id = $1", &[&ex.id])
                .await?;
        }
    }
    Ok(Json(json!({ "ok": true })))
}

async fn list_executors(
    State(s): State<AppState>,
    _user: AuthUser,
) -> Result<Json<Value>, ApiError> {
    let client = db(&s).await?;
    let rows = client
        .query(
            "SELECT id, name, engine, paired_at, last_seen_at, revoked_at, engines FROM executors ORDER BY created_at",
            &[],
        )
        .await?;
    let list: Vec<Value> = rows
        .iter()
        .map(|r| {
            let paired: Option<DateTime<Utc>> = r.get(3);
            let last: Option<DateTime<Utc>> = r.get(4);
            let revoked: Option<DateTime<Utc>> = r.get(5);
            let engines: Value = r.get(6);
            let status = if revoked.is_some() {
                "revoked"
            } else if paired.is_none() {
                "pending"
            } else if last.is_some_and(|t| (Utc::now() - t).num_seconds() < EXECUTOR_ONLINE_WINDOW) {
                "online"
            } else {
                "offline"
            };
            json!({
                "id": r.get::<_, Uuid>(0),
                "name": r.get::<_, String>(1),
                "engine": r.get::<_, String>(2),
                "status": status,
                "engines": engines,
            })
        })
        .collect();
    Ok(Json(json!(list)))
}

async fn revoke_executor(
    State(s): State<AppState>,
    admin: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    require_admin(&admin)?;
    let mut client = db(&s).await?;
    let tx = client.transaction().await?;
    let done = tx
        .execute("UPDATE executors SET revoked_at = now(), token_hash = NULL WHERE id = $1 AND revoked_at IS NULL", &[&id])
        .await?;
    if done == 0 {
        return Err(ApiError::NotFound);
    }
    // jobs em andamento voltam à fila para outro executor (o lease deste é invalidado)
    tx.execute(
        "UPDATE jobs SET status = 'queued', executor_id = NULL, lease_token = NULL, lease_expires = NULL, updated_at = now() \
         WHERE executor_id = $1 AND status = 'running'",
        &[&id],
    )
    .await?;
    tx.commit().await?;
    audit(
        &client,
        &admin.email,
        "executor_revoke",
        &id.to_string(),
        "ok",
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// O executor é o dono atual do job (lease vigente não é exigido para consultar).
async fn check_job_lease_owner(client: &Db, ex: &ExecutorAuth, job: Uuid) -> Result<(), ApiError> {
    client
        .query_opt(
            "SELECT 1 FROM jobs WHERE id = $1 AND executor_id = $2",
            &[&job, &ex.id],
        )
        .await?
        .map(|_| ())
        .ok_or(ApiError::NotFound)
}

/// Lease atual: o executor detém o job, o token confere e o prazo não venceu. Trava a linha do job, o que
/// serializa as escritas dele. Devolve a conversa e se o cancelamento foi pedido.
async fn check_job_lease<C: GenericClient>(
    client: &C,
    ex: &ExecutorAuth,
    job: Uuid,
    lease: Uuid,
) -> Result<(Uuid, bool), ApiError> {
    client
        .query_opt(
            "SELECT conversation_id, cancel_requested_at IS NOT NULL FROM jobs \
             WHERE id = $1 AND executor_id = $2 AND status = 'running' AND lease_token = $3 AND lease_expires > now() FOR UPDATE",
            &[&job, &ex.id, &lease],
        )
        .await?
        .map(|r| (r.get(0), r.get(1)))
        .ok_or(ApiError::Conflict("stale_lease"))
}

/// Emite o estado do job e, quando ele termina, a mensagem do assistente que o anunciou.
async fn publish_job<C: GenericClient>(client: &C, job: Uuid) -> Result<(), ApiError> {
    emit_job(client, job).await?;
    sync_job_message(client, job).await
}

/// Revogação de concessão: jobs do par nó × projeto na fila são cancelados; os em execução recebem o pedido de
/// cancelamento (o executor confirma pelo finish). Eventos saem na mesma transação da revogação.
pub(crate) async fn revoke_pair_jobs<C: GenericClient>(
    c: &C,
    node: Uuid,
    project: Uuid,
) -> Result<(), ApiError> {
    let queued = c
        .query(
            "UPDATE jobs SET status = 'cancelled', error = 'concessão revogada', updated_at = now() \
             WHERE node_id = $1 AND project_id = $2 AND status = 'queued' RETURNING id",
            &[&node, &project],
        )
        .await?;
    let running = c
        .query(
            "UPDATE jobs SET cancel_requested_at = COALESCE(cancel_requested_at, now()), updated_at = now() \
             WHERE node_id = $1 AND project_id = $2 AND status = 'running' RETURNING id",
            &[&node, &project],
        )
        .await?;
    for r in queued.iter().chain(running.iter()) {
        let id: Uuid = r.get(0);
        cancel_queued_tasks(c, id).await?;
        publish_job(c, id).await?;
    }
    Ok(())
}

/// Cancela as ferramentas do job que ainda não foram entregues ao OneNode.
async fn cancel_queued_tasks<C: GenericClient>(client: &C, job: Uuid) -> Result<(), ApiError> {
    for r in client
        .query(
            "UPDATE tasks SET status = 'cancelled', updated_at = now() WHERE job_id = $1 AND status = 'queued' RETURNING id",
            &[&job],
        )
        .await?
    {
        emit_task(client, r.get(0)).await?;
    }
    Ok(())
}

async fn next_job(State(s): State<AppState>, ex: ExecutorAuth) -> Result<Response, ApiError> {
    let mut client = db(&s).await?;
    let tx = client.transaction().await?;
    // concessão revogada cancela o pedido antes de qualquer execução
    for r in tx
        .query(
            "UPDATE jobs j SET status = 'cancelled', error = 'concessão revogada', updated_at = now() WHERE j.status = 'queued' AND j.kind = 'analysis' \
             AND NOT EXISTS (SELECT 1 FROM node_project_grants g JOIN onenodes n ON n.id = g.node_id \
             WHERE g.node_id = j.node_id AND g.project_id = j.project_id AND g.revoked_at IS NULL AND n.revoked_at IS NULL) \
             RETURNING id",
            &[],
        )
        .await?
    {
        publish_job(&tx, r.get(0)).await?;
    }
    // lease vencido: volta à fila; se o cancelamento já fora pedido, encerra (o executor não confirmou)
    for r in tx
        .query(
            "UPDATE jobs SET status = CASE WHEN cancel_requested_at IS NULL THEN 'queued' ELSE 'cancelled' END, \
             error = CASE WHEN cancel_requested_at IS NULL THEN error ELSE 'o OneExecutor não confirmou o cancelamento' END, \
             executor_id = NULL, lease_token = NULL, lease_expires = NULL, updated_at = now() \
             WHERE status = 'running' AND lease_expires < now() RETURNING id",
            &[],
        )
        .await?
    {
        publish_job(&tx, r.get(0)).await?;
    }
    let token = Uuid::new_v4();
    let row = tx
        .query_opt(
            "UPDATE jobs SET status = 'running', executor_id = $1, lease_token = $2, lease_expires = now() + make_interval(secs => $3), updated_at = now() \
             WHERE id = (SELECT id FROM jobs WHERE status = 'queued' ORDER BY created_at FOR UPDATE SKIP LOCKED LIMIT 1) \
             RETURNING id, conversation_id, node_id, project_id, question, lease_token, lease_expires, kind",
            &[&ex.id, &token, &JOB_LEASE_SECONDS],
        )
        .await?;
    if let Some(r) = &row {
        publish_job(&tx, r.get(0)).await?;
    }
    tx.commit().await?;
    match row {
        None => Ok(StatusCode::NO_CONTENT.into_response()),
        Some(r) => {
            let profile = client.query_opt(
                "SELECT c.id, es.instructions FROM executor_settings es \
                 JOIN cli_connections c ON c.id=es.cli_connection_id AND c.executor_id=$1 \
                 JOIN executors e ON e.id=$1 AND e.revoked_at IS NULL \
                 WHERE es.executor_id=$1 AND c.engine='codex' AND c.auth_state='authenticated' \
                   AND c.auth_checked_at>now()-interval '90 seconds' \
                   AND c.usage_authorized=true AND e.last_seen_at>now()-interval '90 seconds' \
                   AND e.engines_reported_at>now()-interval '90 seconds' \
                   AND EXISTS(SELECT 1 FROM jsonb_array_elements(e.engines) x WHERE x->>'name'='codex' AND x->>'installed'='true' AND x->>'authenticated'='true' AND x->>'approved'='true')",
                &[&ex.id],
            ).await?;
            let (connection_id, instructions) = profile
                .map(|p| (Some(p.get::<_, Uuid>(0)), p.get::<_, String>(1)))
                .unwrap_or((None, String::new()));
            Ok(Json(json!({
                "id": r.get::<_, Uuid>(0),
                "conversation_id": r.get::<_, Uuid>(1),
                // jobs de delegação não têm nó nem projeto
                "node_id": r.get::<_, Option<Uuid>>(2),
                "project_id": r.get::<_, Option<Uuid>>(3),
                "question": r.get::<_, String>(4),
                "lease_token": r.get::<_, Uuid>(5),
                "lease_expires_at": r.get::<_, DateTime<Utc>>(6),
                "kind": r.get::<_, String>(7),
                "cli_connection_id": connection_id,
                "instructions": instructions,
            }))
            .into_response())
        },
    }
}

/// O executor pede uma ferramenta de leitura. A tarefa nasce no mesmo OneNode, no mesmo projeto e
/// ligada ao job. Quem processa é o OneNode, com a mesma confinação de sempre. Com cancelamento pedido, não.
async fn request_tool(
    State(s): State<AppState>,
    ex: ExecutorAuth,
    Path(job): Path<Uuid>,
    Json(b): Json<Value>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let lease = parse_uuid(b.get("lease_token")).ok_or(ApiError::BadRequest("invalid_lease"))?;
    let tool = b
        .get("tool")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let args = json!({ "path": b.get("path").and_then(Value::as_str).unwrap_or("") });
    validate_tool_args(&tool, &args).map_err(ApiError::BadRequest)?;
    let mut client = db(&s).await?;
    let tx = client.transaction().await?;
    let (_, cancel_requested) = check_job_lease(&tx, &ex, job, lease).await?;
    if cancel_requested {
        return Err(ApiError::Conflict("cancel_requested"));
    }
    let n: i64 = tx
        .query_one("SELECT count(*) FROM tasks WHERE job_id = $1", &[&job])
        .await?
        .get(0);
    if n >= MAX_TOOL_REQUESTS_PER_JOB {
        return Err(ApiError::Conflict("tool_limit_reached"));
    }
    let r = tx
        .query_one(
            "SELECT conversation_id, node_id, project_id, created_by FROM jobs WHERE id = $1",
            &[&job],
        )
        .await?;
    let (conv, node, project, owner): (Uuid, Uuid, Uuid, Uuid) =
        (r.get(0), r.get(1), r.get(2), r.get(3));
    // concessão revogada (mesmo que a revogação ainda não tenha cancelado o job): nenhuma ferramenta nova
    if check_grant(&tx, node, project).await.is_err() {
        return Err(ApiError::Conflict("grant_revoked"));
    }
    let task: Uuid = tx
        .query_one(
            "INSERT INTO tasks(conversation_id, node_id, project_id, created_by, prompt, idempotency_key, tool, args, job_id) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id",
            &[&conv, &node, &project, &owner, &format!("executor: {tool}"), &format!("job-{job}-{}", Uuid::new_v4()), &tool, &args, &job],
        )
        .await?
        .get(0);
    emit_task(&tx, task).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(json!({ "task_id": task }))))
}

async fn tool_status(
    State(s): State<AppState>,
    ex: ExecutorAuth,
    Path((job, task)): Path<(Uuid, Uuid)>,
) -> Result<Json<Value>, ApiError> {
    let client = db(&s).await?;
    check_job_lease_owner(&client, &ex, job).await?;
    let row = client
        .query_opt(
            "SELECT status, result FROM tasks WHERE id = $1 AND job_id = $2",
            &[&task, &job],
        )
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(
        json!({ "status": row.get::<_, String>(0), "result": row.get::<_, Option<Value>>(1) }),
    ))
}

/// Progresso do executor. Sequência própria do job (um único escritor por job, serializado pelo lock).
/// A resposta informa se o cancelamento foi pedido, para o executor parar cedo.
async fn job_event(
    State(s): State<AppState>,
    ex: ExecutorAuth,
    Path(job): Path<Uuid>,
    Json(b): Json<Value>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let lease = parse_uuid(b.get("lease_token")).ok_or(ApiError::BadRequest("invalid_lease"))?;
    let kind = b
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let payload = b.get("payload").cloned().unwrap_or_else(|| json!({}));
    if !text_len_ok(&kind, 1, 64) || payload.to_string().len() > MAX_EVENT_BYTES {
        return Err(ApiError::BadRequest("invalid_event"));
    }
    let mut client = db(&s).await?;
    let tx = client.transaction().await?;
    let (conv, cancel_requested) = check_job_lease(&tx, &ex, job, lease).await?;
    let seq: i32 = tx
        .query_one(
            "INSERT INTO job_events(job_id, seq, kind, payload) \
             VALUES ($1, (SELECT COALESCE(MAX(seq) + 1, 0) FROM job_events WHERE job_id = $1), $2, $3) RETURNING seq",
            &[&job, &kind, &payload],
        )
        .await?
        .get(0);
    emit(
        &tx,
        conv,
        "job.progress",
        json!({ "job_id": job, "seq": seq, "kind": kind, "payload": payload }),
    )
    .await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "cancel_requested": cancel_requested })),
    ))
}

/// Renova o lease enquanto o Codex roda e informa se o cancelamento foi pedido.
async fn job_heartbeat(
    State(s): State<AppState>,
    ex: ExecutorAuth,
    Path(job): Path<Uuid>,
    Json(b): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let lease = parse_uuid(b.get("lease_token")).ok_or(ApiError::BadRequest("invalid_lease"))?;
    let mut client = db(&s).await?;
    let tx = client.transaction().await?;
    let (_, cancel_requested) = check_job_lease(&tx, &ex, job, lease).await?;
    tx.execute(
        "UPDATE jobs SET lease_expires = now() + make_interval(secs => $2), updated_at = now() WHERE id = $1",
        &[&job, &JOB_LEASE_SECONDS],
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({ "cancel_requested": cancel_requested })))
}

/// Resultado do executor. `cancelled` só vale depois de um pedido de cancelamento.
async fn finish_job(
    State(s): State<AppState>,
    ex: ExecutorAuth,
    Path(job): Path<Uuid>,
    Json(b): Json<Value>,
) -> Result<StatusCode, ApiError> {
    let lease = parse_uuid(b.get("lease_token")).ok_or(ApiError::BadRequest("invalid_lease"))?;
    let status = match b.get("status").and_then(Value::as_str) {
        Some(st @ ("succeeded" | "failed" | "cancelled")) => st,
        _ => return Err(ApiError::BadRequest("invalid_status")),
    };
    let answer = b.get("answer").and_then(Value::as_str).map(String::from);
    if answer
        .as_deref()
        .is_some_and(|a| a.len() > MAX_ANSWER_BYTES)
    {
        return Err(ApiError::BadRequest("answer_too_large"));
    }
    let evidence = b.get("evidence").cloned().unwrap_or_else(|| json!([]));
    let error = if status == "cancelled" {
        Some("cancelado pelo usuário".to_string())
    } else {
        b.get("error").and_then(Value::as_str).map(String::from)
    };
    let mut client = db(&s).await?;
    let tx = client.transaction().await?;
    let (_, cancel_requested) = check_job_lease(&tx, &ex, job, lease).await?;
    if status == "cancelled" && !cancel_requested {
        return Err(ApiError::Conflict("cancel_not_requested"));
    }
    let done = tx
        .execute(
            "UPDATE jobs SET status = $2, answer = $3, evidence = $4, error = $5, lease_token = NULL, lease_expires = NULL, updated_at = now() \
             WHERE id = $1 AND status = 'running' AND lease_token = $6",
            &[&job, &status, &answer, &evidence, &error, &lease],
        )
        .await?;
    if done == 0 {
        return Err(ApiError::Conflict("stale_lease"));
    }
    publish_job(&tx, job).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Cancelamento pedido pelo dono. Job na fila é cancelado já; job em execução só recebe o pedido, e o
/// status final `cancelled` depende da confirmação do executor (ou do vencimento do lease).
pub async fn cancel_job(
    State(s): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let mut conn = db(&s).await?;
    let client = conn.transaction().await?;
    let status: String = client
        .query_opt(
            "SELECT status FROM jobs WHERE id = $1 AND created_by = $2 FOR UPDATE",
            &[&id, &user.id],
        )
        .await?
        .ok_or(ApiError::NotFound)?
        .get(0);
    match status.as_str() {
        "queued" => {
            client
                .execute(
                    "UPDATE jobs SET status = 'cancelled', error = 'cancelado pelo usuário', updated_at = now() WHERE id = $1",
                    &[&id],
                )
                .await?;
        }
        "running" => {
            client
                .execute(
                    "UPDATE jobs SET cancel_requested_at = COALESCE(cancel_requested_at, now()), updated_at = now() WHERE id = $1",
                    &[&id],
                )
                .await?;
        }
        _ => return Err(ApiError::Conflict("already_finished")),
    }
    cancel_queued_tasks(&client, id).await?;
    publish_job(&client, id).await?;
    audit(&client, &user.email, "job_cancel", &id.to_string(), &status).await?;
    client.commit().await?;
    Ok((StatusCode::ACCEPTED, Json(json!({ "id": id }))))
}

async fn get_job(
    State(s): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    let client = db(&s).await?;
    let r = client
        .query_opt(
            "SELECT conversation_id, node_id, project_id, status, question, answer, evidence, error, created_at, updated_at, created_by, cancel_requested_at IS NOT NULL, kind, agent_id, agent_version, delegation FROM jobs WHERE id = $1",
            &[&id],
        )
        .await?
        .ok_or(ApiError::NotFound)?;
    if r.get::<_, Uuid>(10) != user.id {
        return Err(ApiError::NotFound);
    }
    let events = client
        .query(
            "SELECT seq, kind, payload, created_at FROM job_events WHERE job_id = $1 ORDER BY seq",
            &[&id],
        )
        .await?;
    Ok(Json(json!({
        "id": id,
        "conversation_id": r.get::<_, Uuid>(0),
        "node_id": r.get::<_, Option<Uuid>>(1),
        "project_id": r.get::<_, Option<Uuid>>(2),
        "kind": r.get::<_, String>(12),
        "agent_id": r.get::<_, Option<String>>(13),
        "agent_version": r.get::<_, Option<String>>(14),
        "delegation": r.get::<_, Option<Value>>(15),
        "status": r.get::<_, String>(3),
        "question": r.get::<_, String>(4),
        "answer": r.get::<_, Option<String>>(5),
        "evidence": r.get::<_, Value>(6),
        "error": r.get::<_, Option<String>>(7),
        "cancel_requested": r.get::<_, bool>(11),
        "created_at": r.get::<_, DateTime<Utc>>(8),
        "updated_at": r.get::<_, DateTime<Utc>>(9),
        "events": events.iter().map(|e| json!({ "seq": e.get::<_, i32>(0), "kind": e.get::<_, String>(1), "payload": e.get::<_, Value>(2), "created_at": e.get::<_, DateTime<Utc>>(3) })).collect::<Vec<_>>(),
    })))
}

/// Mensagens determinísticas do chat. Nenhuma delas é resposta de IA: são regras do OneHub.
pub const CENTRAL_CONTEXT_MESSAGE: &str = "Contexto OneHub Central: administração do ambiente. Este contexto não analisa projetos. Para analisar um projeto, escolha um OneNode com projeto autorizado.";
pub const NO_PROJECTS_MESSAGE: &str = "Este OneNode está conectado, mas não tem projetos autorizados. Peça ao administrador uma concessão na página OneNodes.";
pub const NO_EXECUTOR_MESSAGE: &str = "Nenhum OneExecutor (Codex) está online neste Central, então a análise não pode ser executada agora. A conversa continua disponível.";
pub const CODEX_NOT_AUTHENTICATED_MESSAGE: &str = "O OneExecutor está online, mas o Codex CLI ainda não está autenticado. Conecte a conta em One Provider AI e tente de novo.";
pub const CODEX_NOT_APPROVED_MESSAGE: &str = "O Codex CLI está instalado neste OneExecutor, mas ainda não foi aprovado para execução.";

/// Há um OneExecutor pareado, não revogado e com sinal recente (mesma janela de presença dos OneNodes).
/// Executor pronto para `engine`: pareado, não revogado, com sinal e relatório recentes, e o motor
/// instalado, autenticado e aprovado. Estar online sem motor pronto não basta.
pub(crate) async fn executor_ready<C: GenericClient>(client: &C, engine: &str) -> Result<bool, ApiError> {
    let row = client
        .query_opt(
            "SELECT 1 FROM executors e, jsonb_array_elements(e.engines) x \
             WHERE e.revoked_at IS NULL AND e.paired_at IS NOT NULL \
             AND e.last_seen_at > now() - ($1::int * interval '1 second') \
             AND e.engines_reported_at > now() - ($1::int * interval '1 second') \
             AND x->>'name' = $2 AND x->>'installed' = 'true' AND x->>'authenticated' = 'true' AND x->>'approved' = 'true' \
             LIMIT 1",
            &[&(EXECUTOR_ONLINE_WINDOW as i32), &engine],
        )
        .await?;
    Ok(row.is_some())
}

/// Motivo determinístico quando o motor existe online, mas ainda não pode executar.
pub(crate) async fn executor_block_reason<C: GenericClient>(client: &C, engine: &str) -> Result<&'static str, ApiError> {
    if executor_ready(client, engine).await? {
        return Ok("");
    }
    let rows = client
        .query(
            "SELECT x->>'installed', x->>'authenticated', x->>'approved' \
             FROM executors e, jsonb_array_elements(e.engines) x \
             WHERE e.revoked_at IS NULL AND e.paired_at IS NOT NULL \
             AND e.last_seen_at > now() - ($1::int * interval '1 second') \
             AND e.engines_reported_at > now() - ($1::int * interval '1 second') \
             AND x->>'name' = $2",
            &[&(EXECUTOR_ONLINE_WINDOW as i32), &engine],
        )
        .await?;
    if rows.is_empty() {
        return Ok(NO_EXECUTOR_MESSAGE);
    }
    let any_installed = rows.iter().any(|r| r.get::<_, String>(0) == "true");
    let any_authenticated = rows.iter().any(|r| r.get::<_, String>(1) == "true");
    let any_approved = rows.iter().any(|r| r.get::<_, String>(2) == "true");
    if any_installed && !any_authenticated {
        return Ok(CODEX_NOT_AUTHENTICATED_MESSAGE);
    }
    if any_installed && any_authenticated && !any_approved {
        return Ok(CODEX_NOT_APPROVED_MESSAGE);
    }
    Ok(NO_EXECUTOR_MESSAGE)
}

/// Estados de motor de todos os executores prontos para receber tarefas (a base da seleção de agentes).
pub(crate) async fn engine_states<C: GenericClient>(client: &C) -> Result<Vec<EngineState>, ApiError> {
    let rows = client
        .query(
            "SELECT x->>'name', x->>'installed', x->>'authenticated', x->>'approved' \
             FROM executors e, jsonb_array_elements(e.engines) x \
             WHERE e.revoked_at IS NULL AND e.paired_at IS NOT NULL \
             AND e.last_seen_at > now() - ($1::int * interval '1 second') \
             AND e.engines_reported_at > now() - ($1::int * interval '1 second')",
            &[&(EXECUTOR_ONLINE_WINDOW as i32)],
        )
        .await?;
    Ok(rows
        .iter()
        .map(|r| EngineState {
            name: r.get::<_, String>(0),
            installed: r.get::<_, String>(1) == "true",
            authenticated: r.get::<_, String>(2) == "true",
            approved: r.get::<_, String>(3) == "true",
        })
        .collect())
}

/// Pedido de análise a partir de texto livre no chat. Retorna (estado da mensagem, texto, job).
/// Sem OneNode, sem projeto escolhido (e ambíguo) ou sem capacidade, o pedido é recusado com a razão.
pub async fn analysis_turn<C: GenericClient>(
    client: &C,
    user: &AuthUser,
    conv: Uuid,
    text: &str,
    node: Option<Uuid>,
    project: Option<Uuid>,
    user_msg: Uuid,
) -> Result<(&'static str, String, Option<Uuid>), ApiError> {
    // Contexto OneHub Central: administração do ambiente, sem análise de projeto e sem OneNode externo.
    let Some(node) = node else {
        return Ok(("rejected", CENTRAL_CONTEXT_MESSAGE.into(), None));
    };
    let state = client
        .query_opt(
            "SELECT revoked_at IS NOT NULL, suspended_at IS NOT NULL FROM onenodes WHERE id = $1",
            &[&node],
        )
        .await?;
    let Some(state) = state else {
        return Ok(("rejected", "OneNode não encontrado.".into(), None));
    };
    if state.get::<_, bool>(0) {
        return Ok(("rejected", "Este OneNode foi revogado. Escolha outro OneNode.".into(), None));
    }
    if state.get::<_, bool>(1) {
        return Ok(("rejected", "Este OneNode está suspenso: análises ficam indisponíveis até ser retomado.".into(), None));
    }
    let project = match project {
        Some(p) => p,
        None => {
            let rows = client
                .query(
                    "SELECT g.project_id FROM node_project_grants g JOIN onenodes n ON n.id = g.node_id \
                     WHERE g.node_id = $1 AND g.revoked_at IS NULL AND n.revoked_at IS NULL",
                    &[&node],
                )
                .await?;
            match rows.len() {
                1 => rows[0].get(0),
                0 => return Ok(("rejected", NO_PROJECTS_MESSAGE.into(), None)),
                _ => return Ok(("rejected", "Este OneNode tem mais de um projeto. Escolha o projeto antes de pedir a análise.".into(), None)),
            }
        }
    };
    if check_grant(client, node, project).await.is_err() {
        return Ok((
            "rejected",
            "O OneNode escolhido não tem permissão para este projeto.".into(),
            None,
        ));
    }
    // Sem motor Codex pronto, nada é enfileirado: o chat não promete uma execução que não pode acontecer.
    let block = executor_block_reason(client, "codex").await?;
    if !block.is_empty() {
        return Ok(("rejected", block.into(), None));
    }
    let open: i64 = client
        .query_one(
            "SELECT count(*) FROM jobs WHERE created_by = $1 AND status IN ('queued', 'running')",
            &[&user.id],
        )
        .await?
        .get(0);
    if open >= MAX_OPEN_JOBS_PER_USER {
        return Ok((
            "rejected",
            "Você já tem análises demais em andamento. Aguarde uma terminar.".into(),
            None,
        ));
    }
    let job: Uuid = client
        .query_one(
            "INSERT INTO jobs(conversation_id, node_id, project_id, created_by, question, idempotency_key) \
             VALUES ($1, $2, $3, $4, $5, $6) RETURNING id",
            &[&conv, &node, &project, &user.id, &text, &format!("turno-{user_msg}")],
        )
        .await?
        .get(0);
    emit_job(client, job).await?;
    audit(
        client,
        &user.email,
        "analysis_create",
        &job.to_string(),
        "queued",
    )
    .await?;
    Ok((
        "task",
        "Pedido enviado ao OneExecutor (Codex) para análise do projeto. O resultado aparece aqui quando terminar.".into(),
        Some(job),
    ))
}

// ───────────── Delegação de OneAgents ─────────────

#[derive(Debug, Deserialize)]
pub struct DelegationBody {
    pub conversation_id: Uuid,
    pub intent: String,
}

/// Agentes embutidos, carregados uma vez. Instruções só entram no prompt do agente selecionado.
fn agent_registry() -> &'static AgentRegistry {
    static REG: std::sync::OnceLock<AgentRegistry> = std::sync::OnceLock::new();
    REG.get_or_init(|| {
        AgentRegistry::new(one_registry::agent::builtin()).expect("agentes embutidos são válidos")
    })
}

/// Prompt da delegação: instruções versionadas do agente, a intenção e os critérios de aceite.
pub fn delegation_prompt(instructions: &str, intent: &str, acceptance: &[String]) -> String {
    let criteria: String = acceptance.iter().map(|c| format!("- {c}\n")).collect();
    format!(
        "{instructions}\n\n## Tarefa\n{intent}\n\n## Critérios de aceite\n{criteria}\n\
         Responda em português e diga o que não foi possível verificar. Não modifique nenhum arquivo."
    )
}

/// Seleciona o agente pela intenção e, se o ambiente permite, cria o job de delegação na fila do OneExecutor.
/// Bloqueio (sem motor pronto, sem ferramenta, sem permissão) é devolvido com motivo, sem job.
async fn create_delegation(
    State(s): State<AppState>,
    user: AuthUser,
    Json(b): Json<DelegationBody>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    if !text_len_ok(&b.intent, 1, 2000) {
        return Err(ApiError::BadRequest("invalid_body"));
    }
    let mut conn = db(&s).await?;
    let tx = conn.transaction().await?;
    owned_conversation(&tx, &user, b.conversation_id).await?;
    let engines = engine_states(&tx).await?;
    let codex_ready = engines.iter().any(|e| e.name == "codex" && e.installed && e.authenticated && e.approved);
    // ferramenta real disponível hoje: o motor Codex em modo somente leitura (nenhuma ferramenta de OneNode)
    let tools_available = if codex_ready { vec!["codex-readonly".to_string()] } else { vec![] };
    let env = Environment {
        engines,
        tools_available,
        // delegação do Central: leitura apenas; nenhuma permissão de escrita ou de OneNode é concedida aqui
        effective_permissions: vec![Permission::Read],
    };
    let orch = orchestrate(agent_registry(), &b.intent, &env);
    let Some(d) = orch.delegation else {
        tx.commit().await?;
        return Ok((
            StatusCode::OK,
            Json(json!({ "status": "no_agent", "reason": orch.no_agent_reason })),
        ));
    };
    if let Status::Blocked { reason } = &d.status {
        let delegation = serde_json::to_value(&d).map_err(|_| ApiError::Internal)?;
        tx.commit().await?;
        return Ok((
            StatusCode::OK,
            Json(json!({ "status": "blocked", "reason": reason, "delegation": delegation })),
        ));
    }
    let instructions = orch.instructions.ok_or(ApiError::Internal)?;
    let prompt = delegation_prompt(instructions, &b.intent, &d.acceptance);
    if !text_len_ok(&prompt, 1, 16 * 1024) {
        return Err(ApiError::BadRequest("prompt_too_large"));
    }
    let delegation = serde_json::to_value(&d).map_err(|_| ApiError::Internal)?;
    let job: Uuid = tx
        .query_one(
            "INSERT INTO jobs(conversation_id, node_id, project_id, created_by, kind, question, idempotency_key, agent_id, agent_version, delegation) \
             VALUES ($1, NULL, NULL, $2, 'delegation', $3, $4, $5, $6, $7) RETURNING id",
            &[
                &b.conversation_id,
                &user.id,
                &prompt,
                &format!("delegacao-{}", Uuid::new_v4()),
                &d.agent_id,
                &d.agent_version,
                &delegation,
            ],
        )
        .await?
        .get(0);
    publish_job(&tx, job).await?;
    audit(&tx, &user.email, "delegation_create", &job.to_string(), "queued").await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "status": "queued", "job_id": job, "delegation": delegation })),
    ))
}

