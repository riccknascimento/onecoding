//! Laço do OneNode: heartbeat, consulta de tarefas, execução confinada e registro de eventos e resultado.
//! Idempotência: o Central só entrega tarefas `queued` e só aceita finalização do lease vigente; o nó não
//! refaz uma tarefa que o Central já considera concluída.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::central::{Central, CentralError, Task};
use crate::tools;

/// Margem antes do vencimento do lease (300 s no Central). Execução que passa disso não é registrada.
pub const LEASE_BUDGET: Duration = Duration::from_secs(240);
pub const ALLOWED_TOOLS: &[&str] = &["files_list", "files_read"];

#[derive(Debug, PartialEq, Eq)]
pub enum Cycle {
    Idle,
    Handled { task: String, status: &'static str },
}

#[derive(Clone, Default)]
pub struct Roots(pub Vec<(String, PathBuf)>);

impl Roots {
    pub fn root_of(&self, project_id: &str) -> Option<&PathBuf> {
        self.0.iter().find(|(p, _)| p == project_id).map(|(_, r)| r)
    }
}

/// Decide o resultado de uma tarefa sem registrar nada (testável).
pub fn execute(roots: &Roots, task: &Task) -> (&'static str, Value) {
    let Some(root) = roots.root_of(&task.project_id) else {
        return (
            "failed",
            json!({ "error": "projeto não autorizado neste OneNode" }),
        );
    };
    let tool = match task.tool.as_deref() {
        Some(t) if ALLOWED_TOOLS.contains(&t) => t,
        _ => {
            return (
                "failed",
                json!({ "error": "ferramenta não permitida neste OneNode" }),
            )
        }
    };
    let started = Instant::now();
    let outcome = tools::run(root, tool, &task.args);
    if started.elapsed() > LEASE_BUDGET {
        // Lease quase vencido: não registra. O Central já terá devolvido a tarefa à fila.
        return (
            "abandoned",
            json!({ "error": "tempo de execução acima do lease" }),
        );
    }
    match outcome {
        Ok(v) => ("succeeded", v),
        Err(msg) => ("failed", json!({ "error": msg })),
    }
}

/// Um ciclo: pega no máximo uma tarefa, executa e registra. Retorna `Idle` se não há nada.
pub fn run_once(central: &Central, roots: &Roots) -> Result<Cycle, CentralError> {
    central.heartbeat()?;
    let Some(task) = central.next_task()? else {
        return Ok(Cycle::Idle);
    };
    central.event(
        &task.id,
        &task.lease_token,
        0,
        "started",
        json!({ "tool": task.tool }),
    )?;
    let (status, result) = execute(roots, &task);
    if status == "abandoned" {
        return Ok(Cycle::Handled {
            task: task.id,
            status,
        });
    }
    central.event(
        &task.id,
        &task.lease_token,
        1,
        "finished",
        json!({ "status": status }),
    )?;
    central.finish(
        &task.id,
        &task.lease_token,
        if status == "succeeded" {
            "succeeded"
        } else {
            "failed"
        },
        result,
    )?;
    Ok(Cycle::Handled {
        task: task.id,
        status,
    })
}

/// Laço contínuo com espera e recuo. Para só em revogação/token inválido.
/// Laço do serviço. Antes de cada ciclo aplica o catálogo do OneHub (sem reiniciar). Suspensão espera e
/// tenta de novo; só token inválido (revogação) encerra o serviço.
pub fn run_forever(central: &Central, policy_path: &std::path::Path, catalog: &mut crate::catalog::Catalog) -> Result<(), String> {
    let mut backoff = Duration::from_secs(2);
    loop {
        let step = crate::catalog::sync(central, policy_path, catalog).and_then(|_| run_once(central, &catalog.roots));
        match step {
            Ok(Cycle::Idle) => {
                backoff = Duration::from_secs(2);
                std::thread::sleep(Duration::from_secs(5));
            }
            Ok(Cycle::Handled { .. }) => backoff = Duration::from_secs(2),
            Err(CentralError::Unauthorized) => {
                return Err("OneNode revogado ou token inválido: pare e pareie de novo".into())
            }
            Err(CentralError::Suspended) => {
                // suspenso: não é falha. Espera mais e tenta de novo, sem encerrar o serviço
                std::thread::sleep(Duration::from_secs(60));
                backoff = Duration::from_secs(2);
            }
            Err(CentralError::Conflict(_)) => backoff = Duration::from_secs(2),
            Err(CentralError::Transient(_)) => {
                std::thread::sleep(backoff);
                backoff = (backoff * 2).min(Duration::from_secs(60));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(project: &str, tool: Option<&str>, args: Value) -> Task {
        Task {
            id: "t".into(),
            project_id: project.into(),
            tool: tool.map(String::from),
            args,
            lease_token: "l".into(),
        }
    }

    #[test]
    fn task_for_unauthorized_project_is_refused_without_running_anything() {
        let roots = Roots(vec![]);
        let (status, res) = execute(
            &roots,
            &task("outro", Some("files_list"), json!({"path": "."})),
        );
        assert_eq!(status, "failed");
        assert!(res["error"].as_str().unwrap().contains("não autorizado"));
    }

    #[test]
    fn tools_outside_the_allowlist_are_refused() {
        let roots = Roots(vec![("p".into(), std::env::temp_dir())]);
        let (status, _) = execute(&roots, &task("p", Some("shell"), json!({"path": "."})));
        assert_eq!(status, "failed");
        let (status, _) = execute(&roots, &task("p", None, json!({"path": "."})));
        assert_eq!(status, "failed");
    }
}
