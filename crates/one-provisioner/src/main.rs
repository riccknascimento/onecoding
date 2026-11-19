//! Binário do provisionador. Configuração por variáveis de ambiente, sem segredos em argumentos:
//! ONEPROV_TOKEN_FILE (token da API local), ONEPROV_NODE_BINARY (binário do OneNode),
//! ONEPROV_NODE_BINARY_SHA256 (sha256 esperado do binário), ONEPROV_LISTEN (padrão 127.0.0.1:8790).

use std::net::SocketAddr;
use std::sync::Arc;

use one_provisioner::install::sha256_hex;
use one_provisioner::service::{self, AppConfig, Registry};

fn env_req(name: &str) -> Result<String, String> {
    std::env::var(name).map_err(|_| format!("defina {name}"))
}

#[tokio::main]
async fn main() -> Result<(), String> {
    let token_file = env_req("ONEPROV_TOKEN_FILE")?;
    let token = std::fs::read_to_string(&token_file)
        .map_err(|_| "não foi possível ler o token da API local".to_string())?
        .trim()
        .to_string();
    if token.len() < 32 {
        return Err("token da API local curto demais (mínimo 32 caracteres)".into());
    }
    let binary = std::path::PathBuf::from(env_req("ONEPROV_NODE_BINARY")?);
    let expected = env_req("ONEPROV_NODE_BINARY_SHA256")?;
    let bytes = std::fs::read(&binary).map_err(|_| "binário do OneNode não encontrado".to_string())?;
    if sha256_hex(&bytes) != expected {
        return Err("binário do OneNode não confere com o sha256 configurado".into());
    }
    let listen: SocketAddr = std::env::var("ONEPROV_LISTEN")
        .unwrap_or_else(|_| "127.0.0.1:8790".into())
        .parse()
        .map_err(|_| "ONEPROV_LISTEN inválido".to_string())?;
    let listen = service::loopback_only(listen)?;
    let state = Arc::new((
        AppConfig { token, binary, binary_sha256: expected },
        Registry::default(),
    ));
    let listener = tokio::net::TcpListener::bind(listen).await.map_err(|_| "não foi possível escutar".to_string())?;
    eprintln!("one-provisioner escutando em {listen} (somente loopback)");
    axum::serve(listener, service::router(state)).await.map_err(|_| "servidor encerrado".to_string())
}
