//! Catálogo fechado de ferramentas do OneNode. Só leitura. Saída com limite de tamanho. Nomes de arquivos
//! e conteúdos são devolvidos ao OneHub, mas nenhuma credencial conhecida é lida de propósito.

use std::path::Path;

use serde_json::{json, Value};

use crate::confine::confine;

pub const MAX_LIST_ENTRIES: usize = 500;
pub const MAX_READ_BYTES: usize = 16 * 1024;

/// Arquivos que parecem segredos nunca são lidos, mesmo dentro do workspace autorizado.
pub fn is_sensitive_name(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_lowercase();
    let in_git = path.components().any(|c| c.as_os_str() == ".git");
    in_git
        || name == ".env"
        || name.starts_with(".env.")
        || name.ends_with(".pem")
        || name.ends_with(".key")
        || name.starts_with("id_rsa")
        || name.starts_with("id_ed25519")
        || name.starts_with("id_ecdsa")
}

pub fn files_list(root: &Path, rel: &str) -> Result<Value, String> {
    let dir = confine(root, rel)?;
    if !dir.is_dir() {
        return Err("não é uma pasta".into());
    }
    let mut entries: Vec<(String, &'static str)> = Vec::new();
    let mut truncated = false;
    for entry in
        std::fs::read_dir(&dir).map_err(|_| "não foi possível listar a pasta".to_string())?
    {
        let Ok(entry) = entry else { continue };
        if entries.len() >= MAX_LIST_ENTRIES {
            truncated = true;
            break;
        }
        // file_type não segue symlinks: um link aparece como link, nunca como o alvo.
        let kind = match entry.file_type() {
            Ok(t) if t.is_symlink() => "symlink",
            Ok(t) if t.is_dir() => "dir",
            Ok(t) if t.is_file() => "file",
            _ => "other",
        };
        entries.push((entry.file_name().to_string_lossy().into_owned(), kind));
    }
    entries.sort();
    Ok(json!({
        "path": rel,
        "truncated": truncated,
        "entries": entries.iter().map(|(n, k)| json!({ "name": n, "kind": k })).collect::<Vec<_>>(),
    }))
}

pub fn files_read(root: &Path, rel: &str) -> Result<Value, String> {
    let file = confine(root, rel)?;
    if is_sensitive_name(&file) {
        return Err("arquivo sensível: leitura recusada".into());
    }
    if !file.is_file() {
        return Err("não é um arquivo regular".into());
    }
    let bytes = std::fs::read(&file).map_err(|_| "não foi possível ler o arquivo".to_string())?;
    let total = bytes.len();
    let shown = &bytes[..total.min(MAX_READ_BYTES)];
    let text = std::str::from_utf8(shown)
        .map_err(|_| "arquivo binário ou fora de UTF-8: leitura recusada".to_string())?;
    Ok(json!({
        "path": rel,
        "bytes": total,
        "truncated": total > MAX_READ_BYTES,
        "content": text,
    }))
}

/// Executa uma ferramenta do catálogo. Qualquer outro nome é recusado.
pub fn run(root: &Path, tool: &str, args: &Value) -> Result<Value, String> {
    let rel = args
        .get("path")
        .and_then(Value::as_str)
        .ok_or("argumento 'path' ausente")?;
    match tool {
        "files_list" => files_list(root, rel),
        "files_read" => files_read(root, rel),
        _ => Err("ferramenta não permitida neste OneNode".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sensitive_names_are_detected() {
        for p in [
            ".env",
            "config/.env.production",
            "chave.pem",
            "deploy.key",
            ".git/config",
            "id_ed25519",
        ] {
            assert!(is_sensitive_name(Path::new(p)), "{p}");
        }
        assert!(!is_sensitive_name(Path::new("src/main.rs")));
    }

    #[test]
    fn unknown_tool_is_refused_and_missing_path_is_refused() {
        let root = std::env::temp_dir();
        assert!(run(&root, "shell", &json!({"path": "."})).is_err());
        assert!(run(&root, "files_list", &json!({})).is_err());
    }
}
