//! Estado real do motor Codex neste executor: instalado, aprovado e autenticado. A sonda nunca executa um
//! binário que não esteja aprovado; o login é verificado pelo status oficial da CLI, sem ler credenciais.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use one_adapters::{acquire, Host, Limits, Outcome, TrustStore};
use one_protocol::EngineKind;
use serde::Serialize;

const PROBE_TTL: Duration = Duration::from_secs(60);
const PROBE_LIMITS: Limits = Limits { timeout: Duration::from_secs(10), max_output: 4096 };

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EngineReport {
    pub name: &'static str,
    pub installed: bool,
    pub authenticated: bool,
    pub approved: bool,
    pub version: Option<String>,
}

/// Interpreta a saída de `codex login status`. Estado "não autenticado" nunca vira autenticado.
pub fn logged_in(status_output: &str) -> bool {
    let s = status_output.to_lowercase();
    s.contains("logged in") && !s.contains("not logged in")
}

pub fn probe_codex(host: &dyn Host, trust: &dyn TrustStore) -> EngineReport {
    let installed = host.inspect("codex").is_some();
    match acquire(EngineKind::Codex, host, trust) {
        Ok(exe) => {
            let status = host.run(&exe, &["login", "status"], PROBE_LIMITS);
            let authenticated = matches!(status.outcome, Outcome::Exited(Some(0))) && logged_in(&status.stdout);
            let version = host
                .run(&exe, &["--version"], PROBE_LIMITS)
                .stdout
                .lines()
                .next()
                .map(|l| l.trim().chars().take(40).collect::<String>())
                .filter(|v| !v.is_empty());
            EngineReport { name: "codex", installed, authenticated, approved: true, version }
        }
        Err(_) => EngineReport { name: "codex", installed, authenticated: false, approved: false, version: None },
    }
}

static CACHE: Mutex<Option<(Instant, EngineReport)>> = Mutex::new(None);

/// Relatório atual, com cache curto: a sonda roda no máximo a cada `PROBE_TTL`.
pub fn current(host: &dyn Host, trust: &dyn TrustStore) -> Vec<EngineReport> {
    let mut cache = CACHE.lock().expect("cache de motores");
    if let Some((at, rep)) = cache.as_ref() {
        if at.elapsed() < PROBE_TTL {
            return vec![rep.clone()];
        }
    }
    let rep = probe_codex(host, trust);
    *cache = Some((Instant::now(), rep.clone()));
    vec![rep]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_logged_in_is_never_read_as_authenticated() {
        assert!(!logged_in("Not logged in\n"));
        assert!(!logged_in(""));
        assert!(logged_in("Logged in using ChatGPT"));
    }
}
