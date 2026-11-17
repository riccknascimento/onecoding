//! Política de workspaces autorizáveis. Quem decide o caminho é o usuário (diálogo nativo), nunca a
//! webview; aqui só se recusa o que não deve ser autorizado e se canonicaliza.

use std::path::{Path, PathBuf};

/// Diretórios do sistema que nunca viram workspace.
const FORBIDDEN_PREFIXES: [&str; 9] = [
    "/System",
    "/Library",
    "/usr",
    "/bin",
    "/sbin",
    "/etc",
    "/private/etc",
    "/dev",
    "/Applications",
];

/// Canonicaliza e valida. `home` é injetado para teste.
pub fn validate(path: &Path, home: Option<&Path>) -> Result<PathBuf, String> {
    let c = std::fs::canonicalize(path).map_err(|e| format!("caminho inválido: {e}"))?;
    if !c.is_dir() {
        return Err("o workspace precisa ser uma pasta".into());
    }
    if c.parent().is_none() {
        return Err("a raiz do disco não pode ser um workspace".into());
    }
    if FORBIDDEN_PREFIXES.iter().any(|p| c.starts_with(p)) {
        return Err("pasta do sistema não pode ser um workspace".into());
    }
    if let Some(h) = home.and_then(|h| std::fs::canonicalize(h).ok()) {
        // a própria pasta pessoal (ou qualquer ancestral dela) é ampla demais
        if h.starts_with(&c) {
            return Err("escolha uma pasta de projeto, não a pasta pessoal inteira".into());
        }
    }
    Ok(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_a_project_dir_and_canonicalizes_symlinks() {
        let d = std::env::temp_dir().join(format!("oa-ws-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("real")).unwrap();
        std::os::unix::fs::symlink(d.join("real"), d.join("link")).unwrap();
        let ok = validate(&d.join("link"), Some(Path::new("/Users/nao-existe"))).unwrap();
        assert_eq!(ok, std::fs::canonicalize(d.join("real")).unwrap());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn rejects_root_system_home_files_and_missing() {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap();
        for bad in ["/", "/System", "/usr/bin", "/etc", "/Library"] {
            assert!(validate(Path::new(bad), Some(&home)).is_err(), "{bad}");
        }
        assert!(
            validate(&home, Some(&home)).is_err(),
            "pasta pessoal inteira"
        );
        assert!(
            validate(home.parent().unwrap(), Some(&home)).is_err(),
            "ancestral da pasta pessoal"
        );
        assert!(
            validate(Path::new("/etc/hosts"), Some(&home)).is_err(),
            "arquivo"
        );
        assert!(validate(Path::new("/nao/existe/xyz"), Some(&home)).is_err());
    }
}
