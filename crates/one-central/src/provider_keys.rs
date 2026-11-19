//! Cifra e decifra API Keys de provedores de IA (AES-256-GCM).
//!
//! A chave-mestra vem de um arquivo fora do banco (`ONEAGENT_CENTRAL_PROVIDER_KEY_FILE`), com 32 bytes em base64.
//! O nome do provedor entra como dado associado (AAD): um texto cifrado não serve para outro provedor.
//! Nada aqui registra o valor da chave nem o texto claro.

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine;

pub const MASTER_KEY_LEN: usize = 32;
const NONCE_LEN: usize = 12;

/// Falha de cifragem. Sem detalhes: o motivo exato não pode vazar para respostas ou logs.
#[derive(Debug, PartialEq, Eq)]
pub struct CryptoError;

pub fn load_master_key(path: &str) -> Result<[u8; MASTER_KEY_LEN], String> {
    let raw = std::fs::read_to_string(path).map_err(|_| "arquivo da chave-mestra ilegível".to_string())?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(raw.trim())
        .map_err(|_| "chave-mestra não está em base64".to_string())?;
    <[u8; MASTER_KEY_LEN]>::try_from(bytes.as_slice())
        .map_err(|_| "chave-mestra deve ter 32 bytes".to_string())
}

/// Cifra a chave do provedor. Devolve (texto cifrado, nonce aleatório de 12 bytes).
pub fn seal(master: &[u8; MASTER_KEY_LEN], provider: &str, api_key: &str) -> Result<(Vec<u8>, Vec<u8>), CryptoError> {
    let mut nonce = [0u8; NONCE_LEN];
    getrandom::fill(&mut nonce).map_err(|_| CryptoError)?;
    let cipher = Aes256Gcm::new_from_slice(master).map_err(|_| CryptoError)?;
    let ct = cipher
        .encrypt(
            &Nonce::from(nonce),
            Payload { msg: api_key.as_bytes(), aad: provider.as_bytes() },
        )
        .map_err(|_| CryptoError)?;
    Ok((ct, nonce.to_vec()))
}

/// Decifra a chave do provedor. Falha fechada: qualquer divergência de chave ou AAD devolve erro.
pub fn open(master: &[u8; MASTER_KEY_LEN], provider: &str, ciphertext: &[u8], nonce: &[u8]) -> Result<String, CryptoError> {
    let nonce: [u8; NONCE_LEN] = nonce.try_into().map_err(|_| CryptoError)?;
    let cipher = Aes256Gcm::new_from_slice(master).map_err(|_| CryptoError)?;
    let pt = cipher
        .decrypt(
            &Nonce::from(nonce),
            Payload { msg: ciphertext, aad: provider.as_bytes() },
        )
        .map_err(|_| CryptoError)?;
    String::from_utf8(pt).map_err(|_| CryptoError)
}

/// Dica mostrada na interface: só os últimos 4 caracteres. Chaves curtas não revelam nada.
pub fn hint(api_key: &str) -> String {
    let chars: Vec<char> = api_key.chars().collect();
    if chars.len() < 12 {
        return "••••".to_string();
    }
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("••••{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; 32] = [7u8; 32];

    #[test]
    fn sealed_key_opens_only_with_the_same_master_and_provider() {
        let (ct, nonce) = seal(&KEY, "openrouter", "sk-or-v1-segredo-de-teste-1234").unwrap();
        assert_eq!(open(&KEY, "openrouter", &ct, &nonce).unwrap(), "sk-or-v1-segredo-de-teste-1234");
        // outro provedor (AAD diferente) não abre o mesmo texto cifrado
        assert!(open(&KEY, "openai", &ct, &nonce).is_err());
        // outra chave-mestra não abre
        assert!(open(&[9u8; 32], "openrouter", &ct, &nonce).is_err());
    }

    #[test]
    fn each_seal_uses_a_fresh_nonce_and_never_stores_plaintext() {
        let (ct1, n1) = seal(&KEY, "gemini", "AIzaSy-chave-de-teste-abcd").unwrap();
        let (ct2, n2) = seal(&KEY, "gemini", "AIzaSy-chave-de-teste-abcd").unwrap();
        assert_ne!(n1, n2);
        assert_ne!(ct1, ct2);
        assert!(!ct1.windows(6).any(|w| w == b"AIzaSy"));
    }

    #[test]
    fn tampered_ciphertext_fails_closed() {
        let (mut ct, nonce) = seal(&KEY, "anthropic", "sk-ant-chave-de-teste-wxyz").unwrap();
        ct[0] ^= 1;
        assert!(open(&KEY, "anthropic", &ct, &nonce).is_err());
    }

    #[test]
    fn hint_shows_only_the_last_four_characters() {
        assert_eq!(hint("sk-or-v1-abcdefghijWXYZ"), "••••WXYZ");
        assert_eq!(hint("curta"), "••••");
    }

    #[test]
    fn master_key_must_be_exactly_32_bytes() {
        let dir = std::env::temp_dir().join(format!("oneagent-mk-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let ok = dir.join("ok");
        let bad = dir.join("bad");
        std::fs::write(&ok, base64::engine::general_purpose::STANDARD.encode([3u8; 32])).unwrap();
        std::fs::write(&bad, base64::engine::general_purpose::STANDARD.encode([3u8; 16])).unwrap();
        assert!(load_master_key(ok.to_str().unwrap()).is_ok());
        assert!(load_master_key(bad.to_str().unwrap()).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
