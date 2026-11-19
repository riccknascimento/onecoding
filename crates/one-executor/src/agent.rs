//! Laço do OneExecutor: heartbeat, pega um job com lease, executa e registra. Falhas transitórias recuam;
//! revogação ou token inválido param o processo.

use std::path::Path;
use std::time::Duration;

use one_adapters::Host;
use one_exec::StoreTrust;

use crate::central::{Central, CentralError};
use crate::job::{handle_job, SnapshotPolicy};

/// Um ciclo: retorna `false` se não havia job.
pub fn run_once(
    central: &Central,
    host: &dyn Host,
    trust: &StoreTrust,
    work: &Path,
    policy: &SnapshotPolicy,
) -> Result<bool, CentralError> {
    let engines = crate::engines::current(host, trust);
    central.heartbeat(&engines)?;
    let Some(job) = central.next_job()? else {
        return Ok(false);
    };
    handle_job(central, host, trust, work, &job, policy)?;
    Ok(true)
}

pub fn run_forever(
    central: &Central,
    host: &dyn Host,
    trust: &StoreTrust,
    work: &Path,
) -> Result<(), String> {
    let policy = SnapshotPolicy::default();
    let mut backoff = Duration::from_secs(2);
    loop {
        match run_once(central, host, trust, work, &policy) {
            Ok(false) => {
                backoff = Duration::from_secs(2);
                std::thread::sleep(Duration::from_secs(5));
            }
            Ok(true) => backoff = Duration::from_secs(2),
            Err(CentralError::Unauthorized) => {
                return Err("OneExecutor revogado ou token inválido: pare e pareie de novo".into())
            }
            Err(CentralError::Conflict(_)) => backoff = Duration::from_secs(2),
            Err(CentralError::Transient(_)) => {
                std::thread::sleep(backoff);
                backoff = (backoff * 2).min(Duration::from_secs(60));
            }
        }
    }
}
