//! Camada SSH do provisionador. Regra central: a fingerprint do servidor é conferida DURANTE o handshake,
//! antes de qualquer credencial ser enviada. Fingerprint diferente da confirmada aborta a conexão.
//! Credenciais ficam só na memória desta operação e são apagadas ao sair (Drop).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use russh::client;
use zeroize::Zeroize;
use russh::keys::{HashAlg, PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use russh::ChannelMsg;

use crate::guard::{resolve_checked, AccessType, GuardError};

pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
pub const EXEC_TIMEOUT: Duration = Duration::from_secs(300);

/// Segredo que nunca aparece em Debug, log ou serialização de volta. Zera a memória ao sair.
pub struct Secret(String);

impl Secret {
    pub fn new(s: String) -> Self {
        Secret(s)
    }
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<redigido>")
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        // sobrescreve o conteúdo antes de liberar a memória
        self.0.zeroize();
    }
}

#[derive(Debug)]
pub enum Auth {
    Password(Secret),
    Key { pem: Secret, passphrase: Option<Secret> },
}

#[derive(Debug, thiserror::Error)]
pub enum SshError {
    #[error(transparent)]
    Guard(#[from] GuardError),
    #[error("não foi possível conectar ao SSH do servidor")]
    Unreachable,
    #[error("tempo esgotado ao conectar ao SSH do servidor")]
    Timeout,
    #[error("a identidade do servidor mudou desde a confirmação: conexão recusada")]
    FingerprintMismatch,
    #[error("chave de host não identificada")]
    NoHostKey,
    #[error("autenticação recusada pelo servidor")]
    AuthRejected,
    #[error("chave privada inválida ou protegida por senha incorreta")]
    BadKey,
    #[error("falha no canal SSH")]
    Channel,
}

/// Handler que confere a fingerprint com a confirmada pelo administrador.
struct VerifyHandler {
    expected: String,
    seen: Arc<Mutex<Option<String>>>,
}

impl client::Handler for VerifyHandler {
    type Error = russh::Error;

    async fn check_server_key(&mut self, key: &PublicKeyOrCertificate) -> Result<bool, Self::Error> {
        let PublicKeyOrCertificate::PublicKey { key, .. } = key else {
            return Ok(false);
        };
        let fp = key.fingerprint(HashAlg::Sha256).to_string();
        *self.seen.lock().expect("lock de fingerprint") = Some(fp.clone());
        // Igual à confirmada: segue para a autenticação. Diferente: a conexão termina aqui.
        Ok(fp == self.expected)
    }
}

pub struct Session {
    handle: client::Handle<VerifyHandler>,
}

/// Conecta, valida a fingerprint no handshake e retorna a sessão ainda não autenticada.
pub async fn connect_verified(host: &str, port: u16, access: AccessType, expected_fp: &str) -> Result<Session, SshError> {
    let addrs = resolve_checked(host, port, access).await?;
    let config = Arc::new(client::Config {
        inactivity_timeout: Some(EXEC_TIMEOUT),
        ..Default::default()
    });
    let seen = Arc::new(Mutex::new(None));
    let mut last = SshError::Unreachable;
    for addr in addrs {
        let handler = VerifyHandler { expected: expected_fp.to_string(), seen: seen.clone() };
        match tokio::time::timeout(CONNECT_TIMEOUT, client::connect(config.clone(), addr, handler)).await {
            Err(_) => last = SshError::Timeout,
            Ok(Ok(handle)) => return Ok(Session { handle }),
            Ok(Err(_)) => {
                let captured = seen.lock().expect("lock de fingerprint").clone();
                last = match captured {
                    Some(fp) if fp != expected_fp => SshError::FingerprintMismatch,
                    Some(_) => SshError::Unreachable,
                    None => SshError::NoHostKey,
                };
            }
        }
    }
    Err(last)
}

impl Session {
    /// Autentica com senha ou chave. Chamada só depois de `connect_verified` ter validado a identidade.
    pub async fn authenticate(&mut self, user: &str, auth: &Auth) -> Result<(), SshError> {
        let result = match auth {
            Auth::Password(pw) => self
                .handle
                .authenticate_password(user.to_string(), pw.expose().to_string())
                .await
                .map_err(|_| SshError::AuthRejected)?,
            Auth::Key { pem, passphrase } => {
                let key = russh::keys::decode_secret_key(pem.expose(), passphrase.as_ref().map(|p| p.expose()))
                    .map_err(|_| SshError::BadKey)?;
                let hash = self.handle.best_supported_rsa_hash().await.ok().flatten().flatten();
                self.handle
                    .authenticate_publickey(user.to_string(), PrivateKeyWithHashAlg::new(Arc::new(key), hash))
                    .await
                    .map_err(|_| SshError::AuthRejected)?
            }
        };
        if matches!(result, russh::client::AuthResult::Success) {
            Ok(())
        } else {
            Err(SshError::AuthRejected)
        }
    }

    /// Executa um comando remoto, enviando `stdin` (se houver) e coletando a saída.
    pub async fn exec(&mut self, command: &str, stdin: Option<&[u8]>) -> Result<ExecOut, SshError> {
        let mut channel = self.handle.channel_open_session().await.map_err(|_| SshError::Channel)?;
        channel.exec(true, command).await.map_err(|_| SshError::Channel)?;
        if let Some(data) = stdin {
            channel.data(data).await.map_err(|_| SshError::Channel)?;
        }
        channel.eof().await.map_err(|_| SshError::Channel)?;
        let mut out = ExecOut::default();
        loop {
            match channel.wait().await {
                Some(ChannelMsg::Data { data }) => out.stdout.extend_from_slice(&data),
                Some(ChannelMsg::ExtendedData { data, .. }) => out.stderr.extend_from_slice(&data),
                Some(ChannelMsg::ExitStatus { exit_status }) => out.code = Some(exit_status),
                Some(ChannelMsg::Close) | None => break,
                _ => {}
            }
        }
        Ok(out)
    }
}

#[derive(Debug, Default)]
pub struct ExecOut {
    pub code: Option<u32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl ExecOut {
    pub fn ok(&self) -> bool {
        self.code == Some(0)
    }
    pub fn stdout_text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).trim().to_string()
    }
}
