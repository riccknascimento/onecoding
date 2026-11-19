//! Identidade do OneNode no Central (id e token). Arquivo com permissão 0600; o token nunca é impresso.

use std::io::Write;
use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct NodeIdentity {
    pub node_id: String,
    pub token: String,
}

pub fn save(path: &Path, id: &NodeIdentity) -> Result<(), String> {
    let json =
        serde_json::to_string(id).map_err(|_| "falha ao serializar identidade".to_string())?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|_| "não foi possível criar a pasta de estado".to_string())?;
    }
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts
        .open(path)
        .map_err(|_| "não foi possível gravar a identidade".to_string())?;
    f.write_all(json.as_bytes())
        .map_err(|_| "não foi possível gravar a identidade".to_string())
}

pub fn load(path: &Path) -> Result<NodeIdentity, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|_| "identidade do OneNode ausente: rode o pareamento".to_string())?;
    serde_json::from_str(&text).map_err(|_| "identidade do OneNode corrompida".to_string())
}
