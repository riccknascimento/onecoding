//! Um job de análise: monta o snapshot pedindo arquivos ao OneNode (pelo Central), roda o Codex sobre ele,
//! e devolve resposta e evidências. O snapshot fica numa pasta própria do job e é apagado ao final.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use one_adapters::Host;
use one_exec::StoreTrust;
use one_node::tools::is_sensitive_name;
use serde_json::{json, Value};

use crate::central::{Central, CentralError, Job, ToolBroker};
use crate::codex::run_codex_with_instructions;

/// Intervalo em que o executor pergunta ao Central se o cancelamento foi pedido (e renova o lease).
const CANCEL_CHECK_EVERY: Duration = Duration::from_secs(2);
/// Progresso do Codex enviado por job. Passou disso, o restante só segue no resultado final.
const MAX_FORWARDED_EVENTS: usize = 40;
/// Tentativas para registrar o resultado quando o Central está fora (reinício): 2, 4, 8 e 16 s.
const MAX_FINISH_TRIES: u32 = 5;

#[derive(Debug, Clone)]
pub struct SnapshotPolicy {
    pub max_files: usize,
    pub max_file_bytes: usize,
    pub poll: Duration,
    pub task_timeout: Duration,
}

impl Default for SnapshotPolicy {
    fn default() -> Self {
        Self {
            max_files: 12,
            max_file_bytes: 16 * 1024,
            poll: Duration::from_millis(500),
            task_timeout: Duration::from_secs(60),
        }
    }
}

/// Espera uma tarefa remota até um estado final. Falha ou tempo esgotado viram erro.
fn wait_task(
    broker: &dyn ToolBroker,
    job: &str,
    task: &str,
    policy: &SnapshotPolicy,
) -> Result<Value, String> {
    let started = Instant::now();
    loop {
        let (status, result) = broker
            .tool_status(job, task)
            .map_err(|e| format!("{e:?}"))?;
        match status.as_str() {
            "succeeded" => return Ok(result),
            "failed" | "cancelled" => {
                return Err(result
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("ferramenta falhou")
                    .to_string())
            }
            _ => {}
        }
        if started.elapsed() > policy.task_timeout {
            return Err("o OneNode não respondeu a tempo".into());
        }
        std::thread::sleep(policy.poll);
    }
}

fn safe_name(name: &str) -> bool {
    !name.is_empty() && !name.contains('/') && !name.contains('\\') && name != "." && name != ".."
}

/// Monta o snapshot do diretório raiz do projeto: arquivos regulares, não sensíveis, com limite de quantidade
/// e de bytes. Só cria arquivos comuns dentro de `dir`. Retorna os nomes copiados (evidências).
pub fn build_snapshot(
    broker: &dyn ToolBroker,
    job: &str,
    lease: &str,
    dir: &Path,
    policy: &SnapshotPolicy,
    cancel: &AtomicBool,
) -> Result<Vec<String>, String> {
    let task = broker
        .request_tool(job, lease, "files_list", ".")
        .map_err(|e| format!("{e:?}"))?;
    let listing = wait_task(broker, job, &task, policy)?;
    let entries = listing
        .get("entries")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut copied = Vec::new();
    for e in entries {
        if cancel.load(Ordering::SeqCst) {
            return Err("cancelado".into());
        }
        if copied.len() >= policy.max_files {
            break;
        }
        let name = e.get("name").and_then(Value::as_str).unwrap_or("");
        let kind = e.get("kind").and_then(Value::as_str).unwrap_or("");
        if kind != "file" || !safe_name(name) || is_sensitive_name(Path::new(name)) {
            continue;
        }
        let read = broker
            .request_tool(job, lease, "files_read", name)
            .map_err(|e| format!("{e:?}"))?;
        let Ok(result) = wait_task(broker, job, &read, policy) else {
            continue;
        };
        let Some(content) = result.get("content").and_then(Value::as_str) else {
            continue;
        };
        let mut cut = content.len().min(policy.max_file_bytes);
        while !content.is_char_boundary(cut) {
            cut -= 1;
        }
        std::fs::write(dir.join(name), &content[..cut])
            .map_err(|_| "não foi possível gravar o snapshot".to_string())?;
        copied.push(name.to_string());
    }
    Ok(copied)
}

/// Pasta do job: nome restrito a caracteres seguros (o id vem do Central).
pub fn job_dir(work: &Path, job: &str) -> PathBuf {
    let safe: String = job
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect();
    work.join(format!("job-{safe}"))
}

/// Executa um job completo e registra o resultado no Central. Sempre apaga a pasta do snapshot.
/// Um vigia consulta o Central enquanto o job roda: se o cancelamento foi pedido, o Codex é encerrado e o
/// job é confirmado como cancelado. O status só vira `cancelled` por essa confirmação.
pub fn handle_job(
    central: &Central,
    host: &dyn Host,
    trust: &StoreTrust,
    work: &Path,
    job: &Job,
    policy: &SnapshotPolicy,
) -> Result<(), CentralError> {
    central.event(&job.id, &job.lease_token, "started", json!({}))?;
    let dir = job_dir(work, &job.id);
    let cancel = AtomicBool::new(false);
    let done = AtomicBool::new(false);
    let outcome = std::thread::scope(|scope| {
        scope.spawn(|| watch_cancel(central, job, &cancel, &done));
        let outcome = run_job(central, host, trust, &dir, job, policy, &cancel);
        done.store(true, Ordering::SeqCst);
        outcome
    });
    let _ = std::fs::remove_dir_all(&dir);
    if cancel.load(Ordering::SeqCst) {
        return with_retry(|| central.finish_cancelled(&job.id, &job.lease_token));
    }
    match outcome {
        Ok((answer, evidence)) => with_retry(|| {
            central.finish(
                &job.id,
                &job.lease_token,
                true,
                Some(&answer),
                &evidence,
                None,
            )
        }),
        Err(msg) => {
            with_retry(|| central.finish(&job.id, &job.lease_token, false, None, &[], Some(&msg)))
        }
    }
}

/// Repete só falhas transitórias (Central fora). Recusas definitivas (lease vencido, token) não são repetidas.
fn with_retry(mut attempt: impl FnMut() -> Result<(), CentralError>) -> Result<(), CentralError> {
    let mut wait = Duration::from_secs(2);
    let mut tries = 1;
    loop {
        match attempt() {
            Err(CentralError::Transient(_)) if tries < MAX_FINISH_TRIES => {
                std::thread::sleep(wait);
                wait *= 2;
                tries += 1;
            }
            other => return other,
        }
    }
}

fn run_job(
    central: &Central,
    host: &dyn Host,
    trust: &StoreTrust,
    dir: &Path,
    job: &Job,
    policy: &SnapshotPolicy,
    cancel: &AtomicBool,
) -> Result<(String, Vec<String>), String> {
    std::fs::create_dir_all(dir)
        .map_err(|_| "não foi possível criar a pasta do job".to_string())?;
    if job.kind == "delegation" {
        // delegação de OneAgent: workspace vazio e isolado; nenhum arquivo de projeto é lido
        let mut sent = 0usize;
        let mut forward = |kind: &str, payload: Value| {
            if sent < MAX_FORWARDED_EVENTS {
                sent += 1;
                let _ = central.event(&job.id, &job.lease_token, kind, payload);
            }
        };
        forward("codex_started", json!({ "modo": "delegacao" }));
        let answer = crate::codex::run_codex_prompt(host, trust, dir, &job.question, cancel, &mut forward)?;
        return Ok((answer, Vec::new()));
    }
    let evidence = build_snapshot(central, &job.id, &job.lease_token, dir, policy, cancel)?;
    let _ = central.event(
        &job.id,
        &job.lease_token,
        "snapshot",
        json!({ "arquivos": evidence.len() }),
    );
    let mut sent = 0usize;
    let mut forward = |kind: &str, payload: Value| {
        if sent < MAX_FORWARDED_EVENTS {
            sent += 1;
            let _ = central.event(&job.id, &job.lease_token, kind, payload);
        }
    };
    forward("codex_started", json!({}));
    let answer = run_codex_with_instructions(
        host,
        trust,
        dir,
        &job.question,
        &job.instructions,
        cancel,
        &mut forward,
    )?;
    Ok((answer, evidence))
}

/// Pergunta ao Central, a cada intervalo, se o cancelamento foi pedido. A mesma chamada renova o lease.
/// Lease perdido ou token recusado também param o Codex: o Central não aceitaria o resultado mesmo.
fn watch_cancel(central: &Central, job: &Job, cancel: &AtomicBool, done: &AtomicBool) {
    while !done.load(Ordering::SeqCst) {
        match central.job_heartbeat(&job.id, &job.lease_token) {
            Ok(true) => cancel.store(true, Ordering::SeqCst),
            Ok(false) | Err(CentralError::Transient(_)) => {}
            Err(CentralError::Conflict(_) | CentralError::Unauthorized) => {
                cancel.store(true, Ordering::SeqCst)
            }
        }
        let waited = Instant::now();
        while waited.elapsed() < CANCEL_CHECK_EVERY && !done.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transient_failures_are_retried_but_refusals_are_not() {
        let mut calls = 0;
        let ok = with_retry(|| {
            calls += 1;
            if calls < 2 {
                Err(CentralError::Transient("conexão indisponível".into()))
            } else {
                Ok(())
            }
        });
        assert!(ok.is_ok());
        assert_eq!(calls, 2);

        let mut refusals = 0;
        let refused = with_retry(|| {
            refusals += 1;
            Err(CentralError::Conflict("stale_lease".into()))
        });
        assert!(matches!(refused, Err(CentralError::Conflict(_))));
        assert_eq!(refusals, 1, "recusa definitiva não é repetida");
    }
    use crate::central::CentralError;
    use serde_json::json;
    use std::sync::Mutex;

    /// Broker falso: responde a listagem e a leitura sem rede.
    struct FakeBroker {
        log: Mutex<Vec<(String, String)>>,
    }

    impl ToolBroker for FakeBroker {
        fn request_tool(
            &self,
            _job: &str,
            _lease: &str,
            tool: &str,
            path: &str,
        ) -> Result<String, CentralError> {
            self.log.lock().unwrap().push((tool.into(), path.into()));
            Ok(format!("t{}", self.log.lock().unwrap().len()))
        }
        fn tool_status(&self, _job: &str, task: &str) -> Result<(String, Value), CentralError> {
            let last = self.log.lock().unwrap().last().cloned().unwrap();
            let n: usize = task[1..].parse().unwrap();
            let _ = n;
            Ok(match last.0.as_str() {
                "files_list" => (
                    "succeeded".into(),
                    json!({"entries": [
                        {"name": "README.md", "kind": "file"},
                        {"name": ".env", "kind": "file"},
                        {"name": "chave.pem", "kind": "file"},
                        {"name": "sub", "kind": "dir"},
                        {"name": "../fora.txt", "kind": "file"},
                        {"name": "grande.txt", "kind": "file"},
                        {"name": "extra.md", "kind": "file"}
                    ]}),
                ),
                _ => {
                    let body = if last.1 == "grande.txt" {
                        "é".repeat(20_000)
                    } else {
                        format!("conteúdo de {}", last.1)
                    };
                    ("succeeded".into(), json!({"content": body}))
                }
            })
        }
    }

    #[test]
    fn snapshot_copies_only_safe_regular_files_within_limits() {
        let dir = std::env::temp_dir().join(format!("oa-snap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let broker = FakeBroker {
            log: Mutex::new(vec![]),
        };
        let policy = SnapshotPolicy {
            max_files: 3,
            max_file_bytes: 1000,
            poll: Duration::from_millis(1),
            task_timeout: Duration::from_secs(5),
        };
        let copied =
            build_snapshot(&broker, "j", "l", &dir, &policy, &AtomicBool::new(false)).unwrap();
        // nunca lê segredos, nomes com ".." nem subpastas; respeita o limite de arquivos
        assert_eq!(copied, vec!["README.md", "grande.txt", "extra.md"]);
        let asked: Vec<String> = broker
            .log
            .lock()
            .unwrap()
            .iter()
            .map(|(_, p)| p.clone())
            .collect();
        assert!(
            !asked
                .iter()
                .any(|p| p == ".env" || p == "chave.pem" || p.contains("..")),
            "{asked:?}"
        );
        // limite de bytes respeitado e sem corte no meio de caractere UTF-8
        let grande = std::fs::read_to_string(dir.join("grande.txt")).unwrap();
        assert!(grande.len() <= 1000 && grande.chars().all(|c| c == 'é'));
        // só arquivos comuns foram criados dentro do snapshot
        assert!(!dir.join("../fora.txt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn failing_tool_task_becomes_an_error_with_timeout() {
        struct Failing;
        impl ToolBroker for Failing {
            fn request_tool(
                &self,
                _: &str,
                _: &str,
                _: &str,
                _: &str,
            ) -> Result<String, CentralError> {
                Ok("t1".into())
            }
            fn tool_status(&self, _: &str, _: &str) -> Result<(String, Value), CentralError> {
                Ok(("running".into(), Value::Null))
            }
        }
        let policy = SnapshotPolicy {
            max_files: 1,
            max_file_bytes: 10,
            poll: Duration::from_millis(1),
            task_timeout: Duration::from_millis(20),
        };
        let dir = std::env::temp_dir();
        assert_eq!(
            build_snapshot(&Failing, "j", "l", &dir, &policy, &AtomicBool::new(false)).unwrap_err(),
            "o OneNode não respondeu a tempo"
        );
    }

    #[test]
    fn job_folder_name_is_restricted_to_safe_characters() {
        let p = job_dir(Path::new("/tmp/w"), "../../etc/abc-123");
        assert_eq!(p, Path::new("/tmp/w/job-etcabc-123"));
    }
}
