//! Rotas do OneHub Central. Cada operação valida identidade, papel, dono do recurso e concessão
//! (OneNode × projeto). Quem não tem acesso recebe "não encontrado", sem revelar se o recurso existe.

use axum::extract::{FromRequestParts, Path, State};
use axum::http::{header, request::Parts, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use deadpool_postgres::GenericClient;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio_postgres::Row;
use tower_http::cors::{AllowOrigin, CorsLayer};
use uuid::Uuid;

use std::sync::Arc;

use crate::auth::{hash_password, new_secret, sha256, verify_password};
use crate::db::DbPool;
use crate::orchestrator::{
    parse_command, tool_for_path, Command, InferenceOutcome, InferenceProvider, UnavailableProvider,
};

pub const SESSION_TTL_HOURS: i32 = 12;
pub const PAIRING_TTL_MINUTES: i32 = 15;
pub const LEASE_SECONDS: f64 = 300.0;
pub const ONLINE_WINDOW_SECONDS: i64 = 90;
/// Bloqueio de força bruta: falhas por e-mail dentro da janela. Vale também para e-mails inexistentes.
pub const LOGIN_MAX_FAILURES: i64 = 5;
/// Limite por origem (IP do cliente) na mesma janela. Maior que o da conta, para não bloquear um escritório inteiro
/// por causa de uma conta; ainda assim trava varreduras de muitos e-mails a partir de uma mesma origem.
pub const LOGIN_MAX_ORIGIN_FAILURES: i64 = 30;
pub const LOGIN_WINDOW_MINUTES: i32 = 15;
const MAX_PROMPT_CHARS: usize = 16 * 1024;
const MAX_EVENT_BYTES: usize = 16 * 1024;
const MAX_RESULT_BYTES: usize = 64 * 1024;

#[derive(Clone)]
pub struct AppState {
    pub pool: DbPool,
    pub provider: Arc<dyn InferenceProvider>,
    pub settings: Settings,
}

/// Política de sessão e de origem. Padrões seguros: cookie `Secure` e nenhuma origem cross-site.
#[derive(Clone)]
pub struct Settings {
    /// `Secure` no cookie de sessão. Só desligar para desenvolvimento em http://localhost.
    pub cookie_secure: bool,
    /// Origens cross-site autorizadas (CORS e CSRF). A mesma origem (Origin == Host) é sempre aceita.
    pub allowed_origins: Vec<String>,
    /// IPs de proxies reversos confiáveis (ex.: o Caddy). `X-Forwarded-For` só vale quando a conexão vem de um
    /// desses IPs. Vazio por padrão: o cabeçalho é ignorado.
    pub trusted_proxies: Vec<std::net::IpAddr>,
    /// Chave-mestra das API Keys de provedores (fora do banco). Ausente: esses endpoints respondem 503.
    pub provider_key: Option<[u8; 32]>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            cookie_secure: true,
            allowed_origins: Vec::new(),
            trusted_proxies: Vec::new(),
            provider_key: None,
        }
    }
}

pub const SESSION_COOKIE: &str = "oh_session";

pub fn session_cookie(value: &str, secure: bool, max_age_secs: i64) -> String {
    let mut c = format!(
        "{SESSION_COOKIE}={value}; Path=/; HttpOnly; SameSite=Strict; Max-Age={max_age_secs}"
    );
    if secure {
        c.push_str("; Secure");
    }
    c
}

fn cookie_token(parts: &Parts) -> Option<String> {
    parts
        .headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .find_map(|kv| {
            kv.trim()
                .strip_prefix(SESSION_COOKIE)
                .and_then(|r| r.strip_prefix('='))
                .filter(|t| !t.is_empty())
                .map(String::from)
        })
}

/// Anti-CSRF para sessão por cookie: a origem precisa ser a mesma do Host ou estar na lista autorizada.
fn origin_allowed(parts: &Parts, allowed: &[String]) -> bool {
    let Some(origin) = parts
        .headers
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
    else {
        return false;
    };
    if allowed.iter().any(|a| a == origin) {
        return true;
    }
    let host = parts
        .headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok());
    matches!((host, origin.split_once("://")), (Some(h), Some((_, o))) if h == o)
}

fn is_safe_method(m: &axum::http::Method) -> bool {
    matches!(
        *m,
        axum::http::Method::GET | axum::http::Method::HEAD | axum::http::Method::OPTIONS
    )
}

#[derive(Debug)]
pub enum ApiError {
    BadRequest(&'static str),
    Unauthorized,
    TooManyAttempts,
    Forbidden(&'static str),
    NotFound,
    Conflict(&'static str),
    /// OneNode suspenso pelo administrador: o nó recua e tenta de novo, sem parar o serviço.
    Locked(&'static str),
    /// Recurso indisponível por configuração do servidor (ex.: chave-mestra ausente). O serviço continua de pé.
    Unavailable(&'static str),
    Internal,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code) = match self {
            ApiError::BadRequest(c) => (StatusCode::BAD_REQUEST, c),
            ApiError::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized"),
            ApiError::TooManyAttempts => (StatusCode::TOO_MANY_REQUESTS, "too_many_attempts"),
            ApiError::Forbidden(c) => (StatusCode::FORBIDDEN, c),
            ApiError::NotFound => (StatusCode::NOT_FOUND, "not_found"),
            ApiError::Conflict(c) => (StatusCode::CONFLICT, c),
            ApiError::Locked(c) => (StatusCode::LOCKED, c),
            ApiError::Unavailable(c) => (StatusCode::SERVICE_UNAVAILABLE, c),
            ApiError::Internal => (StatusCode::INTERNAL_SERVER_ERROR, "internal"),
        };
        (status, Json(json!({ "error": code }))).into_response()
    }
}

impl From<tokio_postgres::Error> for ApiError {
    fn from(_: tokio_postgres::Error) -> Self {
        // Sem detalhes do banco na resposta nem no log.
        eprintln!("erro de banco (detalhes omitidos)");
        ApiError::Internal
    }
}

impl From<deadpool_postgres::PoolError> for ApiError {
    fn from(_: deadpool_postgres::PoolError) -> Self {
        eprintln!("pool de conexões indisponível");
        ApiError::Internal
    }
}

pub(crate) type Db = deadpool_postgres::Object;

pub(crate) async fn db(state: &AppState) -> Result<Db, ApiError> {
    Ok(state.pool.get().await?)
}

pub(crate) fn bearer(parts: &Parts) -> Option<String> {
    parts
        .headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(|t| t.trim().to_string())
}

/// Usuário com sessão válida (não revogada, não expirada, conta ativa). Aceita o cookie HttpOnly do
/// navegador ou o token em `Authorization` (clientes que não são navegador). Sessão por cookie exige
/// origem autorizada em requisições que alteram estado.
pub struct AuthUser {
    pub id: Uuid,
    pub email: String,
    pub is_admin: bool,
    /// Hash da sessão usada nesta requisição (o stream revalida essa mesma sessão).
    pub session: Vec<u8>,
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let (token, from_cookie) = match bearer(parts) {
            Some(t) => (t, false),
            None => (cookie_token(parts).ok_or(ApiError::Unauthorized)?, true),
        };
        if from_cookie
            && !is_safe_method(&parts.method)
            && !origin_allowed(parts, &state.settings.allowed_origins)
        {
            return Err(ApiError::Forbidden("origin_not_allowed"));
        }
        let hash = sha256(&token);
        let client = db(state).await?;
        let row = client
            .query_opt(
                "SELECT u.id, u.email, u.role FROM sessions s JOIN users u ON u.id = s.user_id \
                 WHERE s.token_hash = $1 AND s.revoked_at IS NULL AND s.expires_at > now() AND NOT u.disabled",
                &[&hash],
            )
            .await?
            .ok_or(ApiError::Unauthorized)?;
        let role: String = row.get(2);
        Ok(AuthUser {
            id: row.get(0),
            email: row.get(1),
            is_admin: role == "admin",
            session: hash,
        })
    }
}

/// OneNode autenticado pelo próprio token (emitido no pareamento). Revogado = recusado.
pub struct NodeAuth {
    pub id: Uuid,
}

impl FromRequestParts<AppState> for NodeAuth {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let token = bearer(parts).ok_or(ApiError::Unauthorized)?;
        let client = db(state).await?;
        let row = client
            .query_opt(
                "SELECT id, suspended_at IS NOT NULL FROM onenodes WHERE token_hash = $1 AND revoked_at IS NULL",
                &[&sha256(&token)],
            )
            .await?
            .ok_or(ApiError::Unauthorized)?;
        if row.get::<_, bool>(1) {
            return Err(ApiError::Locked("node_suspended"));
        }
        Ok(NodeAuth { id: row.get(0) })
    }
}

pub(crate) fn require_admin(user: &AuthUser) -> Result<(), ApiError> {
    if user.is_admin {
        Ok(())
    } else {
        Err(ApiError::Forbidden("admin_required"))
    }
}

pub(crate) async fn audit<C: GenericClient>(
    client: &C,
    actor: &str,
    action: &str,
    subject: &str,
    outcome: &str,
) -> Result<(), ApiError> {
    client
        .execute(
            "INSERT INTO audit_log(actor, action, subject, outcome) VALUES ($1, $2, $3, $4)",
            &[&actor, &action, &subject, &outcome],
        )
        .await?;
    Ok(())
}

pub(crate) fn text_len_ok(s: &str, min: usize, max: usize) -> bool {
    let n = s.trim().chars().count();
    n >= min && n <= max
}

#[derive(Deserialize)]
pub struct Credentials {
    pub email: String,
    pub password: String,
}

pub fn router(pool: DbPool) -> Router {
    router_with(pool, Arc::new(UnavailableProvider))
}

/// Roteador com um provedor de IA específico (o padrão é indisponível).
pub fn router_with(pool: DbPool, provider: Arc<dyn InferenceProvider>) -> Router {
    router_with_settings(pool, provider, Settings::default())
}

/// Roteador completo: provedor, política de sessão e origens. CORS só para origens listadas (sem curinga).
pub fn router_with_settings(
    pool: DbPool,
    provider: Arc<dyn InferenceProvider>,
    settings: Settings,
) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::list(
            settings
                .allowed_origins
                .iter()
                .filter_map(|o| HeaderValue::from_str(o).ok()),
        ))
        .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE])
        .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION])
        .allow_credentials(true);
    let has_origins = !settings.allowed_origins.is_empty();
    let app = Router::new()
        .route("/health", get(health))
        .route("/v1/login", post(login))
        .route("/v1/logout", post(logout))
        .route("/v1/me", get(me))
        .route("/v1/users", post(create_user))
        .route("/v1/onenodes", get(list_onenodes).post(create_onenode))
        .route("/v1/onenodes/{id}", get(get_onenode))
        .route("/v1/onenodes/{id}/revoke", post(revoke_onenode))
        .route("/v1/onenodes/{id}/suspend", post(suspend_onenode))
        .route("/v1/onenodes/{id}/resume", post(resume_onenode))
        .route("/node/v1/catalog", get(node_catalog))
        .route("/node/v1/catalog/applied", post(node_catalog_applied))
        .route("/v1/projects", get(list_projects).post(create_project))
        .route("/v1/projects/{id}/grants", post(grant_project))
        .route(
            "/v1/projects/{id}/grants/{node_id}/revoke",
            post(revoke_grant),
        )
        .route(
            "/v1/conversations",
            get(list_conversations).post(create_conversation),
        )
        .route(
            "/v1/conversations/{id}/messages",
            get(list_messages).post(add_message),
        )
        .route("/v1/conversations/{id}/turns", post(turn))
        .route("/v1/conversations/{id}/context", put(set_context))
        .route(
            "/v1/provider-connections",
            get(list_provider_connections).post(create_provider_connection),
        )
        .route(
            "/v1/provider-connections/{id}",
            axum::routing::delete(delete_provider_connection),
        )
        .route("/v1/tasks", get(list_tasks).post(create_task))
        .route("/v1/grants", get(list_grants))
        .route("/v1/tasks/{id}", get(get_task))
        .route("/node/v1/pair", post(pair_node))
        .route("/node/v1/heartbeat", post(heartbeat))
        .route("/node/v1/tasks/next", get(next_task))
        .route("/node/v1/tasks/{id}/events", post(task_event))
        .route("/node/v1/tasks/{id}/finish", post(finish_task))
        .merge(crate::executor::routes())
        .merge(crate::cli_auth::routes())
        .merge(crate::events::routes())
        .with_state(AppState {
            pool,
            provider,
            settings,
        });
    if has_origins {
        app.layer(cors)
    } else {
        app
    }
}

async fn health(State(s): State<AppState>) -> (StatusCode, Json<Value>) {
    let ok = match db(&s).await {
        Ok(c) => c.query_one("SELECT 1", &[]).await.is_ok(),
        Err(_) => false,
    };
    if ok {
        (
            StatusCode::OK,
            Json(json!({ "status": "ok", "database": "ok" })),
        )
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "status": "degraded", "database": "unavailable" })),
        )
    }
}

/// IP usado para o limite por origem. `X-Forwarded-For` só é considerado quando a conexão vem de um proxy
/// confiável. Nesse caso vale o último endereço: o proxy acrescenta o IP real do cliente ao fim da lista, e o
/// que o cliente enviou fica antes dele. Qualquer outro caso usa o IP do próprio socket, então um cabeçalho
/// forjado não escapa do limite.
pub fn client_origin(
    peer: Option<std::net::SocketAddr>,
    headers: &axum::http::HeaderMap,
    trusted_proxies: &[std::net::IpAddr],
) -> String {
    let peer_ip = peer.map(|p| p.ip());
    if peer_ip.is_some_and(|ip| trusted_proxies.contains(&ip)) {
        let forwarded = headers
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.rsplit(',').next())
            .and_then(|v| v.trim().parse::<std::net::IpAddr>().ok());
        if let Some(ip) = forwarded {
            return ip.to_string();
        }
    }
    peer_ip
        .map(|ip| ip.to_string())
        .unwrap_or_else(|| "desconhecida".into())
}

/// Endereço do socket de quem conecta, quando o servidor o informa (ausente em testes).
pub struct PeerAddr(pub Option<std::net::SocketAddr>);

impl<S: Send + Sync> FromRequestParts<S> for PeerAddr {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        let addr = parts
            .extensions
            .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
            .map(|c| c.0);
        Ok(PeerAddr(addr))
    }
}

async fn login(
    State(s): State<AppState>,
    PeerAddr(peer): PeerAddr,
    headers: axum::http::HeaderMap,
    Json(b): Json<Credentials>,
) -> Result<Response, ApiError> {
    let email = b.email.trim().to_lowercase();
    let origin = client_origin(peer, &headers, &s.settings.trusted_proxies);
    let mut conn = db(&s).await?;
    // Transação com locks por origem e por conta (sempre nesta ordem): tentativas concorrentes ficam em fila,
    // então contagem, verificação e registro da falha acontecem sem corrida.
    let tx = conn.transaction().await?;
    tx.execute(
        "SELECT pg_advisory_xact_lock(hashtext($1))",
        &[&format!("login-origin:{origin}")],
    )
    .await?;
    tx.execute(
        "SELECT pg_advisory_xact_lock(hashtext($1))",
        &[&format!("login-email:{email}")],
    )
    .await?;
    let recent: i64 = tx
        .query_one(
            "SELECT count(*) FROM login_failures WHERE email = $1 AND failed_at > now() - make_interval(mins => $2)",
            &[&email, &LOGIN_WINDOW_MINUTES],
        )
        .await?
        .get(0);
    let from_origin: i64 = tx
        .query_one(
            "SELECT count(*) FROM login_failures WHERE origin = $1 AND failed_at > now() - make_interval(mins => $2)",
            &[&origin, &LOGIN_WINDOW_MINUTES],
        )
        .await?
        .get(0);
    if recent >= LOGIN_MAX_FAILURES || from_origin >= LOGIN_MAX_ORIGIN_FAILURES {
        audit(&tx, &email, "login", &email, "locked").await?;
        tx.commit().await?;
        return Err(ApiError::TooManyAttempts);
    }
    let row: Option<Row> = tx
        .query_opt(
            "SELECT id, password_hash, disabled FROM users WHERE email = $1",
            &[&email],
        )
        .await?;
    let ok = match &row {
        Some(r) => verify_password(&b.password, r.get::<_, &str>(1)) && !r.get::<_, bool>(2),
        None => {
            // Mesmo custo de Argon2 para e-mail inexistente: não revela quais contas existem.
            let _ = hash_password(&b.password);
            false
        }
    };
    let Some(row) = row.filter(|_| ok) else {
        tx.execute(
            "INSERT INTO login_failures(email, origin) VALUES ($1, $2)",
            &[&email, &origin],
        )
        .await?;
        audit(&tx, &email, "login", &email, "denied").await?;
        tx.commit().await?;
        return Err(ApiError::Unauthorized);
    };
    tx.execute("DELETE FROM login_failures WHERE email = $1", &[&email])
        .await?;
    let user_id: Uuid = row.get(0);
    let token = new_secret();
    let expires: DateTime<Utc> = tx
        .query_one(
            "INSERT INTO sessions(user_id, token_hash, expires_at) VALUES ($1, $2, now() + make_interval(hours => $3)) RETURNING expires_at",
            &[&user_id, &sha256(&token), &SESSION_TTL_HOURS],
        )
        .await?
        .get(0);
    audit(&tx, &email, "login", &user_id.to_string(), "ok").await?;
    tx.commit().await?;
    let cookie = session_cookie(
        &token,
        s.settings.cookie_secure,
        i64::from(SESSION_TTL_HOURS) * 3600,
    );
    // Navegador (envia Origin) recebe só o cookie HttpOnly. Clientes sem navegador (executor, nó, API) recebem o
    // token no corpo, como antes.
    let body = if headers.get(header::ORIGIN).is_some() {
        json!({ "expires_at": expires })
    } else {
        json!({ "token": token, "expires_at": expires })
    };
    Ok(([(header::SET_COOKIE, cookie)], Json(body)).into_response())
}

async fn logout(State(s): State<AppState>, user: AuthUser) -> Result<Response, ApiError> {
    let client = db(&s).await?;
    client
        .execute(
            "UPDATE sessions SET revoked_at = now() WHERE user_id = $1 AND revoked_at IS NULL",
            &[&user.id],
        )
        .await?;
    audit(&client, &user.email, "logout", &user.id.to_string(), "ok").await?;
    // remove o cookie do navegador (mesmos atributos, validade zero)
    let cleared = session_cookie("", s.settings.cookie_secure, 0);
    Ok(([(header::SET_COOKIE, cleared)], StatusCode::NO_CONTENT).into_response())
}

async fn me(user: AuthUser) -> Json<Value> {
    Json(
        json!({ "id": user.id, "email": user.email, "role": if user.is_admin { "admin" } else { "member" } }),
    )
}

async fn create_user(
    State(s): State<AppState>,
    admin: AuthUser,
    Json(b): Json<Value>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    require_admin(&admin)?;
    let email = b
        .get("email")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_lowercase();
    let password = b.get("password").and_then(Value::as_str).unwrap_or("");
    let role = b.get("role").and_then(Value::as_str).unwrap_or("member");
    if !text_len_ok(&email, 3, 254) || password.chars().count() < 12 {
        return Err(ApiError::BadRequest("invalid_user"));
    }
    if role != "member" && role != "admin" {
        return Err(ApiError::BadRequest("invalid_role"));
    }
    let hash = hash_password(password).map_err(|_| ApiError::Internal)?;
    let client = db(&s).await?;
    let id: Uuid = match client
        .query_one(
            "INSERT INTO users(email, password_hash, role) VALUES ($1, $2, $3) RETURNING id",
            &[&email, &hash, &role],
        )
        .await
    {
        Ok(r) => r.get(0),
        Err(_) => return Err(ApiError::Conflict("email_taken")),
    };
    audit(&client, &admin.email, "user_create", &id.to_string(), "ok").await?;
    Ok((StatusCode::CREATED, Json(json!({ "id": id, "role": role }))))
}

/// Registra um OneNode e devolve o código de pareamento de uso único (exibido uma vez).
async fn create_onenode(
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
            "INSERT INTO onenodes(name, pairing_hash, pairing_expires) VALUES ($1, $2, now() + make_interval(mins => $3)) \
             RETURNING id, pairing_expires",
            &[&name, &sha256(&code), &PAIRING_TTL_MINUTES],
        )
        .await?;
    let id: Uuid = row.get(0);
    let expires: DateTime<Utc> = row.get(1);
    audit(
        &client,
        &admin.email,
        "onenode_create",
        &id.to_string(),
        "ok",
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "id": id, "pairing_code": code, "pairing_expires_at": expires })),
    ))
}

const PROVIDERS: [&str; 4] = ["openrouter", "openai", "gemini", "anthropic"];

fn provider_master_key(s: &AppState) -> Result<[u8; 32], ApiError> {
    s.settings
        .provider_key
        .ok_or(ApiError::Unavailable("provider_key_missing"))
}

/// Conexões de provedor sem nenhum segredo: só a dica dos últimos caracteres.
async fn list_provider_connections(
    State(s): State<AppState>,
    admin: AuthUser,
) -> Result<Json<Value>, ApiError> {
    require_admin(&admin)?;
    let client = db(&s).await?;
    let rows = client
        .query(
            "SELECT id, name, provider, key_hint, default_model, status, last_checked_at, created_at \
             FROM provider_connections ORDER BY created_at",
            &[],
        )
        .await?;
    let items: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.get::<_, Uuid>(0),
                "name": r.get::<_, String>(1),
                "provider": r.get::<_, String>(2),
                "key_hint": r.get::<_, String>(3),
                "default_model": r.get::<_, Option<String>>(4),
                "status": r.get::<_, String>(5),
                "last_checked_at": r.get::<_, Option<DateTime<Utc>>>(6),
                "created_at": r.get::<_, DateTime<Utc>>(7),
            })
        })
        .collect();
    Ok(Json(json!({ "connections": items })))
}

async fn create_provider_connection(
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
    let provider = b.get("provider").and_then(Value::as_str).unwrap_or("");
    if !PROVIDERS.contains(&provider) {
        return Err(ApiError::BadRequest("invalid_provider"));
    }
    let api_key = b
        .get("api_key")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if !(16..=512).contains(&api_key.len()) || api_key.chars().any(char::is_whitespace) {
        return Err(ApiError::BadRequest("invalid_api_key"));
    }
    let default_model = b
        .get("default_model")
        .and_then(Value::as_str)
        .map(|m| m.trim().to_string())
        .filter(|m| !m.is_empty());
    if let Some(m) = &default_model {
        if !text_len_ok(m, 1, 120) {
            return Err(ApiError::BadRequest("invalid_model"));
        }
    }
    let master = provider_master_key(&s)?;
    let (ciphertext, nonce) =
        crate::provider_keys::seal(&master, provider, &api_key).map_err(|_| ApiError::Internal)?;
    let hint = crate::provider_keys::hint(&api_key);
    let client = db(&s).await?;
    let row = client
        .query_one(
            "INSERT INTO provider_connections(name, provider, key_ciphertext, key_nonce, key_hint, default_model, created_by) \
             VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING id, created_at",
            &[&name, &provider, &ciphertext, &nonce, &hint, &default_model, &admin.email],
        )
        .await?;
    let id: Uuid = row.get(0);
    audit(
        &client,
        &admin.email,
        "provider_connection_create",
        &id.to_string(),
        "ok",
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "id": id,
            "name": name,
            "provider": provider,
            "key_hint": hint,
            "default_model": default_model,
            "status": "unverified",
        })),
    ))
}

async fn delete_provider_connection(
    State(s): State<AppState>,
    admin: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    require_admin(&admin)?;
    let client = db(&s).await?;
    let removed = client
        .execute("DELETE FROM provider_connections WHERE id = $1", &[&id])
        .await?;
    if removed == 0 {
        return Err(ApiError::NotFound);
    }
    audit(
        &client,
        &admin.email,
        "provider_connection_delete",
        &id.to_string(),
        "ok",
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn pair_node(
    State(s): State<AppState>,
    Json(b): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let id = b
        .get("node_id")
        .and_then(Value::as_str)
        .and_then(|v| Uuid::parse_str(v).ok())
        .ok_or(ApiError::BadRequest("invalid_node_id"))?;
    let code = b.get("code").and_then(Value::as_str).unwrap_or("");
    let token = new_secret();
    let client = db(&s).await?;
    // Uso único: consome o pareamento só se código, validade e revogação conferirem.
    let row = client
        .query_opt(
            "UPDATE onenodes SET pairing_hash = NULL, pairing_expires = NULL, token_hash = $3, paired_at = now(), last_seen_at = now() \
             WHERE id = $1 AND pairing_hash = $2 AND pairing_expires > now() AND revoked_at IS NULL RETURNING id",
            &[&id, &sha256(code), &sha256(&token)],
        )
        .await?
        .ok_or(ApiError::Unauthorized)?;
    let node_id: Uuid = row.get(0);
    audit(&client, "onenode", "pair", &node_id.to_string(), "ok").await?;
    Ok(Json(json!({ "node_id": node_id, "token": token })))
}

async fn heartbeat(State(s): State<AppState>, node: NodeAuth) -> Result<Json<Value>, ApiError> {
    let client = db(&s).await?;
    client
        .execute(
            "UPDATE onenodes SET last_seen_at = now() WHERE id = $1",
            &[&node.id],
        )
        .await?;
    Ok(Json(json!({ "ok": true, "server_time": Utc::now() })))
}

fn node_status(revoked: bool, paired: bool, last_seen: Option<DateTime<Utc>>) -> &'static str {
    if revoked {
        "revoked"
    } else if !paired {
        "pending"
    } else if last_seen.is_some_and(|t| (Utc::now() - t).num_seconds() < ONLINE_WINDOW_SECONDS) {
        "online"
    } else {
        "offline"
    }
}

async fn list_onenodes(
    State(s): State<AppState>,
    _user: AuthUser,
) -> Result<Json<Value>, ApiError> {
    let client = db(&s).await?;
    let rows = client
        .query(
            "SELECT id, name, paired_at, last_seen_at, revoked_at, suspended_at, access_type, catalog_version, \
             applied_version, applied_at FROM onenodes ORDER BY created_at",
            &[],
        )
        .await?;
    let nodes: Vec<Value> = rows
        .iter()
        .map(|r| {
            let paired: Option<DateTime<Utc>> = r.get(2);
            let last: Option<DateTime<Utc>> = r.get(3);
            let revoked: Option<DateTime<Utc>> = r.get(4);
            let suspended: Option<DateTime<Utc>> = r.get(5);
            let catalog_version: i64 = r.get(7);
            let applied_version: Option<i64> = r.get(8);
            json!({
                "id": r.get::<_, Uuid>(0),
                "name": r.get::<_, String>(1),
                "status": node_status(revoked.is_some(), paired.is_some(), last),
                "last_seen_at": last,
                "suspended": suspended.is_some(),
                "access_type": r.get::<_, String>(6),
                // o nó já aplicou o catálogo atual quando as duas versões coincidem
                "catalog_in_sync": applied_version == Some(catalog_version),
                "applied_at": r.get::<_, Option<DateTime<Utc>>>(9),
            })
        })
        .collect();
    Ok(Json(json!(nodes)))
}

async fn suspend_onenode(State(s): State<AppState>, admin: AuthUser, Path(id): Path<Uuid>) -> Result<StatusCode, ApiError> {
    set_node_suspended(&s, &admin, id, true).await
}

async fn resume_onenode(State(s): State<AppState>, admin: AuthUser, Path(id): Path<Uuid>) -> Result<StatusCode, ApiError> {
    set_node_suspended(&s, &admin, id, false).await
}

/// Suspender bloqueia novas chamadas do nó (423) e bumpa o catálogo para que ele reaplique o estado.
/// Não revoga o token: reativar restaura o acesso sem novo pareamento.
async fn set_node_suspended(s: &AppState, admin: &AuthUser, id: Uuid, suspend: bool) -> Result<StatusCode, ApiError> {
    require_admin(admin)?;
    let mut client = db(s).await?;
    let tx = client.transaction().await?;
    let sql = if suspend {
        "UPDATE onenodes SET suspended_at = now(), catalog_version = catalog_version + 1 \
         WHERE id = $1 AND revoked_at IS NULL AND suspended_at IS NULL"
    } else {
        "UPDATE onenodes SET suspended_at = NULL, catalog_version = catalog_version + 1 \
         WHERE id = $1 AND revoked_at IS NULL AND suspended_at IS NOT NULL"
    };
    if tx.execute(sql, &[&id]).await? == 0 {
        return Err(ApiError::NotFound);
    }
    tx.commit().await?;
    audit(
        &client,
        &admin.email,
        if suspend { "node_suspend" } else { "node_resume" },
        &id.to_string(),
        "ok",
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Catálogo do próprio nó: só concessões ativas com raiz pedida. Projeto sem concessão não aparece.
async fn node_catalog(State(s): State<AppState>, node: NodeAuth) -> Result<Json<Value>, ApiError> {
    let client = db(&s).await?;
    let version: i64 = client
        .query_one("SELECT catalog_version FROM onenodes WHERE id = $1", &[&node.id])
        .await?
        .get(0);
    let rows = client
        .query(
            "SELECT p.id, p.name, g.root_path FROM node_project_grants g JOIN projects p ON p.id = g.project_id \
             WHERE g.node_id = $1 AND g.revoked_at IS NULL AND g.root_path IS NOT NULL ORDER BY p.name",
            &[&node.id],
        )
        .await?;
    let projects: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "project_id": r.get::<_, Uuid>(0),
                "name": r.get::<_, String>(1),
                "root_path": r.get::<_, String>(2),
            })
        })
        .collect();
    Ok(Json(json!({ "version": version, "projects": projects })))
}

#[derive(Deserialize, serde::Serialize)]
pub struct Rejected {
    pub project_id: Uuid,
    pub reason: String,
}

#[derive(Deserialize)]
pub struct AppliedBody {
    pub version: i64,
    pub accepted: Vec<Uuid>,
    pub rejected: Vec<Rejected>,
}

/// O nó confirma a versão efetivamente aplicada. Versão antiga (o catálogo mudou no meio) é recusada.
async fn node_catalog_applied(
    State(s): State<AppState>,
    node: NodeAuth,
    Json(b): Json<AppliedBody>,
) -> Result<StatusCode, ApiError> {
    if b.accepted.len() > 200 || b.rejected.len() > 200 || b.rejected.iter().any(|r| r.reason.len() > 120) {
        return Err(ApiError::BadRequest("report_too_large"));
    }
    let client = db(&s).await?;
    let report = json!({ "accepted": b.accepted, "rejected": b.rejected });
    let done = client
        .execute(
            "UPDATE onenodes SET applied_version = $2, applied_at = now(), applied_report = $3 \
             WHERE id = $1 AND catalog_version = $2",
            &[&node.id, &b.version, &report],
        )
        .await?;
    if done == 0 {
        return Err(ApiError::Conflict("catalog_changed"));
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn revoke_onenode(
    State(s): State<AppState>,
    admin: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    require_admin(&admin)?;
    let mut client = db(&s).await?;
    let tx = client.transaction().await?;
    let done = tx
        .execute("UPDATE onenodes SET revoked_at = now(), token_hash = NULL WHERE id = $1 AND revoked_at IS NULL", &[&id])
        .await?;
    if done == 0 {
        return Err(ApiError::NotFound);
    }
    // Tarefas ainda não concluídas deste OneNode são canceladas: nada novo é entregue a ele.
    tx.execute(
        "UPDATE tasks SET status = 'cancelled', lease_expires = NULL, lease_token = NULL, updated_at = now() \
         WHERE node_id = $1 AND status IN ('queued', 'running')",
        &[&id],
    )
    .await?;
    tx.commit().await?;
    audit(
        &client,
        &admin.email,
        "onenode_revoke",
        &id.to_string(),
        "ok",
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn create_project(
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
    let client = db(&s).await?;
    let id: Uuid = client
        .query_one(
            "INSERT INTO projects(name, created_by) VALUES ($1, $2) RETURNING id",
            &[&name, &admin.id],
        )
        .await?
        .get(0);
    audit(
        &client,
        &admin.email,
        "project_create",
        &id.to_string(),
        "ok",
    )
    .await?;
    Ok((StatusCode::CREATED, Json(json!({ "id": id, "name": name }))))
}

async fn list_projects(
    State(s): State<AppState>,
    _user: AuthUser,
) -> Result<Json<Value>, ApiError> {
    let client = db(&s).await?;
    let rows = client
        .query(
            "SELECT id, name, created_at FROM projects ORDER BY created_at",
            &[],
        )
        .await?;
    Ok(Json(json!(rows
        .iter()
        .map(|r| json!({ "id": r.get::<_, Uuid>(0), "name": r.get::<_, String>(1), "created_at": r.get::<_, DateTime<Utc>>(2) }))
        .collect::<Vec<_>>())))
}

#[derive(Deserialize)]
pub struct GrantBody {
    pub node_id: Uuid,
    /// Raiz pedida neste OneNode. O nó só a usa se a política local a aceitar.
    pub root_path: String,
}

/// Formato mínimo da raiz pedida. A decisão final é do OneNode (política local e canonicalização).
pub(crate) fn validate_root_path(path: &str) -> Result<(), ApiError> {
    let bad = !path.starts_with('/')
        || path.len() < 2
        || path.len() > 512
        || path.contains('\0')
        || path.split('/').any(|c| c == "..")
        || path != path.trim();
    if bad {
        return Err(ApiError::BadRequest("root_path_invalid"));
    }
    Ok(())
}

async fn grant_project(
    State(s): State<AppState>,
    admin: AuthUser,
    Path(project_id): Path<Uuid>,
    Json(b): Json<GrantBody>,
) -> Result<StatusCode, ApiError> {
    require_admin(&admin)?;
    validate_root_path(&b.root_path)?;
    let mut client = db(&s).await?;
    let tx = client.transaction().await?;
    let node_ok = tx
        .query_opt(
            "SELECT 1 FROM onenodes WHERE id = $1 AND revoked_at IS NULL FOR UPDATE",
            &[&b.node_id],
        )
        .await?;
    let proj_ok = tx
        .query_opt("SELECT 1 FROM projects WHERE id = $1", &[&project_id])
        .await?;
    if node_ok.is_none() || proj_ok.is_none() {
        return Err(ApiError::NotFound);
    }
    tx.execute(
        "INSERT INTO node_project_grants(node_id, project_id, granted_by, root_path) VALUES ($1, $2, $3, $4) \
         ON CONFLICT (node_id, project_id) DO UPDATE SET revoked_at = NULL, granted_by = EXCLUDED.granted_by, \
         granted_at = now(), root_path = EXCLUDED.root_path",
        &[&b.node_id, &project_id, &admin.id, &b.root_path],
    )
    .await?;
    // o catálogo do nó mudou: ele precisa buscar e aplicar a nova versão
    tx.execute("UPDATE onenodes SET catalog_version = catalog_version + 1 WHERE id = $1", &[&b.node_id])
        .await?;
    tx.commit().await?;
    audit(
        &client,
        &admin.email,
        "grant",
        &format!("{}:{}", b.node_id, project_id),
        "ok",
    )
    .await?;
    Ok(StatusCode::CREATED)
}

async fn revoke_grant(
    State(s): State<AppState>,
    admin: AuthUser,
    Path((project_id, node_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    require_admin(&admin)?;
    let mut client = db(&s).await?;
    let tx = client.transaction().await?;
    let done = tx
        .execute(
            "UPDATE node_project_grants SET revoked_at = now() WHERE node_id = $1 AND project_id = $2 AND revoked_at IS NULL",
            &[&node_id, &project_id],
        )
        .await?;
    if done == 0 {
        return Err(ApiError::NotFound);
    }
    tx.execute("UPDATE onenodes SET catalog_version = catalog_version + 1 WHERE id = $1", &[&node_id])
        .await?;
    for r in tx
        .query(
            "UPDATE tasks SET status = 'cancelled', lease_expires = NULL, lease_token = NULL, updated_at = now() \
             WHERE node_id = $1 AND project_id = $2 AND status IN ('queued', 'running') RETURNING id",
            &[&node_id, &project_id],
        )
        .await?
    {
        // a interface acompanha o cancelamento das ferramentas na mesma transação
        crate::events::emit_task(&tx, r.get(0)).await?;
    }
    crate::executor::revoke_pair_jobs(&tx, node_id, project_id).await?;
    tx.commit().await?;
    audit(
        &client,
        &admin.email,
        "grant_revoke",
        &format!("{node_id}:{project_id}"),
        "ok",
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct NewConversation {
    pub title: Option<String>,
}

async fn create_conversation(
    State(s): State<AppState>,
    user: AuthUser,
    Json(b): Json<NewConversation>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let title = b.title.unwrap_or_else(|| "Nova conversa".into());
    if !text_len_ok(&title, 1, 200) {
        return Err(ApiError::BadRequest("invalid_title"));
    }
    let client = db(&s).await?;
    let id: Uuid = client
        .query_one(
            "INSERT INTO conversations(owner_id, title) VALUES ($1, $2) RETURNING id",
            &[&user.id, &title.trim()],
        )
        .await?
        .get(0);
    Ok((
        StatusCode::CREATED,
        Json(json!({ "id": id, "title": title.trim() })),
    ))
}

async fn list_conversations(
    State(s): State<AppState>,
    user: AuthUser,
) -> Result<Json<Value>, ApiError> {
    let client = db(&s).await?;
    let rows = client
        .query(
            "SELECT id, title, updated_at, node_id, project_id FROM conversations WHERE owner_id = $1 ORDER BY updated_at DESC LIMIT 200",
            &[&user.id],
        )
        .await?;
    Ok(Json(json!(rows
        .iter()
        .map(|r| json!({
            "id": r.get::<_, Uuid>(0),
            "title": r.get::<_, String>(1),
            "updated_at": r.get::<_, DateTime<Utc>>(2),
            "node_id": r.get::<_, Option<Uuid>>(3),
            "project_id": r.get::<_, Option<Uuid>>(4),
        }))
        .collect::<Vec<_>>())))
}

fn message_json(r: &Row) -> Value {
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

/// Grant ativo OneNode × projeto, com o nó não revogado. Sem ele, nada é criado nesse escopo.
pub(crate) async fn check_grant<C: GenericClient>(
    client: &C,
    node: Uuid,
    project: Uuid,
) -> Result<(), ApiError> {
    let ok = client
        .query_opt(
            "SELECT 1 FROM node_project_grants g JOIN onenodes n ON n.id = g.node_id \
             WHERE g.node_id = $1 AND g.project_id = $2 AND g.revoked_at IS NULL AND n.revoked_at IS NULL",
            &[&node, &project],
        )
        .await?;
    ok.map(|_| ()).ok_or(ApiError::Forbidden("no_grant"))
}

#[derive(Deserialize)]
pub struct TurnBody {
    pub text: String,
    pub node_id: Option<Uuid>,
    pub project_id: Option<Uuid>,
}

/// Um turno do chat numa única transação: mensagem do usuário, roteamento (comando determinístico ou
/// provedor de IA), tarefa ou análise e resposta. Cada mudança grava o seu evento na mesma transação.
#[derive(Deserialize)]
pub struct ContextBody {
    /// Ausentes os dois: contexto OneHub Central (administração, sem OneNode).
    pub node_id: Option<Uuid>,
    pub project_id: Option<Uuid>,
}

/// Persiste o contexto escolhido para a conversa. Autorização no backend: dono da conversa, nó não revogado
/// e concessão ativa para o projeto. Escolher um nó suspenso é permitido (ele só fica indisponível para análise).
async fn set_context(
    State(s): State<AppState>,
    user: AuthUser,
    Path(conv): Path<Uuid>,
    Json(b): Json<ContextBody>,
) -> Result<StatusCode, ApiError> {
    let mut conn = db(&s).await?;
    let tx = conn.transaction().await?;
    owned_conversation(&tx, &user, conv).await?;
    match (b.node_id, b.project_id) {
        (None, Some(_)) => return Err(ApiError::BadRequest("context_incomplete")),
        (None, None) => {}
        (Some(n), project) => {
            let node = tx
                .query_opt("SELECT revoked_at IS NOT NULL FROM onenodes WHERE id = $1", &[&n])
                .await?
                .ok_or(ApiError::NotFound)?;
            if node.get::<_, bool>(0) {
                return Err(ApiError::Conflict("node_revoked"));
            }
            if let Some(p) = project {
                check_grant(&tx, n, p).await?;
            }
        }
    }
    tx.execute(
        "UPDATE conversations SET node_id = $2, project_id = $3 WHERE id = $1",
        &[&conv, &b.node_id, &b.project_id],
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn turn(
    State(s): State<AppState>,
    user: AuthUser,
    Path(conv): Path<Uuid>,
    Json(b): Json<TurnBody>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let mut conn = db(&s).await?;
    let client = conn.transaction().await?;
    owned_conversation(&client, &user, conv).await?;
    let text = b.text.trim().to_string();
    if !text_len_ok(&text, 1, 16 * 1024) {
        return Err(ApiError::BadRequest("invalid_body"));
    }
    // contexto: OneNode e projeto só mudam com concessão ativa; ambos ou nenhum
    let (node, mut project) = match (b.node_id, b.project_id) {
        // OneNode sem projeto: o projeto é resolvido pelo nome citado, pelas concessões ou pede-se a escolha
        (Some(n), p) => {
            let p = match p {
                Some(p) => {
                    check_grant(&client, n, p).await?;
                    Some(p)
                }
                // sem projeto informado: mantém o projeto atual só se for do mesmo OneNode
                None => {
                    let cur = client
                        .query_one(
                            "SELECT node_id, project_id FROM conversations WHERE id = $1",
                            &[&conv],
                        )
                        .await?;
                    if cur.get::<_, Option<Uuid>>(0) == Some(n) {
                        cur.get::<_, Option<Uuid>>(1)
                    } else {
                        None
                    }
                }
            };
            client
                .execute(
                    "UPDATE conversations SET node_id = $2, project_id = $3 WHERE id = $1",
                    &[&conv, &n, &p],
                )
                .await?;
            (Some(n), p)
        }
        (None, None) => {
            let r = client
                .query_one(
                    "SELECT node_id, project_id FROM conversations WHERE id = $1",
                    &[&conv],
                )
                .await?;
            (r.get::<_, Option<Uuid>>(0), r.get::<_, Option<Uuid>>(1))
        }
        // projeto sem OneNode não identifica nada: o contexto está incompleto
        (None, Some(_)) => return Err(ApiError::BadRequest("context_incomplete")),
    };
    let command = parse_command(&text);
    // /investigar sem projeto escolhido: usa o único projeto autorizado do OneNode (mesma regra do texto livre)
    if project.is_none() && matches!(command, Some(Command::Investigar { .. })) {
        if let Some(n) = node {
            let rows = client
                .query(
                    "SELECT g.project_id FROM node_project_grants g JOIN onenodes o ON o.id = g.node_id \
                     WHERE g.node_id = $1 AND g.revoked_at IS NULL AND o.revoked_at IS NULL",
                    &[&n],
                )
                .await?;
            if let [only] = rows.as_slice() {
                let p: Uuid = only.get(0);
                client
                    .execute(
                        "UPDATE conversations SET project_id = $2 WHERE id = $1",
                        &[&conv, &p],
                    )
                    .await?;
                project = Some(p);
            }
        }
    }
    // Texto livre: o projeto citado pelo nome (só entre os autorizados do OneNode). Escolha explícita vence.
    let mut ambiguous: Option<Vec<String>> = None;
    if command.is_none() && b.project_id.is_none() {
        if let Some(n) = node {
            match crate::mention::mentioned_projects(&client, n, &text)
                .await?
                .as_slice()
            {
                [(p, _)] => {
                    if project != Some(*p) {
                        client
                            .execute(
                                "UPDATE conversations SET project_id = $2 WHERE id = $1",
                                &[&conv, p],
                            )
                            .await?;
                        project = Some(*p);
                    }
                }
                [] => {}
                many => {
                    ambiguous = Some(many.iter().map(|(_, name)| name.clone()).collect());
                }
            }
        }
    }
    let (user_id, user_at) =
        insert_message(&client, conv, "user", &text, "completed", None, None).await?;
    // o título da conversa nasce da primeira mensagem (até 60 caracteres)
    client
        .execute("UPDATE conversations SET title = left($2, 60) WHERE id = $1 AND title = 'Nova conversa'", &[&conv, &text])
        .await?;

    let mut job_link: Option<Uuid> = None;
    let (state, body, task): (&str, String, Option<Uuid>) = match command {
        Some(Command::Investigar { path }) => match (node, project) {
            (Some(n), Some(p)) => match check_grant(&client, n, p).await {
                Err(_) => (
                    "rejected",
                    "O OneNode escolhido não tem permissão para este projeto.".into(),
                    None,
                ),
                Ok(()) => {
                    let tool = tool_for_path(&path);
                    let args = json!({ "path": path });
                    let task_id: Uuid = client
                        .query_one(
                            "INSERT INTO tasks(conversation_id, node_id, project_id, created_by, prompt, idempotency_key, tool, args) \
                             VALUES ($1, $2, $3, $4, $5, $6, $7, $8) RETURNING id",
                            &[&conv, &n, &p, &user.id, &text, &format!("turno-{user_id}"), &tool, &args],
                        )
                        .await?
                        .get(0);
                    crate::events::emit_task(&client, task_id).await?;
                    (
                        "task",
                        format!("Tarefa criada: {tool} em \"{path}\". O resultado aparece abaixo."),
                        Some(task_id),
                    )
                }
            },
            _ => (
                "rejected",
                "Escolha um OneNode e um projeto antes de usar /investigar.".into(),
                None,
            ),
        },
        Some(Command::NotEnabled(m)) => ("rejected", format!("Comando indisponível: {m}."), None),
        Some(Command::Unknown) => (
            "rejected",
            "Comando desconhecido. Use /investigar, /corrigir, /melhorar ou /testar.".into(),
            None,
        ),
        None => match ambiguous {
            Some(names) => (
                "rejected",
                format!(
                    "A mensagem cita mais de um projeto deste OneNode ({}). Escolha o projeto antes de pedir a análise.",
                    names.join(", ")
                ),
                None,
            ),
            None => {
                let history: Vec<(String, String)> = client
                    .query(
                        "SELECT role, body FROM messages WHERE conversation_id = $1 AND state = 'completed' ORDER BY created_at DESC, id DESC LIMIT 20",
                        &[&conv],
                    )
                    .await?
                    .iter()
                    .rev()
                    .map(|r| (r.get::<_, String>(0), r.get::<_, String>(1)))
                    .collect();
                match s.provider.reply(&history, &text) {
                    InferenceOutcome::Reply(t) => ("completed", t, None),
                    // Sem provedor de IA no Central: o pedido vai ao OneExecutor, que analisa o projeto escolhido.
                    InferenceOutcome::Unavailable(_) => {
                        let (st, body, job) = crate::executor::analysis_turn(
                            &client, &user, conv, &text, node, project, user_id,
                        )
                        .await?;
                        job_link = job;
                        (st, body, None)
                    }
                }
            }
        },
    };
    let (_asst_id, asst_at) =
        insert_message(&client, conv, "assistant", &body, state, task, job_link).await?;
    client
        .execute(
            "UPDATE conversations SET updated_at = now() WHERE id = $1",
            &[&conv],
        )
        .await?;
    audit(&client, &user.email, "turn", &conv.to_string(), state).await?;
    client.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "user_message": { "id": user_id, "role": "user", "body": text, "created_at": user_at, "state": "completed", "task_id": null, "job_id": null },
            "assistant_message": { "id": _asst_id, "role": "assistant", "body": body, "created_at": asst_at, "state": state, "task_id": task, "job_id": job_link },
        })),
    ))
}

/// Grava uma mensagem e emite `message.created` na mesma transação.
pub(crate) async fn insert_message<C: GenericClient>(
    client: &C,
    conv: Uuid,
    role: &str,
    body: &str,
    state: &str,
    task: Option<Uuid>,
    job: Option<Uuid>,
) -> Result<(Uuid, DateTime<Utc>), ApiError> {
    let r = client
        .query_one(
            "INSERT INTO messages(conversation_id, role, body, state, task_id, job_id) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id, created_at",
            &[&conv, &role, &body, &state, &task, &job],
        )
        .await?;
    let (id, at): (Uuid, DateTime<Utc>) = (r.get(0), r.get(1));
    crate::events::emit(
        client,
        conv,
        "message.created",
        json!({ "message": { "id": id, "role": role, "body": body, "created_at": at, "state": state, "task_id": task, "job_id": job } }),
    )
    .await?;
    Ok((id, at))
}

/// Confere se a conversa existe E pertence a quem pergunta. Para terceiros: "não encontrada".
pub(crate) async fn owned_conversation<C: GenericClient>(
    client: &C,
    user: &AuthUser,
    id: Uuid,
) -> Result<(), ApiError> {
    client
        .query_opt(
            "SELECT 1 FROM conversations WHERE id = $1 AND owner_id = $2",
            &[&id, &user.id],
        )
        .await?
        .map(|_| ())
        .ok_or(ApiError::NotFound)
}

#[derive(Deserialize)]
pub struct NewMessage {
    pub body: String,
}

async fn add_message(
    State(s): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
    Json(b): Json<NewMessage>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    if !text_len_ok(&b.body, 1, 16 * 1024) {
        return Err(ApiError::BadRequest("invalid_body"));
    }
    let mut conn = db(&s).await?;
    // mensagem, evento e atualização da conversa confirmam juntos (ou não confirmam)
    let client = conn.transaction().await?;
    owned_conversation(&client, &user, id).await?;
    let (mid, _) = insert_message(&client, id, "user", &b.body, "completed", None, None).await?;
    client
        .execute(
            "UPDATE conversations SET updated_at = now() WHERE id = $1",
            &[&id],
        )
        .await?;
    client.commit().await?;
    Ok((StatusCode::CREATED, Json(json!({ "id": mid }))))
}

async fn list_messages(
    State(s): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    let client = db(&s).await?;
    owned_conversation(&client, &user, id).await?;
    let rows = client
        .query(
            "SELECT id, role, body, created_at, state, task_id, job_id FROM messages WHERE conversation_id = $1 ORDER BY created_at, id",
            &[&id],
        )
        .await?;
    Ok(Json(json!(rows
        .iter()
        .map(message_json)
        .collect::<Vec<_>>())))
}

#[derive(Deserialize)]
pub struct NewTask {
    pub node_id: Uuid,
    pub project_id: Uuid,
    pub prompt: String,
    pub tool: String,
    #[serde(default = "empty_args")]
    pub args: Value,
    pub idempotency_key: String,
    pub conversation_id: Option<Uuid>,
}

fn empty_args() -> Value {
    json!({})
}

/// Catálogo fechado de ferramentas do OneNode. Nada além disso é aceito, em nenhuma camada.
pub const TOOLS: &[&str] = &["files_list", "files_read"];

/// Valida o formato dos argumentos de cada ferramenta. Caminhos são sempre relativos ao workspace;
/// a confinação real (symlinks, `..`) é aplicada pelo OneNode, que tem o sistema de arquivos.
pub fn validate_tool_args(tool: &str, args: &Value) -> Result<(), &'static str> {
    if !TOOLS.contains(&tool) {
        return Err("tool_not_allowed");
    }
    let obj = args.as_object().ok_or("invalid_args")?;
    if obj.keys().any(|k| k != "path") {
        return Err("invalid_args");
    }
    let path = obj
        .get("path")
        .and_then(Value::as_str)
        .ok_or("invalid_args")?;
    if path.len() > 512 || path.contains('\0') {
        return Err("invalid_args");
    }
    match tool {
        "files_read" if path.trim().is_empty() => Err("invalid_args"),
        _ => Ok(()),
    }
}

async fn create_task(
    State(s): State<AppState>,
    user: AuthUser,
    Json(b): Json<NewTask>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    if !text_len_ok(&b.prompt, 1, MAX_PROMPT_CHARS) || !text_len_ok(&b.idempotency_key, 8, 128) {
        return Err(ApiError::BadRequest("invalid_task"));
    }
    validate_tool_args(&b.tool, &b.args).map_err(ApiError::BadRequest)?;
    let mut client = db(&s).await?;
    if let Some(c) = b.conversation_id {
        owned_conversation(&client, &user, c).await?;
    }
    let node = client
        .query_opt(
            "SELECT revoked_at, suspended_at FROM onenodes WHERE id = $1",
            &[&b.node_id],
        )
        .await?
        .ok_or(ApiError::NotFound)?;
    if node.get::<_, Option<DateTime<Utc>>>(0).is_some() {
        return Err(ApiError::Conflict("node_revoked"));
    }
    // suspenso: nenhuma tarefa nova é aceita (a suspensão é reversível, então é 423, não revogação)
    if node.get::<_, Option<DateTime<Utc>>>(1).is_some() {
        return Err(ApiError::Locked("node_suspended"));
    }
    // Sem concessão ativa OneNode × projeto, nenhuma tarefa é criada.
    let grant = client
        .query_opt(
            "SELECT 1 FROM node_project_grants WHERE node_id = $1 AND project_id = $2 AND revoked_at IS NULL",
            &[&b.node_id, &b.project_id],
        )
        .await?;
    if grant.is_none() {
        return Err(ApiError::Forbidden("no_grant"));
    }
    // Idempotência: mesmo envio repetido devolve a mesma tarefa; chave com outro conteúdo é conflito.
    if let Some(r) = client
        .query_opt(
            "SELECT id, node_id, project_id, prompt, tool, args FROM tasks WHERE created_by = $1 AND idempotency_key = $2",
            &[&user.id, &b.idempotency_key],
        )
        .await?
    {
        let same = r.get::<_, Uuid>(1) == b.node_id
            && r.get::<_, Uuid>(2) == b.project_id
            && r.get::<_, String>(3) == b.prompt
            && r.get::<_, Option<String>>(4).as_deref() == Some(b.tool.as_str())
            && r.get::<_, Value>(5) == b.args;
        if !same {
            return Err(ApiError::Conflict("idempotency_key_reused"));
        }
        return Ok((StatusCode::OK, Json(json!({ "id": r.get::<_, Uuid>(0), "status": "existing" }))));
    }
    let tx = client.transaction().await?;
    let id: Uuid = tx
        .query_one(
            "INSERT INTO tasks(conversation_id, node_id, project_id, created_by, prompt, idempotency_key, tool, args) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8) RETURNING id",
            &[&b.conversation_id, &b.node_id, &b.project_id, &user.id, &b.prompt, &b.idempotency_key, &b.tool, &b.args],
        )
        .await?
        .get(0);
    crate::events::emit_task(&tx, id).await?;
    tx.commit().await?;
    audit(
        &client,
        &user.email,
        "task_create",
        &id.to_string(),
        "queued",
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "id": id, "status": "queued" })),
    ))
}

async fn list_tasks(State(s): State<AppState>, user: AuthUser) -> Result<Json<Value>, ApiError> {
    let client = db(&s).await?;
    let rows = client
        .query(
            "SELECT t.id, t.node_id, n.name, t.project_id, p.name, t.tool, t.status, t.created_at, t.updated_at \
             FROM tasks t JOIN onenodes n ON n.id = t.node_id JOIN projects p ON p.id = t.project_id \
             WHERE t.created_by = $1 ORDER BY t.created_at DESC LIMIT 100",
            &[&user.id],
        )
        .await?;
    Ok(Json(json!(rows
        .iter()
        .map(|r| json!({
            "id": r.get::<_, Uuid>(0),
            "node_id": r.get::<_, Uuid>(1),
            "node_name": r.get::<_, String>(2),
            "project_id": r.get::<_, Uuid>(3),
            "project_name": r.get::<_, String>(4),
            "tool": r.get::<_, Option<String>>(5),
            "status": r.get::<_, String>(6),
            "created_at": r.get::<_, DateTime<Utc>>(7),
            "updated_at": r.get::<_, DateTime<Utc>>(8),
        }))
        .collect::<Vec<_>>())))
}

/// Concessões ativas OneNode × projeto (metadados), para a interface escolher o escopo permitido.
async fn list_grants(State(s): State<AppState>, _user: AuthUser) -> Result<Json<Value>, ApiError> {
    let client = db(&s).await?;
    let rows = client
        .query(
            "SELECT g.node_id, n.name, g.project_id, p.name, g.root_path FROM node_project_grants g \
             JOIN onenodes n ON n.id = g.node_id JOIN projects p ON p.id = g.project_id \
             WHERE g.revoked_at IS NULL AND n.revoked_at IS NULL ORDER BY n.name, p.name",
            &[],
        )
        .await?;
    Ok(Json(json!(rows
        .iter()
        .map(|r| json!({
            "node_id": r.get::<_, Uuid>(0),
            "node_name": r.get::<_, String>(1),
            "project_id": r.get::<_, Uuid>(2),
            "project_name": r.get::<_, String>(3),
            "root_path": r.get::<_, Option<String>>(4),
        }))
        .collect::<Vec<_>>())))
}

/// Detalhe administrativo de um OneNode: estado real, concessões com raiz e versões do catálogo.
async fn get_onenode(
    State(s): State<AppState>,
    admin: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    require_admin(&admin)?;
    let client = db(&s).await?;
    let row = client
        .query_opt(
            "SELECT name, paired_at, last_seen_at, revoked_at, suspended_at, access_type, catalog_version, \
             applied_version, applied_at, applied_report FROM onenodes WHERE id = $1",
            &[&id],
        )
        .await?
        .ok_or(ApiError::NotFound)?;
    let paired: Option<DateTime<Utc>> = row.get(1);
    let last: Option<DateTime<Utc>> = row.get(2);
    let revoked: Option<DateTime<Utc>> = row.get(3);
    let suspended: Option<DateTime<Utc>> = row.get(4);
    let catalog_version: i64 = row.get(6);
    let applied_version: Option<i64> = row.get(7);
    let grants = client
        .query(
            "SELECT p.id, p.name, g.root_path FROM node_project_grants g JOIN projects p ON p.id = g.project_id \
             WHERE g.node_id = $1 AND g.revoked_at IS NULL ORDER BY p.name",
            &[&id],
        )
        .await?;
    let grants: Vec<Value> = grants
        .iter()
        .map(|g| {
            json!({
                "project_id": g.get::<_, Uuid>(0),
                "project_name": g.get::<_, String>(1),
                "root_path": g.get::<_, Option<String>>(2),
            })
        })
        .collect();
    Ok(Json(json!({
        "id": id,
        "name": row.get::<_, String>(0),
        "status": node_status(revoked.is_some(), paired.is_some(), last),
        "suspended": suspended.is_some(),
        "paired_at": paired,
        "last_seen_at": last,
        "access_type": row.get::<_, String>(5),
        "catalog_version": catalog_version,
        "applied_version": applied_version,
        "applied_at": row.get::<_, Option<DateTime<Utc>>>(8),
        "applied_report": row.get::<_, Option<Value>>(9),
        "catalog_in_sync": applied_version == Some(catalog_version),
        "grants": grants,
    })))
}

async fn get_task(
    State(s): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    let client = db(&s).await?;
    let row = client
        .query_opt(
            "SELECT id, node_id, project_id, created_by, status, result, created_at, updated_at, tool, args FROM tasks WHERE id = $1",
            &[&id],
        )
        .await?
        .ok_or(ApiError::NotFound)?;
    if row.get::<_, Uuid>(3) != user.id && !user.is_admin {
        return Err(ApiError::NotFound);
    }
    let events = client
        .query("SELECT seq, kind, payload, created_at FROM task_events WHERE task_id = $1 ORDER BY seq", &[&id])
        .await?;
    Ok(Json(json!({
        "id": row.get::<_, Uuid>(0),
        "node_id": row.get::<_, Uuid>(1),
        "project_id": row.get::<_, Uuid>(2),
        "status": row.get::<_, String>(4),
        "result": row.get::<_, Option<Value>>(5),
        "created_at": row.get::<_, DateTime<Utc>>(6),
        "updated_at": row.get::<_, DateTime<Utc>>(7),
        "tool": row.get::<_, Option<String>>(8),
        "args": row.get::<_, Value>(9),
        "events": events.iter().map(|e| json!({
            "seq": e.get::<_, i32>(0),
            "kind": e.get::<_, String>(1),
            "payload": e.get::<_, Value>(2),
            "created_at": e.get::<_, DateTime<Utc>>(3),
        })).collect::<Vec<_>>(),
    })))
}

/// Entrega a próxima tarefa queued do OneNode, com lease. Lease vencido volta à fila.
async fn next_task(State(s): State<AppState>, node: NodeAuth) -> Result<Response, ApiError> {
    let mut client = db(&s).await?;
    let tx = client.transaction().await?;
    // Tarefas sem concessão ativa são canceladas antes de qualquer entrega.
    for r in tx
        .query(
            "UPDATE tasks t SET status = 'cancelled', updated_at = now() WHERE t.node_id = $1 AND t.status = 'queued' \
             AND NOT EXISTS (SELECT 1 FROM node_project_grants g WHERE g.node_id = t.node_id AND g.project_id = t.project_id AND g.revoked_at IS NULL) \
             RETURNING id",
            &[&node.id],
        )
        .await?
    {
        crate::events::emit_task(&tx, r.get(0)).await?;
    }
    // Lease vencido: a tarefa volta à fila e o nó anterior perde o direito de finalizá-la.
    for r in tx
        .query(
            "UPDATE tasks SET status = 'queued', lease_expires = NULL, lease_token = NULL, updated_at = now() \
             WHERE node_id = $1 AND status = 'running' AND lease_expires < now() RETURNING id",
            &[&node.id],
        )
        .await?
    {
        crate::events::emit_task(&tx, r.get(0)).await?;
    }
    let token = Uuid::new_v4();
    // SKIP LOCKED: consultas simultâneas do mesmo nó nunca recebem a mesma tarefa.
    let row = tx
        .query_opt(
            "UPDATE tasks SET status = 'running', lease_token = $2, lease_expires = now() + make_interval(secs => $3), updated_at = now() \
             WHERE id = (SELECT id FROM tasks WHERE node_id = $1 AND status = 'queued' ORDER BY created_at FOR UPDATE SKIP LOCKED LIMIT 1) \
             RETURNING id, project_id, prompt, mode, lease_token, lease_expires, tool, args",
            &[&node.id, &token, &LEASE_SECONDS],
        )
        .await?;
    if let Some(r) = &row {
        crate::events::emit_task(&tx, r.get(0)).await?;
    }
    tx.commit().await?;
    match row {
        None => Ok(StatusCode::NO_CONTENT.into_response()),
        Some(r) => Ok(Json(json!({
            "id": r.get::<_, Uuid>(0),
            "project_id": r.get::<_, Uuid>(1),
            "prompt": r.get::<_, String>(2),
            "mode": r.get::<_, String>(3),
            "lease_token": r.get::<_, Uuid>(4),
            "lease_expires_at": r.get::<_, DateTime<Utc>>(5),
            "tool": r.get::<_, Option<String>>(6),
            "args": r.get::<_, Value>(7),
        }))
        .into_response()),
    }
}

/// Confere que o nó detém o lease atual e que ele ainda não venceu. Sem isso, a escrita é recusada.
async fn check_lease<C: GenericClient>(
    client: &C,
    node: &NodeAuth,
    task: Uuid,
    lease: Uuid,
) -> Result<(), ApiError> {
    client
        .query_opt(
            "SELECT 1 FROM tasks WHERE id = $1 AND node_id = $2 AND status = 'running' AND lease_token = $3 AND lease_expires > now() FOR UPDATE",
            &[&task, &node.id, &lease],
        )
        .await?
        .map(|_| ())
        .ok_or(ApiError::Conflict("stale_lease"))
}

fn parse_uuid(v: Option<&Value>) -> Option<Uuid> {
    v.and_then(Value::as_str)
        .and_then(|s| Uuid::parse_str(s).ok())
}

async fn task_event(
    State(s): State<AppState>,
    node: NodeAuth,
    Path(id): Path<Uuid>,
    Json(b): Json<Value>,
) -> Result<StatusCode, ApiError> {
    let lease = parse_uuid(b.get("lease_token")).ok_or(ApiError::BadRequest("invalid_lease"))?;
    let seq = b
        .get("seq")
        .and_then(Value::as_i64)
        .and_then(|v| i32::try_from(v).ok())
        .filter(|v| *v >= 0)
        .ok_or(ApiError::BadRequest("invalid_seq"))?;
    let kind = b
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let payload = b.get("payload").cloned().unwrap_or_else(|| json!({}));
    if !text_len_ok(&kind, 1, 64) || payload.to_string().len() > MAX_EVENT_BYTES {
        return Err(ApiError::BadRequest("invalid_event"));
    }
    let client = db(&s).await?;
    check_lease(&client, &node, id, lease).await?;
    let inserted = client
        .execute(
            "INSERT INTO task_events(task_id, seq, kind, payload) VALUES ($1, $2, $3, $4) ON CONFLICT (task_id, seq) DO NOTHING",
            &[&id, &seq, &kind, &payload],
        )
        .await?;
    if inserted == 0 {
        return Err(ApiError::Conflict("duplicate_seq"));
    }
    Ok(StatusCode::CREATED)
}

async fn finish_task(
    State(s): State<AppState>,
    node: NodeAuth,
    Path(id): Path<Uuid>,
    Json(b): Json<Value>,
) -> Result<StatusCode, ApiError> {
    let lease = parse_uuid(b.get("lease_token")).ok_or(ApiError::BadRequest("invalid_lease"))?;
    let status = b.get("status").and_then(Value::as_str).unwrap_or("");
    if status != "succeeded" && status != "failed" {
        return Err(ApiError::BadRequest("invalid_status"));
    }
    let result = b.get("result").cloned().unwrap_or_else(|| json!({}));
    if result.to_string().len() > MAX_RESULT_BYTES {
        return Err(ApiError::BadRequest("result_too_large"));
    }
    let mut client = db(&s).await?;
    let tx = client.transaction().await?;
    check_lease(&tx, &node, id, lease).await?;
    // Finalização atômica: só o lease vigente conclui, e só uma vez.
    let done = tx
        .execute(
            "UPDATE tasks SET status = $3, result = $4, lease_expires = NULL, lease_token = NULL, updated_at = now() \
             WHERE id = $1 AND node_id = $2 AND status = 'running' AND lease_token = $5 AND lease_expires > now()",
            &[&id, &node.id, &status, &result, &lease],
        )
        .await?;
    if done == 0 {
        return Err(ApiError::Conflict("stale_lease"));
    }
    crate::events::emit_task(&tx, id).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod origin_tests {
    use super::client_origin;
    use axum::http::HeaderMap;
    use std::net::{IpAddr, SocketAddr};

    fn proxy() -> Vec<IpAddr> {
        vec!["172.18.0.1".parse().unwrap()]
    }

    fn xff(v: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("x-forwarded-for", v.parse().unwrap());
        h
    }

    #[test]
    fn forwarded_header_is_only_used_from_a_trusted_proxy() {
        let proxy_peer: SocketAddr = "172.18.0.1:5000".parse().unwrap();
        let remote: SocketAddr = "203.0.113.9:5000".parse().unwrap();
        // sem lista de proxies: o cabeçalho forjado não muda nada
        assert_eq!(
            client_origin(Some(proxy_peer), &xff("198.51.100.7"), &[]),
            "172.18.0.1"
        );
        // conexão de fora da lista: cabeçalho forjado ignorado
        assert_eq!(
            client_origin(Some(remote), &xff("198.51.100.7"), &proxy()),
            "203.0.113.9"
        );
        // proxy confiável: vale o último endereço (acrescentado pelo proxy)
        assert_eq!(
            client_origin(Some(proxy_peer), &xff("10.0.0.1, 198.51.100.7"), &proxy()),
            "198.51.100.7"
        );
        // loopback não é confiável por si só
        let local: SocketAddr = "127.0.0.1:5000".parse().unwrap();
        assert_eq!(
            client_origin(Some(local), &xff("198.51.100.7"), &[]),
            "127.0.0.1"
        );
        // sem socket (testes) e sem cabeçalho válido: origem fixa
        assert_eq!(
            client_origin(None, &HeaderMap::new(), &proxy()),
            "desconhecida"
        );
    }
}
