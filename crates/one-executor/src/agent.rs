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
    // contas de CLI: só pede uma sessão quando há perfis configurados e espaço para executá-la
    if central.profiles().is_some() && crate::cli_session::has_capacity() {
        if let Some(session) = central.next_cli_session()? {
            crate::cli_session::spawn(central, session, trust);
            return Ok(true);
        }
    }
    let Some(job) = central.next_job()? else {
        return Ok(false);
    };
    let Some(connection_id) = job.cli_connection_id.as_deref() else {
        central.finish(
            &job.id,
            &job.lease_token,
            false,
            None,
            &[],
            Some("Executor sem conexão Codex autenticada e autorizada"),
        )?;
        return Ok(true);
    };
    if !crate::cli_session::safe_id(connection_id) {
        central.finish(
            &job.id,
            &job.lease_token,
            false,
            None,
            &[],
            Some("perfil Codex inválido"),
        )?;
        return Ok(true);
    }
    let Some(profiles) = central.profiles() else {
        central.finish(
            &job.id,
            &job.lease_token,
            false,
            None,
            &[],
            Some("diretório de perfis não configurado"),
        )?;
        return Ok(true);
    };
    let profile = crate::cli_session::profile_dir(profiles, connection_id)
        .map_err(|_| CentralError::Transient("perfil Codex indisponível".into()))?;
    let Some(profile_host) = host.with_profile_home(&profile) else {
        central.finish(
            &job.id,
            &job.lease_token,
            false,
            None,
            &[],
            Some("host não suporta perfil isolado"),
        )?;
        return Ok(true);
    };
    handle_job(central, profile_host.as_ref(), trust, work, &job, policy)?;
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
