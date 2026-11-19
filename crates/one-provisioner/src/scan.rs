//! Fase 1 do provisionamento: lê a fingerprint da chave de host do destino SEM autenticar.
//! O callback captura a chave e recusa a sessão, então nenhuma credencial chega ao servidor.
//! A fingerprint só vira confiança depois que o administrador a confirma na Web (fase 2 compara).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use russh::client;
use russh::keys::{HashAlg, PublicKeyOrCertificate};

use crate::guard::{resolve_checked, AccessType, GuardError};

pub const SCAN_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    #[error(transparent)]
    Guard(#[from] GuardError),
    #[error("não foi possível conectar ao SSH do destino")]
    Unreachable,
    #[error("tempo esgotado ao conectar ao SSH do destino")]
    Timeout,
    #[error("o destino não apresentou chave de host")]
    NoHostKey,
}

#[derive(Clone, Default)]
struct KeyCapture(Arc<Mutex<Option<String>>>);

struct ScanHandler {
    captured: KeyCapture,
}

impl client::Handler for ScanHandler {
    type Error = russh::Error;

    async fn check_server_key(&mut self, key: &PublicKeyOrCertificate) -> Result<bool, Self::Error> {
        // Certificados de host não são aceitos na fase 1: só chave pública simples gera fingerprint.
        if let PublicKeyOrCertificate::PublicKey { key, .. } = key {
            let fp = key.fingerprint(HashAlg::Sha256).to_string();
            *self.captured.0.lock().expect("lock de fingerprint") = Some(fp);
        }
        // Recusa: a conexão termina aqui, antes de qualquer autenticação.
        Ok(false)
    }
}

/// Lê a fingerprint SHA-256 do host informado. Não autentica e não envia credenciais.
pub async fn scan_fingerprint(host: &str, port: u16, access: AccessType) -> Result<String, ScanError> {
    let addrs = resolve_checked(host, port, access).await?;
    let config = Arc::new(client::Config {
        inactivity_timeout: Some(SCAN_TIMEOUT),
        ..Default::default()
    });
    let captured = KeyCapture::default();
    let mut last_err = ScanError::Unreachable;
    for addr in addrs {
        let handler = ScanHandler { captured: captured.clone() };
        if tokio::time::timeout(SCAN_TIMEOUT, client::connect(config.clone(), addr, handler)).await.is_err() {
            last_err = ScanError::Timeout;
        }
        if let Some(fp) = captured.0.lock().expect("lock de fingerprint").clone() {
            return Ok(fp);
        }
    }
    if captured.0.lock().expect("lock de fingerprint").is_none() {
        return Err(last_err);
    }
    Err(ScanError::NoHostKey)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn loopback_target_is_refused_before_any_connection() {
        let r = scan_fingerprint("127.0.0.1", 22, AccessType::Direct).await;
        assert!(matches!(r, Err(ScanError::Guard(_))));
    }

    #[tokio::test]
    async fn tailnet_name_is_refused_for_direct_access() {
        let r = scan_fingerprint("servidor.taile8e329.ts.net", 22, AccessType::Direct).await;
        assert!(matches!(r, Err(ScanError::Guard(_))));
    }
}
