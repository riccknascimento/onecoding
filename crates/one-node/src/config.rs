//! Configuração do OneNode. Lista explícita de projetos e raízes; campos desconhecidos são recusados,
//! para que nenhuma credencial possa ser injetada pela configuração.

use std::path::PathBuf;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeConfig {
    /// URL do OneHub Central. HTTP simples só para loopback (testes e túnel SSH); fora disso, HTTPS.
    pub central_url: String,
    /// Arquivo com a identidade do nó (id e token), permissão 0600. Nunca vai para o Git.
    pub state_file: PathBuf,
    pub projects: Vec<ProjectRoot>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectRoot {
    /// Id do projeto no OneHub Central (o nó só atende projetos listados aqui E concedidos pelo Central).
    pub project_id: String,
    /// Pasta do workspace neste servidor. Validada e canonicalizada ao carregar.
    pub root: PathBuf,
}

pub fn parse(text: &str) -> Result<NodeConfig, String> {
    let cfg: NodeConfig =
        serde_json::from_str(text).map_err(|_| "configuração inválida".to_string())?;
    if !(cfg.central_url.starts_with("https://") || is_loopback_http(&cfg.central_url)) {
        return Err("central_url precisa ser https (ou http apenas em loopback)".into());
    }
    // Projetos agora vêm do catálogo do OneHub, aplicado pela política local. Lista local é recusada
    // para não haver duas fontes de verdade.
    if !cfg.projects.is_empty() {
        return Err("projetos vêm do OneHub: remova \"projects\" do node.json".into());
    }
    Ok(cfg)
}

fn is_loopback_http(url: &str) -> bool {
    url.strip_prefix("http://")
        .and_then(|rest| rest.split('/').next())
        .map(|host| host.starts_with("127.0.0.1:") || host.starts_with("localhost:"))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_unknown_fields_so_credentials_cannot_be_injected() {
        let r = parse(
            r#"{"central_url":"https://h.exemplo","state_file":"/x","projects":[],"openai_api_key":"sk-x"}"#,
        );
        assert!(r.is_err());
        let r = parse(
            r#"{"central_url":"https://h.exemplo","state_file":"/x","projects":[{"project_id":"p","root":"/y","claude_token":"t"}]}"#,
        );
        assert!(r.is_err());
    }

    #[test]
    fn plain_http_only_on_loopback() {
        let ok = r#"{"central_url":"http://127.0.0.1:8787","state_file":"/x","projects":[]}"#;
        assert!(parse(ok).is_ok());
        let bad = r#"{"central_url":"http://187.77.52.164:8787","state_file":"/x","projects":[]}"#;
        assert!(parse(bad).is_err());
    }

    #[test]
    fn node_without_projects_is_valid() {
        let none = r#"{"central_url":"https://h.exemplo","state_file":"/x","projects":[]}"#;
        assert!(parse(none).expect("zero projetos é válido").projects.is_empty());
    }

    #[test]
    fn local_project_list_is_refused_because_the_hub_owns_projects() {
        let with = r#"{"central_url":"https://h.exemplo","state_file":"/x","projects":[{"project_id":"p","root":"/y"}]}"#;
        assert!(parse(with).is_err());
    }
}
