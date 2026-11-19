//! Configuração do OneExecutor. Campos desconhecidos são recusados (nenhuma credencial entra por aqui).

use std::path::PathBuf;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorConfig {
    /// OneHub Central (HTTPS; HTTP só em loopback).
    pub central_url: String,
    /// Identidade do executor no Central (id e token), permissão 0600.
    pub state_file: PathBuf,
    /// Banco do OneAgent onde a aprovação da CLI (Codex) é verificada. Nunca copiado para outro lugar.
    pub trust_db: PathBuf,
    /// Raiz temporária dos snapshots. Cada job tem a própria pasta, apagada ao final.
    pub work_dir: PathBuf,
    /// Perfis isolados das contas de CLI (um HOME por conta). Padrão: `profiles` ao lado de `work_dir`.
    #[serde(default)]
    pub profiles_dir: Option<PathBuf>,
}

impl ExecutorConfig {
    pub fn profiles_root(&self) -> PathBuf {
        self.profiles_dir.clone().unwrap_or_else(|| {
            self.work_dir.parent().unwrap_or(&self.work_dir).join("profiles")
        })
    }
}

pub fn parse(text: &str) -> Result<ExecutorConfig, String> {
    let cfg: ExecutorConfig =
        serde_json::from_str(text).map_err(|_| "configuração inválida".to_string())?;
    let loopback = cfg
        .central_url
        .strip_prefix("http://")
        .and_then(|rest| rest.split('/').next())
        .is_some_and(|h| h.starts_with("127.0.0.1:") || h.starts_with("localhost:"));
    if !(cfg.central_url.starts_with("https://") || loopback) {
        return Err("central_url precisa ser https (ou http apenas em loopback)".into());
    }
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_credential_fields_and_remote_plain_http() {
        let base = r#""state_file":"/x","trust_db":"/y","work_dir":"/z""#;
        assert!(parse(&format!(
            r#"{{"central_url":"https://h.exemplo",{base},"openai_api_key":"sk-x"}}"#
        ))
        .is_err());
        assert!(parse(&format!(
            r#"{{"central_url":"http://187.77.52.164:8787",{base}}}"#
        ))
        .is_err());
        assert!(parse(&format!(
            r#"{{"central_url":"http://127.0.0.1:8787",{base}}}"#
        ))
        .is_ok());
    }
}
