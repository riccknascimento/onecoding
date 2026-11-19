//! Segredos do OneHub Central. Senhas: Argon2id (PHC). Tokens de sessão, de OneNode e códigos de
//! pareamento: 32 bytes aleatórios, entregues uma única vez, guardados só como SHA-256.

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use sha2::{Digest, Sha256};

/// Falha ao gerar sal ou hash (RNG indisponível ou parâmetros inválidos). Sem detalhes para o chamador.
#[derive(Debug)]
pub struct HashError;

pub fn hash_password(password: &str) -> Result<String, HashError> {
    let mut salt_bytes = [0u8; 16];
    getrandom::fill(&mut salt_bytes).map_err(|_| HashError)?;
    let salt = SaltString::encode_b64(&salt_bytes).map_err(|_| HashError)?;
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|_| HashError)
}

pub fn verify_password(password: &str, phc: &str) -> bool {
    PasswordHash::new(phc)
        .map(|parsed| {
            Argon2::default()
                .verify_password(password.as_bytes(), &parsed)
                .is_ok()
        })
        .unwrap_or(false)
}

/// Segredo aleatório de 256 bits, codificado em base64url sem preenchimento.
pub fn new_secret() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("gerador de números aleatórios do sistema indisponível");
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Hash usado como chave no banco. Segredos têm alta entropia, então SHA-256 basta (sem sal).
pub fn sha256(secret: &str) -> Vec<u8> {
    Sha256::digest(secret.as_bytes()).to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_hash_is_argon2id_and_verifies() {
        let phc = hash_password("senha-forte-123").unwrap();
        assert!(phc.starts_with("$argon2id$"), "{phc}");
        assert!(verify_password("senha-forte-123", &phc));
        assert!(!verify_password("errada", &phc));
        assert!(!verify_password("x", "nao-e-phc"));
    }

    #[test]
    fn secrets_are_unique_and_hash_is_stable_without_storing_plaintext() {
        let a = new_secret();
        let b = new_secret();
        assert_ne!(a, b);
        assert_eq!(a.len(), 43);
        assert_eq!(sha256(&a), sha256(&a));
        assert_ne!(sha256(&a), a.as_bytes());
    }
}
