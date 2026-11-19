//! API local do provisionador. Escuta só em loopback e exige token. Só o OneHub Central a chama.
//! Não tem banco, não guarda credenciais e não devolve segredos. Execuções ficam em memória.

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::guard::AccessType;
use crate::install::{self, redact, InstallSpec, Remote, Step, StepStatus};
use crate::scan::scan_fingerprint;
use crate::ssh::{self, Auth, Secret};

#[derive(Clone)]
pub struct AppConfig {
    pub token: String,
    pub binary: std::path::PathBuf,
    pub binary_sha256: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RunStatus {
    Running,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunView {
    pub run_id: Uuid,
    pub status: RunStatus,
    pub steps: Vec<Step>,
    pub error: Option<String>,
}

#[derive(Default)]
pub struct Registry {
    runs: Mutex<HashMap<Uuid, RunView>>,
    /// Um provisionamento por servidor de cada vez.
    hosts: Mutex<HashSet<String>>,
}

pub type Shared = Arc<(AppConfig, Registry)>;

#[derive(Deserialize)]
pub struct AuthWire {
    pub kind: String,
    pub secret: String,
    pub passphrase: Option<String>,
}

impl AuthWire {
    fn into_auth(self) -> Result<Auth, &'static str> {
        match self.kind.as_str() {
            "password" => Ok(Auth::Password(Secret::new(self.secret))),
            "key" => Ok(Auth::Key {
                pem: Secret::new(self.secret),
                passphrase: self.passphrase.map(Secret::new),
            }),
            _ => Err("tipo de autenticação inválido"),
        }
    }
}

#[derive(Deserialize)]
pub struct ScanBody {
    pub host: String,
    pub port: u16,
    pub access: String,
}

#[derive(Deserialize)]
pub struct RunBody {
    pub run_id: Uuid,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub access: String,
    pub expected_fingerprint: String,
    pub auth: AuthWire,
    pub node_id: Uuid,
    /// Presente quando é preciso parear; ausente quando o servidor já tem a identidade deste OneNode.
    pub pairing_code: Option<String>,
    pub central_url: String,
}

fn access_of(s: &str) -> Result<AccessType, (StatusCode, &'static str)> {
    match s {
        "direct" => Ok(AccessType::Direct),
        "tailscale" => Ok(AccessType::Tailscale),
        _ => Err((StatusCode::BAD_REQUEST, "access_type_invalid")),
    }
}

fn authorized(headers: &HeaderMap, token: &str) -> bool {
    let Some(value) = headers.get("authorization").and_then(|v| v.to_str().ok()) else {
        return false;
    };
    let Some(given) = value.strip_prefix("Bearer ") else {
        return false;
    };
    // comparação em tempo constante
    let a = given.as_bytes();
    let b = token.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn guard_auth(headers: &HeaderMap, state: &Shared) -> Result<(), (StatusCode, &'static str)> {
    if authorized(headers, &state.0.token) {
        Ok(())
    } else {
        Err((StatusCode::UNAUTHORIZED, "unauthorized"))
    }
}

pub fn router(state: Shared) -> Router {
    Router::new()
        .route("/health", get(|| async { Json(serde_json::json!({ "status": "ok" })) }))
        .route("/v1/scan", post(scan))
        .route("/v1/runs", post(start_run))
        .route("/v1/runs/{id}", get(get_run))
        .with_state(state)
}

async fn scan(
    State(state): State<Shared>,
    headers: HeaderMap,
    Json(b): Json<ScanBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    guard_auth(&headers, &state).map_err(err)?;
    let access = access_of(&b.access).map_err(err)?;
    match scan_fingerprint(&b.host, b.port, access).await {
        Ok(fp) => Ok(Json(serde_json::json!({ "fingerprint": fp }))),
        Err(e) => Err(err((StatusCode::UNPROCESSABLE_ENTITY, ssh_code_for_scan(&e)))),
    }
}

fn ssh_code_for_scan(e: &crate::scan::ScanError) -> &'static str {
    use crate::scan::ScanError as S;
    match e {
        S::Guard(_) => "destination_refused",
        S::Unreachable => "unreachable",
        S::Timeout => "timeout",
        S::NoHostKey => "no_host_key",
    }
}

fn err((status, code): (StatusCode, &'static str)) -> (StatusCode, Json<serde_json::Value>) {
    (status, Json(serde_json::json!({ "error": code })))
}

async fn start_run(
    State(state): State<Shared>,
    headers: HeaderMap,
    Json(b): Json<RunBody>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, Json<serde_json::Value>)> {
    guard_auth(&headers, &state).map_err(err)?;
    let access = access_of(&b.access).map_err(err)?;
    if !valid_user(&b.user) {
        return Err(err((StatusCode::BAD_REQUEST, "user_invalid")));
    }
    if !b.central_url.starts_with("https://") {
        return Err(err((StatusCode::BAD_REQUEST, "central_url_invalid")));
    }
    let key = format!("{}:{}", b.host.to_ascii_lowercase(), b.port);
    {
        let mut hosts = state.1.hosts.lock().expect("lock de servidores");
        if !hosts.insert(key.clone()) {
            return Err(err((StatusCode::CONFLICT, "run_in_progress")));
        }
    }
    {
        let mut runs = state.1.runs.lock().expect("lock de execuções");
        if runs.contains_key(&b.run_id) {
            state.1.hosts.lock().expect("lock de servidores").remove(&key);
            return Err(err((StatusCode::CONFLICT, "run_exists")));
        }
        runs.insert(b.run_id, RunView { run_id: b.run_id, status: RunStatus::Running, steps: vec![], error: None });
    }
    let run_id = b.run_id;
    let shared = state.clone();
    tokio::spawn(async move {
        execute(shared.clone(), b, access).await;
        shared.1.hosts.lock().expect("lock de servidores").remove(&key);
    });
    Ok((StatusCode::ACCEPTED, Json(serde_json::json!({ "run_id": run_id }))))
}

fn valid_user(u: &str) -> bool {
    !u.is_empty() && u.len() <= 32 && u.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
}

async fn get_run(
    State(state): State<Shared>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<RunView>, (StatusCode, Json<serde_json::Value>)> {
    guard_auth(&headers, &state).map_err(err)?;
    let runs = state.1.runs.lock().expect("lock de execuções");
    runs.get(&id).cloned().map(Json).ok_or_else(|| err((StatusCode::NOT_FOUND, "not_found")))
}

fn push_step(state: &Shared, run_id: Uuid, step: Step) {
    if let Some(run) = state.1.runs.lock().expect("lock de execuções").get_mut(&run_id) {
        run.steps.push(step);
    }
}

fn finish(state: &Shared, run_id: Uuid, result: Result<(), String>, secrets: &[&str]) {
    if let Some(run) = state.1.runs.lock().expect("lock de execuções").get_mut(&run_id) {
        match result {
            Ok(()) => run.status = RunStatus::Succeeded,
            Err(e) => {
                run.status = RunStatus::Failed;
                run.error = Some(redact(&e, secrets));
            }
        }
    }
}

/// Execução completa. Credenciais saem de escopo ao final (Secret zera a memória ao ser descartado).
async fn execute(state: Shared, b: RunBody, access: AccessType) {
    let run_id = b.run_id;
    let auth = match b.auth.into_auth() {
        Ok(a) => a,
        Err(e) => return finish(&state, run_id, Err(e.into()), &[]),
    };
    let code = b.pairing_code.clone();
    let mut secrets: Vec<&str> = Vec::new();
    if let Some(c) = code.as_deref() {
        secrets.push(c);
    }
    // a senha/chave entra na lista de redação; a cópia é um Secret, apagado ao sair do escopo
    let secret_text = match &auth {
        Auth::Password(s) => Secret::new(s.expose().to_string()),
        Auth::Key { pem, .. } => Secret::new(pem.expose().to_string()),
    };
    secrets.push(secret_text.expose());

    let mut session = match ssh::connect_verified(&b.host, b.port, access, &b.expected_fingerprint).await {
        Ok(s) => s,
        Err(e) => {
            push_step(&state, run_id, Step { name: "conexao", status: StepStatus::Failed, detail: e.to_string() });
            return finish(&state, run_id, Err(e.to_string()), &secrets);
        }
    };
    push_step(
        &state,
        run_id,
        Step { name: "conexao", status: StepStatus::Ok, detail: "identidade do servidor confirmada".into() },
    );
    if let Err(e) = session.authenticate(&b.user, &auth).await {
        push_step(&state, run_id, Step { name: "autenticacao", status: StepStatus::Failed, detail: e.to_string() });
        return finish(&state, run_id, Err(e.to_string()), &secrets);
    }
    push_step(&state, run_id, Step { name: "autenticacao", status: StepStatus::Ok, detail: "autenticado".into() });

    let spec = InstallSpec {
        central_url: &b.central_url,
        node_id: b.node_id,
        pairing_code: code.as_deref(),
        binary: &state.0.binary,
        binary_sha256: &state.0.binary_sha256,
    };
    let result = {
        let mut remote = Remote { session: &mut session, user: &b.user, auth: &auth };
        install::install(&mut remote, &spec, |step| push_step(&state, run_id, step.clone())).await
    };
    finish(&state, run_id, result, &secrets);
}

/// Endereço de escuta: somente loopback. Qualquer outro endereço é recusado na inicialização.
pub fn loopback_only(addr: SocketAddr) -> Result<SocketAddr, String> {
    if addr.ip().is_loopback() {
        Ok(addr)
    } else {
        Err("o provisionador só escuta em loopback".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_comparison_requires_exact_bearer_match() {
        let mut h = HeaderMap::new();
        h.insert("authorization", "Bearer segredo-123".parse().unwrap());
        assert!(authorized(&h, "segredo-123"));
        assert!(!authorized(&h, "segredo-124"));
        assert!(!authorized(&HeaderMap::new(), "segredo-123"));
        let mut basic = HeaderMap::new();
        basic.insert("authorization", "Basic segredo-123".parse().unwrap());
        assert!(!authorized(&basic, "segredo-123"));
    }

    #[test]
    fn listener_refuses_non_loopback_addresses() {
        assert!(loopback_only("127.0.0.1:8790".parse().unwrap()).is_ok());
        assert!(loopback_only("0.0.0.0:8790".parse().unwrap()).is_err());
        assert!(loopback_only("187.77.52.164:8790".parse().unwrap()).is_err());
    }

    #[test]
    fn user_names_are_restricted_to_safe_characters() {
        assert!(valid_user("root"));
        assert!(valid_user("admin.user-1"));
        assert!(!valid_user("root; rm -rf /"));
        assert!(!valid_user(""));
    }
}
