//! Contas de CLI (Claude Code, Codex, Cursor) e seus logins oficiais, coordenados pelo Central e executados
//! pelo OneExecutor sob o usuário do serviço. O Central nunca vê credenciais: recebe só o estado, o link oficial
//! e o código de dispositivo de um login em andamento, e o código que o próprio usuário cola (Claude).

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::executor::{ExecutorAuth, EXECUTOR_ONLINE_WINDOW};
use crate::routes::{audit, db, require_admin, text_len_ok, ApiError, AppState, AuthUser};

/// Prazo para um login ser concluído (o executor também aplica esse limite localmente).
const SESSION_TTL_MINUTES: i32 = 10;
const ENGINES: [&str; 3] = ["codex", "claude", "cursor"];

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/cli-connections", get(list_connections).post(create_connection))
        .route("/v1/cli-connections/{id}", axum::routing::delete(delete_connection))
        .route("/v1/cli-connections/{id}/login", post(start_login))
        .route("/v1/cli-connections/{id}/status", post(start_status))
        .route("/v1/cli-connections/{id}/logout", post(start_logout))
        .route("/v1/cli-connections/{id}/usage", post(set_usage))
        .route("/v1/cli-sessions/{id}", get(get_session))
        .route("/v1/cli-sessions/{id}/cancel", post(cancel_session))
        .route("/v1/cli-sessions/{id}/code", post(submit_code))
        .route("/executor/v1/cli/next", post(next_session))
        .route("/executor/v1/cli/sessions/{id}/control", post(session_control))
        .route("/executor/v1/cli/sessions/{id}/event", post(session_event))
}

/// Links aceitos para autorização: só HTTPS de domínios oficiais de cada fornecedor (defesa em profundidade:
/// o executor já filtra antes de enviar).
fn official_url(engine: &str, url: &str) -> bool {
    let hosts: &[&str] = match engine {
        "codex" => &["auth.openai.com"],
        "claude" => &["claude.ai", "console.anthropic.com"],
        "cursor" => &["cursor.com"],
        _ => return false,
    };
    let Some(rest) = url.strip_prefix("https://") else { return false };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    hosts.contains(&host) && url.len() <= 2048 && !url.chars().any(char::is_control)
}

fn device_code_ok(code: &str) -> bool {
    let parts: Vec<&str> = code.split('-').collect();
    parts.len() == 2
        && parts.iter().all(|p| (4..=5).contains(&p.len()) && p.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()))
}

async fn list_connections(State(s): State<AppState>, admin: AuthUser) -> Result<Json<Value>, ApiError> {
    require_admin(&admin)?;
    let client = db(&s).await?;
    let rows = client
        .query(
            "SELECT c.id, c.name, c.engine, c.executor_id, e.name, c.usage_authorized, c.auth_state, c.auth_checked_at, \
                    a.id, a.action, a.state, a.url, a.device_code, a.result \
             FROM cli_connections c \
             JOIN executors e ON e.id = c.executor_id \
             LEFT JOIN LATERAL (SELECT * FROM cli_sessions s WHERE s.connection_id = c.id ORDER BY s.created_at DESC LIMIT 1) a ON true \
             ORDER BY c.created_at",
            &[],
        )
        .await?;
    let items: Vec<Value> = rows.iter().map(connection_json).collect();
    Ok(Json(json!({ "connections": items })))
}

fn connection_json(r: &tokio_postgres::Row) -> Value {
    let session_id: Option<Uuid> = r.get(8);
    let state: Option<String> = r.get(10);
    let awaiting = state.as_deref() == Some("awaiting_code");
    let session = session_id.map(|id| {
        json!({
            "id": id,
            "action": r.get::<_, String>(9),
            "state": state,
            "url": r.get::<_, Option<String>>(11),
            "device_code": r.get::<_, Option<String>>(12),
            "awaiting_code": awaiting,
            "result": r.get::<_, Option<String>>(13),
        })
    });
    json!({
        "id": r.get::<_, Uuid>(0),
        "name": r.get::<_, String>(1),
        "engine": r.get::<_, String>(2),
        "executor_id": r.get::<_, Uuid>(3),
        "executor_name": r.get::<_, String>(4),
        "usage_authorized": r.get::<_, bool>(5),
        "auth_state": r.get::<_, String>(6),
        "auth_checked_at": r.get::<_, Option<DateTime<Utc>>>(7),
        "session": session,
    })
}

async fn create_connection(
    State(s): State<AppState>,
    admin: AuthUser,
    Json(b): Json<Value>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    require_admin(&admin)?;
    let name = b.get("name").and_then(Value::as_str).unwrap_or("").trim().to_string();
    if !text_len_ok(&name, 1, 120) {
        return Err(ApiError::BadRequest("invalid_name"));
    }
    let engine = b.get("engine").and_then(Value::as_str).unwrap_or("");
    if !ENGINES.contains(&engine) {
        return Err(ApiError::BadRequest("invalid_engine"));
    }
    let executor_id = b
        .get("executor_id")
        .and_then(Value::as_str)
        .and_then(|v| Uuid::parse_str(v).ok())
        .ok_or(ApiError::BadRequest("invalid_executor"))?;
    // o uso desta conta precisa ser declarado explicitamente pelo administrador
    if b.get("usage_authorized").and_then(Value::as_bool) != Some(true) {
        return Err(ApiError::BadRequest("usage_not_authorized"));
    }
    let client = db(&s).await?;
    let exists = client
        .query_opt("SELECT 1 FROM executors WHERE id = $1 AND revoked_at IS NULL", &[&executor_id])
        .await?;
    if exists.is_none() {
        return Err(ApiError::NotFound);
    }
    let row = client
        .query_one(
            "INSERT INTO cli_connections(name, engine, executor_id, usage_authorized, created_by) \
             VALUES ($1, $2, $3, true, $4) RETURNING id",
            &[&name, &engine, &executor_id, &admin.email],
        )
        .await
        .map_err(|_| ApiError::Conflict("name_in_use"))?;
    let id: Uuid = row.get(0);
    audit(&client, &admin.email, "cli_connection_create", &id.to_string(), "ok").await?;
    Ok((StatusCode::CREATED, Json(json!({ "id": id, "name": name, "engine": engine, "auth_state": "unknown" }))))
}

/// Cria uma sessão para a conta, se o executor dela estiver online e não houver outra sessão ativa.
async fn enqueue(s: &AppState, admin: &AuthUser, id: Uuid, action: &str) -> Result<(StatusCode, Json<Value>), ApiError> {
    require_admin(admin)?;
    let client = db(s).await?;
    let conn = client
        .query_opt(
            "SELECT c.executor_id, c.usage_authorized FROM cli_connections c WHERE c.id = $1",
            &[&id],
        )
        .await?
        .ok_or(ApiError::NotFound)?;
    let executor_id: Uuid = conn.get(0);
    let authorized: bool = conn.get(1);
    if !authorized {
        return Err(ApiError::Forbidden("usage_not_authorized"));
    }
    let online = client
        .query_opt(
            "SELECT 1 FROM executors e WHERE e.id = $1 AND e.revoked_at IS NULL AND e.paired_at IS NOT NULL \
             AND e.last_seen_at > now() - ($2::int * interval '1 second')",
            &[&executor_id, &(EXECUTOR_ONLINE_WINDOW as i32)],
        )
        .await?;
    if online.is_none() {
        return Err(ApiError::Conflict("executor_offline"));
    }
    let row = client
        .query_opt(
            "INSERT INTO cli_sessions(connection_id, action, created_by, expires_at) \
             SELECT $1, $2, $3, now() + make_interval(mins => $4) \
             WHERE NOT EXISTS (SELECT 1 FROM cli_sessions WHERE connection_id = $1 AND state IN ('pending','running','awaiting_code')) \
             RETURNING id, expires_at",
            &[&id, &action, &admin.email, &SESSION_TTL_MINUTES],
        )
        .await?
        .ok_or(ApiError::Conflict("session_in_progress"))?;
    let sid: Uuid = row.get(0);
    let expires: DateTime<Utc> = row.get(1);
    audit(&client, &admin.email, &format!("cli_{action}_start"), &id.to_string(), "ok").await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({ "session": { "id": sid, "action": action, "state": "pending", "expires_at": expires } })),
    ))
}

async fn start_login(State(s): State<AppState>, admin: AuthUser, Path(id): Path<Uuid>) -> Result<(StatusCode, Json<Value>), ApiError> {
    enqueue(&s, &admin, id, "login").await
}

async fn start_status(State(s): State<AppState>, admin: AuthUser, Path(id): Path<Uuid>) -> Result<(StatusCode, Json<Value>), ApiError> {
    enqueue(&s, &admin, id, "status").await
}

async fn start_logout(State(s): State<AppState>, admin: AuthUser, Path(id): Path<Uuid>) -> Result<(StatusCode, Json<Value>), ApiError> {
    enqueue(&s, &admin, id, "logout").await
}

async fn set_usage(State(s): State<AppState>, admin: AuthUser, Path(id): Path<Uuid>, Json(b): Json<Value>) -> Result<StatusCode, ApiError> {
    require_admin(&admin)?;
    let allowed = b.get("usage_authorized").and_then(Value::as_bool).ok_or(ApiError::BadRequest("invalid_usage"))?;
    let client = db(&s).await?;
    let n = client
        .execute(
            "UPDATE cli_connections SET usage_authorized = $2, updated_at = now() WHERE id = $1",
            &[&id, &allowed],
        )
        .await?;
    if n == 0 {
        return Err(ApiError::NotFound);
    }
    audit(&client, &admin.email, "cli_usage_set", &id.to_string(), if allowed { "allowed" } else { "revoked" }).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_connection(State(s): State<AppState>, admin: AuthUser, Path(id): Path<Uuid>) -> Result<StatusCode, ApiError> {
    require_admin(&admin)?;
    let client = db(&s).await?;
    let row = client
        .query_opt("SELECT auth_state FROM cli_connections WHERE id = $1", &[&id])
        .await?
        .ok_or(ApiError::NotFound)?;
    if row.get::<_, String>(0) == "authenticated" {
        // desconecte a conta antes de remover: a credencial local não pode ficar órfã
        return Err(ApiError::Conflict("logout_first"));
    }
    let active = client
        .query_opt(
            "SELECT 1 FROM cli_sessions WHERE connection_id = $1 AND state IN ('pending','running','awaiting_code')",
            &[&id],
        )
        .await?;
    if active.is_some() {
        return Err(ApiError::Conflict("session_in_progress"));
    }
    client.execute("DELETE FROM cli_connections WHERE id = $1", &[&id]).await?;
    audit(&client, &admin.email, "cli_connection_delete", &id.to_string(), "ok").await?;
    Ok(StatusCode::NO_CONTENT)
}

fn session_json(r: &tokio_postgres::Row) -> Value {
    let state: String = r.get(2);
    json!({
        "id": r.get::<_, Uuid>(0),
        "connection_id": r.get::<_, Uuid>(1),
        "state": state,
        "awaiting_code": state == "awaiting_code",
        "url": r.get::<_, Option<String>>(3),
        "device_code": r.get::<_, Option<String>>(4),
        "result": r.get::<_, Option<String>>(5),
        "expires_at": r.get::<_, DateTime<Utc>>(6),
    })
}

async fn get_session(State(s): State<AppState>, admin: AuthUser, Path(id): Path<Uuid>) -> Result<Json<Value>, ApiError> {
    require_admin(&admin)?;
    let client = db(&s).await?;
    let row = client
        .query_opt(
            "SELECT id, connection_id, state, url, device_code, result, expires_at FROM cli_sessions WHERE id = $1",
            &[&id],
        )
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(session_json(&row)))
}

async fn cancel_session(State(s): State<AppState>, admin: AuthUser, Path(id): Path<Uuid>) -> Result<StatusCode, ApiError> {
    require_admin(&admin)?;
    let client = db(&s).await?;
    // pendente cancela na hora; em andamento o executor encerra o processo na próxima checagem
    let n = client
        .execute(
            "UPDATE cli_sessions SET cancel_requested = true, \
               state = CASE WHEN state = 'pending' THEN 'cancelled' ELSE state END, \
               finished_at = CASE WHEN state = 'pending' THEN now() ELSE finished_at END \
             WHERE id = $1 AND state IN ('pending','running','awaiting_code')",
            &[&id],
        )
        .await?;
    if n == 0 {
        return Err(ApiError::NotFound);
    }
    audit(&client, &admin.email, "cli_session_cancel", &id.to_string(), "ok").await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Código colado pelo usuário quando o fluxo oficial pede (Claude Code). Vai ao executor uma única vez.
async fn submit_code(State(s): State<AppState>, admin: AuthUser, Path(id): Path<Uuid>, Json(b): Json<Value>) -> Result<StatusCode, ApiError> {
    require_admin(&admin)?;
    let code = b.get("code").and_then(Value::as_str).unwrap_or("").trim().to_string();
    if !(1..=256).contains(&code.len()) || code.chars().any(char::is_whitespace) {
        return Err(ApiError::BadRequest("invalid_code"));
    }
    let client = db(&s).await?;
    let n = client
        .execute(
            "UPDATE cli_sessions SET pasted_code = $2 WHERE id = $1 AND state = 'awaiting_code'",
            &[&id, &code],
        )
        .await?;
    if n == 0 {
        return Err(ApiError::Conflict("not_awaiting_code"));
    }
    audit(&client, &admin.email, "cli_session_code", &id.to_string(), "sent").await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Próxima sessão pendente das contas deste executor. Marca como em execução (uma entrega por sessão).
async fn next_session(State(s): State<AppState>, ex: ExecutorAuth) -> Result<Json<Value>, ApiError> {
    let client = db(&s).await?;
    // sessões vencidas nunca são entregues
    client
        .execute(
            "UPDATE cli_sessions SET state = 'timed_out', finished_at = now() \
             WHERE state IN ('pending','running','awaiting_code') AND expires_at < now()",
            &[],
        )
        .await?;
    let row = client
        .query_opt(
            "UPDATE cli_sessions SET state = 'running', started_at = now() \
             WHERE id = (SELECT s.id FROM cli_sessions s JOIN cli_connections c ON c.id = s.connection_id \
                         WHERE c.executor_id = $1 AND s.state = 'pending' AND NOT s.cancel_requested \
                         ORDER BY s.created_at LIMIT 1 FOR UPDATE SKIP LOCKED) \
             RETURNING id, action, connection_id, (SELECT engine FROM cli_connections WHERE id = connection_id)",
            &[&ex.id],
        )
        .await?;
    let Some(r) = row else {
        return Ok(Json(json!({ "session": null })));
    };
    Ok(Json(json!({ "session": {
        "id": r.get::<_, Uuid>(0),
        "action": r.get::<_, String>(1),
        "connection_id": r.get::<_, Uuid>(2),
        "engine": r.get::<_, String>(3),
    } })))
}

/// Controle de uma sessão em execução: cancelamento e código colado (entregue uma vez).
async fn session_control(State(s): State<AppState>, ex: ExecutorAuth, Path(id): Path<Uuid>) -> Result<Json<Value>, ApiError> {
    let client = db(&s).await?;
    let row = owned_session(&client, ex.id, id).await?;
    let (cancel, state): (bool, String) = (row.get(0), row.get(1));
    let expired = client
        .query_one("SELECT expires_at < now() FROM cli_sessions WHERE id = $1", &[&id])
        .await?
        .get::<_, bool>(0);
    if expired && (state == "running" || state == "awaiting_code") {
        client
            .execute("UPDATE cli_sessions SET state = 'timed_out', finished_at = now() WHERE id = $1", &[&id])
            .await?;
        return Ok(Json(json!({ "cancel": true, "pasted_code": null })));
    }
    Ok(Json(json!({ "cancel": cancel, "pasted_code": take_pasted(&client, id).await? })))
}

/// Entrega o código colado e o apaga na mesma operação: ele sai do banco no instante da entrega.
async fn take_pasted<C: deadpool_postgres::GenericClient>(client: &C, id: Uuid) -> Result<Option<String>, ApiError> {
    let row = client
        .query_opt(
            "WITH old AS (SELECT id, pasted_code FROM cli_sessions WHERE id = $1 FOR UPDATE) \
             UPDATE cli_sessions s SET pasted_code = NULL FROM old WHERE s.id = old.id RETURNING old.pasted_code",
            &[&id],
        )
        .await?;
    Ok(row.and_then(|r| r.get::<_, Option<String>>(0)))
}

async fn owned_session<C: deadpool_postgres::GenericClient>(
    client: &C,
    executor_id: Uuid,
    session_id: Uuid,
) -> Result<tokio_postgres::Row, ApiError> {
    client
        .query_opt(
            "SELECT s.cancel_requested, s.state FROM cli_sessions s JOIN cli_connections c ON c.id = s.connection_id \
             WHERE s.id = $1 AND c.executor_id = $2",
            &[&session_id, &executor_id],
        )
        .await?
        .ok_or(ApiError::NotFound)
}

/// Eventos do executor: link oficial, código de dispositivo, aguardando código, ou fim com o estado da conta.
async fn session_event(State(s): State<AppState>, ex: ExecutorAuth, Path(id): Path<Uuid>, Json(b): Json<Value>) -> Result<StatusCode, ApiError> {
    let client = db(&s).await?;
    let owned = client
        .query_opt(
            "SELECT c.engine, c.id FROM cli_sessions s JOIN cli_connections c ON c.id = s.connection_id \
             WHERE s.id = $1 AND c.executor_id = $2",
            &[&id, &ex.id],
        )
        .await?
        .ok_or(ApiError::NotFound)?;
    let engine: String = owned.get(0);
    let connection_id: Uuid = owned.get(1);
    let kind = b.get("kind").and_then(Value::as_str).unwrap_or("");
    match kind {
        "url" => {
            let url = b.get("url").and_then(Value::as_str).unwrap_or("");
            if !official_url(&engine, url) {
                return Err(ApiError::BadRequest("url_not_official"));
            }
            client
                .execute("UPDATE cli_sessions SET url = $2 WHERE id = $1 AND state IN ('running','awaiting_code')", &[&id, &url])
                .await?;
        }
        "device_code" => {
            let code = b.get("code").and_then(Value::as_str).unwrap_or("");
            if !device_code_ok(code) {
                return Err(ApiError::BadRequest("invalid_device_code"));
            }
            client
                .execute("UPDATE cli_sessions SET device_code = $2 WHERE id = $1 AND state IN ('running','awaiting_code')", &[&id, &code])
                .await?;
        }
        "awaiting_code" => {
            client
                .execute("UPDATE cli_sessions SET state = 'awaiting_code' WHERE id = $1 AND state = 'running'", &[&id])
                .await?;
        }
        "finished" => {
            let state = b.get("state").and_then(Value::as_str).unwrap_or("");
            if !["succeeded", "failed", "cancelled", "timed_out"].contains(&state) {
                return Err(ApiError::BadRequest("invalid_state"));
            }
            let auth = b.get("auth").and_then(Value::as_str).unwrap_or("unknown");
            if !["authenticated", "unauthenticated", "unknown"].contains(&auth) {
                return Err(ApiError::BadRequest("invalid_auth"));
            }
            let result = b.get("result").and_then(Value::as_str).map(|r| r.chars().take(200).collect::<String>());
            client
                .execute(
                    "UPDATE cli_sessions SET state = $2, finished_at = now(), url = NULL, device_code = NULL, pasted_code = NULL, result = $3 \
                     WHERE id = $1",
                    &[&id, &state, &result],
                )
                .await?;
            // o estado da conta só muda com uma verificação real (status); "unknown" não apaga o último estado conhecido
            if auth != "unknown" {
                client
                    .execute(
                        "UPDATE cli_connections SET auth_state = $2, auth_checked_at = now(), updated_at = now() WHERE id = $1",
                        &[&connection_id, &auth],
                    )
                    .await?;
            }
        }
        _ => return Err(ApiError::BadRequest("invalid_event")),
    }
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_official_https_urls_are_accepted_per_engine() {
        assert!(official_url("codex", "https://auth.openai.com/codex/device"));
        assert!(official_url("claude", "https://claude.ai/oauth/authorize?x=1"));
        assert!(official_url("cursor", "https://cursor.com/loginDeepControl?x=1"));
        assert!(!official_url("codex", "http://auth.openai.com/codex/device"));
        assert!(!official_url("codex", "https://claude.ai/x"));
        assert!(!official_url("codex", "https://auth.openai.com.evil.test/x"));
        assert!(!official_url("bash", "https://auth.openai.com/x"));
    }

    #[test]
    fn device_codes_have_the_expected_shape_only() {
        assert!(device_code_ok("ABCD-1234"));
        assert!(device_code_ok("ABCDE-12345"));
        assert!(!device_code_ok("abcd-1234"));
        assert!(!device_code_ok("ABCD1234"));
        assert!(!device_code_ok("ABCD-12345678"));
    }
}
