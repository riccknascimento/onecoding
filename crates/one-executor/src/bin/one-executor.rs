//! Binário do OneExecutor.
//! - `one-executor pair`: `ONEAGENT_EXECUTOR_CONFIG`, `ONEAGENT_EXECUTOR_ID` e `ONEAGENT_EXECUTOR_PAIR_CODE`.
//!   Grava a identidade no `state_file` (0600). O código é de uso único.
//! - `one-executor run`: `ONEAGENT_EXECUTOR_CONFIG` e a identidade gravada.
//! - `one-executor approve`: mostra o binário do Codex encontrado e sua impressão digital.
//! - `one-executor approve <fingerprint>`: aprova o Codex SOMENTE se a impressão digital confere com o binário
//!   atual. Ato administrativo deliberado: nada é aprovado sem essa confirmação.

use std::sync::{Arc, Mutex};

use one_adapters::SystemHost;
use one_exec::StoreTrust;
use one_executor::agent;
use one_executor::central::Central;
use one_executor::config;
use one_node::state;

fn load_config() -> Result<config::ExecutorConfig, String> {
    let path =
        std::env::var("ONEAGENT_EXECUTOR_CONFIG").map_err(|_| "defina ONEAGENT_EXECUTOR_CONFIG")?;
    let text = std::fs::read_to_string(&path)
        .map_err(|_| "configuração do OneExecutor não encontrada".to_string())?;
    config::parse(&text)
}

fn main() -> Result<(), String> {
    let cfg = load_config()?;
    match std::env::args().nth(1).as_deref() {
        Some("pair") => {
            let id =
                std::env::var("ONEAGENT_EXECUTOR_ID").map_err(|_| "defina ONEAGENT_EXECUTOR_ID")?;
            let code = std::env::var("ONEAGENT_EXECUTOR_PAIR_CODE")
                .map_err(|_| "defina ONEAGENT_EXECUTOR_PAIR_CODE")?;
            let token = Central::pair(&cfg.central_url, &id, &code)
                .map_err(|e| format!("pareamento recusado: {e:?}"))?;
            state::save(
                &cfg.state_file,
                &state::NodeIdentity {
                    node_id: id.clone(),
                    token,
                },
            )?;
            println!("OneExecutor pareado: {id}");
            Ok(())
        }
        Some("run") => {
            let identity = state::load(&cfg.state_file)?;
            let central = Central::new(&cfg.central_url, &identity.token)?.with_profiles(cfg.profiles_root());
            // aprovação da CLI: a mesma do app (banco do OneAgent, somente o registro do Codex)
            let store = one_store::Store::open(&cfg.trust_db)
                .map_err(|_| "não foi possível abrir o registro de aprovações".to_string())?;
            let trust = StoreTrust(Arc::new(Mutex::new(store)));
            let host = SystemHost::new();
            eprintln!("OneExecutor {} ativo", identity.node_id);
            agent::run_forever(&central, &host, &trust, &cfg.work_dir)
        }
        Some("approve") => {
            let store = one_store::Store::open(&cfg.trust_db)
                .map_err(|_| "não foi possível abrir o registro de aprovações".to_string())?;
            let trust = StoreTrust(Arc::new(Mutex::new(store)));
            let host = SystemHost::new();
            let c = one_adapters::candidate(one_protocol::EngineKind::Codex, &host, &trust)
                .ok_or("Codex não encontrado neste servidor")?;
            match std::env::args().nth(2) {
                None => {
                    println!("Codex encontrado: {}", c.path);
                    println!("impressão digital: {}", c.fingerprint);
                    println!("estado atual: {:?}", c.state);
                    println!("para aprovar: one-executor approve {}", c.fingerprint);
                    Ok(())
                }
                Some(fp) if fp == c.fingerprint => {
                    one_adapters::TrustStore::approve(&trust, one_protocol::EngineKind::Codex, &c.fingerprint, &c.path)?;
                    println!("Codex aprovado: {}", c.fingerprint);
                    Ok(())
                }
                Some(_) => Err("impressão digital diferente do binário atual: nada foi aprovado".into()),
            }
        }
        _ => Err("uso: one-executor pair | run | approve [fingerprint]".into()),
    }
}
