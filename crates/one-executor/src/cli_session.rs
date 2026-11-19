//! Sessões de conta de CLI (login, status e logout), executadas pelo OneExecutor em um perfil isolado por conta.
//!
//! - Cada conta tem o próprio HOME: as credenciais de uma não sobrescrevem as de outra.
//! - O binário precisa estar aprovado (`acquire`); sem aprovação, a sessão termina como falha.
//! - O estado da conta vem da verificação oficial de status da CLI, nunca de presunção.
//! - Nenhum token, cookie ou arquivo de credencial é lido ou enviado ao Central.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

use one_adapters::{acquire, detect_engine, Host, Limits, Outcome as RunOutcome, SystemHost};
use one_protocol::{AuthState, EngineKind};
use serde_json::json;

use crate::central::{CliSession, Central};
use crate::cli_login::{self, Engine, Hint, Outcome};
use one_exec::StoreTrust;

/// Prazo de um login (o Central também expira a sessão no mesmo prazo).
const LOGIN_TIMEOUT: Duration = Duration::from_secs(9 * 60);
const CONTROL_POLL: Duration = Duration::from_secs(2);
const MAX_ACTIVE: usize = 4;
const LOGOUT_LIMITS: Limits = Limits { timeout: Duration::from_secs(60), max_output: 4096 };

static ACTIVE: AtomicUsize = AtomicUsize::new(0);

/// Há espaço para mais uma sessão? Só então o executor pede uma ao Central (cada pedido já a marca como entregue).
pub fn has_capacity() -> bool {
    ACTIVE.load(Ordering::SeqCst) < MAX_ACTIVE
}

/// Identificador de conta usado como nome de pasta: só UUID, sem separadores nem `..`.
pub fn safe_id(id: &str) -> bool {
    id.len() == 36 && id.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

fn kind(engine: Engine) -> EngineKind {
    match engine {
        Engine::Codex => EngineKind::Codex,
        Engine::Claude => EngineKind::Claude,
        Engine::Cursor => EngineKind::Cursor,
    }
}

fn auth_word(a: AuthState) -> &'static str {
    match a {
        AuthState::LoggedIn => "authenticated",
        AuthState::LoggedOut => "unauthenticated",
        AuthState::Unknown => "unknown",
    }
}

/// Estado oficial da conta, lido com o HOME do perfil.
fn auth_of(kind: EngineKind, host: &SystemHost, trust: &StoreTrust) -> &'static str {
    auth_word(detect_engine(kind, host, trust, false).auth)
}

fn finish(central: &Central, id: &str, state: &str, auth: &str, result: Option<&str>) -> Result<(), String> {
    central
        .cli_event(id, json!({ "kind": "finished", "state": state, "auth": auth, "result": result }))
        .map_err(|_| "não foi possível informar o resultado ao OneHub".to_string())
}

/// Inicia a sessão em uma thread própria, para o heartbeat continuar durante um login de minutos.
pub fn spawn(central: &Central, session: CliSession, trust: &StoreTrust) {
    let Some(root) = central.profiles().map(Path::to_path_buf) else { return };
    let Ok(worker) = Central::new(central.base(), central.token()) else { return };
    let trust = StoreTrust(trust.0.clone());
    ACTIVE.fetch_add(1, Ordering::SeqCst);
    std::thread::spawn(move || {
        let outcome = handle(&worker, &session, &trust, &root);
        if let Err(msg) = outcome {
            let _ = finish(&worker, &session.id, "failed", "unknown", Some(&msg));
        }
        ACTIVE.fetch_sub(1, Ordering::SeqCst);
    });
}

fn handle(central: &Central, s: &CliSession, trust: &StoreTrust, root: &Path) -> Result<(), String> {
    let engine = Engine::parse(&s.engine).ok_or("motor desconhecido")?;
    if !safe_id(&s.connection_id) {
        return Err("conta inválida".into());
    }
    let profile = profile_dir(root, &s.connection_id)?;
    let kind = kind(engine);
    let host = SystemHost::new().with_profile_home(profile.clone());
    let exe = acquire(kind, &host, trust).map_err(|_| "CLI não aprovada neste executor".to_string())?;

    match s.action.as_str() {
        "status" => {
            let auth = auth_of(kind, &host, trust);
            finish(central, &s.id, "succeeded", auth, None)
        }
        "logout" => {
            let out = host.run(&exe, engine.logout_args(), LOGOUT_LIMITS);
            if !matches!(out.outcome, RunOutcome::Exited(Some(0))) {
                return finish(central, &s.id, "failed", "unknown", Some("logout não concluído pela CLI"));
            }
            let auth = auth_of(kind, &host, trust);
            finish(central, &s.id, "succeeded", auth, None)
        }
        "login" => login(central, s, engine, kind, &host, trust, exe.path(), &profile),
        _ => Err("ação desconhecida".into()),
    }
}

fn profile_dir(root: &Path, connection_id: &str) -> Result<PathBuf, String> {
    let dir = root.join(connection_id);
    std::fs::create_dir_all(&dir).map_err(|_| "perfil da conta indisponível".to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
            .map_err(|_| "permissão do perfil não aplicada".to_string())?;
    }
    Ok(dir)
}

#[allow(clippy::too_many_arguments)]
fn login(
    central: &Central,
    s: &CliSession,
    engine: Engine,
    kind: EngineKind,
    host: &SystemHost,
    trust: &StoreTrust,
    exe: &Path,
    profile: &Path,
) -> Result<(), String> {
    let cancel = Arc::new(AtomicBool::new(false));
    let done = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel::<String>();

    // vigia: cancelamento e código colado, sem bloquear a leitura da saída da CLI
    {
        let cancel = cancel.clone();
        let done = done.clone();
        let session = s.id.clone();
        let needs_code = engine.needs_pasted_code();
        let watcher = Central::new(central.base(), central.token()).map_err(|_| "cliente indisponível".to_string())?;
        std::thread::spawn(move || {
            while !done.load(Ordering::SeqCst) {
                std::thread::sleep(CONTROL_POLL);
                if let Ok((cancelled, pasted)) = watcher.cli_control(&session) {
                    if cancelled {
                        cancel.store(true, Ordering::SeqCst);
                        return;
                    }
                    if let (true, Some(code)) = (needs_code, pasted) {
                        let _ = tx.send(code);
                    }
                }
            }
        });
    }

    let pasted = engine.needs_pasted_code().then_some(rx);
    let outcome = cli_login::run_login(engine, exe, profile, LOGIN_TIMEOUT, &cancel, pasted, &mut |hint| {
        let event = match hint {
            Hint::Url(url) => json!({ "kind": "url", "url": url }),
            Hint::DeviceCode(code) => json!({ "kind": "device_code", "code": code }),
            Hint::AwaitingPastedCode => json!({ "kind": "awaiting_code" }),
        };
        // um evento perdido não interrompe o login: o estado final é informado ao fim
        let _ = central.cli_event(&s.id, event);
    });
    done.store(true, Ordering::SeqCst);

    match outcome {
        Outcome::Exited => {
            let auth = auth_of(kind, host, trust);
            if auth == "authenticated" {
                finish(central, &s.id, "succeeded", auth, None)
            } else {
                finish(central, &s.id, "failed", auth, Some("login não confirmado pelo status da CLI"))
            }
        }
        Outcome::Failed(msg) => finish(central, &s.id, "failed", "unknown", Some(&msg)),
        Outcome::TimedOut => finish(central, &s.id, "timed_out", "unknown", None),
        Outcome::Cancelled => finish(central, &s.id, "cancelled", "unknown", None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_ids_cannot_escape_the_profiles_folder() {
        assert!(safe_id("8c2f3d1e-1111-4a2b-9c3d-0123456789ab"));
        assert!(!safe_id("../../etc/passwd"));
        assert!(!safe_id("8c2f3d1e-1111-4a2b-9c3d-0123456789a/"));
        assert!(!safe_id(""));
    }

    #[test]
    fn auth_states_map_to_the_central_vocabulary() {
        assert_eq!(auth_word(AuthState::LoggedIn), "authenticated");
        assert_eq!(auth_word(AuthState::LoggedOut), "unauthenticated");
        assert_eq!(auth_word(AuthState::Unknown), "unknown");
    }
}
