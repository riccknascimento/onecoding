//! OneHub Central (Etapa B). Monólito modular: API HTTP autenticada, PostgreSQL como fonte de verdade,
//! registro de OneNodes com identidade própria e tarefas com lease. Ver `routes.rs` para as rotas.

pub mod auth;
pub mod bootstrap;
pub mod cli_auth;
pub mod db;
pub mod events;
pub mod executor;
pub mod mention;
pub mod orchestrator;
pub mod provider_http;
pub mod provider_keys;
pub mod routes;

pub use routes::{router, AppState};

/// Verdadeiro para endereços que só aceitam conexão local.
pub fn is_loopback_bind(addr: &str) -> bool {
    match addr.parse::<std::net::SocketAddr>() {
        Ok(sock) => sock.ip().is_loopback(),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::is_loopback_bind;

    #[test]
    fn only_loopback_addresses_are_accepted_by_default() {
        assert!(is_loopback_bind("127.0.0.1:8787"));
        assert!(is_loopback_bind("[::1]:8787"));
        assert!(!is_loopback_bind("0.0.0.0:8787"));
        assert!(!is_loopback_bind("187.77.52.164:443"));
        assert!(!is_loopback_bind("nao-e-endereco"));
    }
}
