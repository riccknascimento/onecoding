//! Binário do OneNode.
//! - `one-node pair`: usa `ONEAGENT_NODE_CONFIG` e `ONEAGENT_NODE_ID`; o código vem de `ONEAGENT_NODE_PAIR_CODE`
//!   ou, se ausente, da entrada padrão (assim não aparece em `ps`). Grava a identidade no `state_file` (0600).
//! - `one-node run`: usa `ONEAGENT_NODE_CONFIG`, a identidade gravada e a política local
//!   (`ONEAGENT_NODE_POLICY`, padrão `/etc/onenode/policy.json`). Projetos vêm do OneHub.

use one_node::catalog::Catalog;
use one_node::central::Central;
use one_node::{agent, config, state};
use std::io::BufRead;

fn load_config() -> Result<config::NodeConfig, String> {
    let path = std::env::var("ONEAGENT_NODE_CONFIG").map_err(|_| "defina ONEAGENT_NODE_CONFIG")?;
    let text = std::fs::read_to_string(&path)
        .map_err(|_| "configuração do OneNode não encontrada".to_string())?;
    config::parse(&text)
}

fn main() -> Result<(), String> {
    let cfg = load_config()?;
    match std::env::args().nth(1).as_deref() {
        Some("pair") => {
            let node_id =
                std::env::var("ONEAGENT_NODE_ID").map_err(|_| "defina ONEAGENT_NODE_ID")?;
            let code = match std::env::var("ONEAGENT_NODE_PAIR_CODE") {
                Ok(c) => c,
                Err(_) => {
                    let mut line = String::new();
                    std::io::stdin()
                        .lock()
                        .read_line(&mut line)
                        .map_err(|_| "não foi possível ler o código de pareamento")?;
                    line.trim().to_string()
                }
            };
            let token = Central::pair(&cfg.central_url, &node_id, &code)
                .map_err(|e| format!("pareamento recusado: {e:?}"))?;
            state::save(
                &cfg.state_file,
                &state::NodeIdentity {
                    node_id: node_id.clone(),
                    token,
                },
            )?;
            println!("OneNode pareado: {node_id}");
            Ok(())
        }
        Some("run") => {
            let id = state::load(&cfg.state_file)?;
            let policy = std::env::var("ONEAGENT_NODE_POLICY")
                .unwrap_or_else(|_| "/etc/onenode/policy.json".to_string());
            let central = Central::new(&cfg.central_url, &id.token)?;
            eprintln!("OneNode {} iniciado; catálogo do OneHub será aplicado", id.node_id);
            agent::run_forever(&central, std::path::Path::new(&policy), &mut Catalog::default())
        }
        _ => Err("uso: one-node pair | one-node run".into()),
    }
}
