//! Confinação de caminhos: nada fora da raiz do projeto é lido. Recusa caminhos absolutos, `..`, NUL e
//! barras invertidas; depois canonicaliza (resolve symlinks) e confere que o resultado continua dentro da
//! raiz. Symlink que aponta para fora é recusado mesmo que o nome esteja dentro.

use std::path::{Component, Path, PathBuf};

pub const OUTSIDE: &str = "caminho fora do workspace autorizado";

pub fn confine(root: &Path, rel: &str) -> Result<PathBuf, String> {
    if rel.contains('\0') || rel.contains('\\') {
        return Err(OUTSIDE.into());
    }
    let rel_path = Path::new(rel);
    for c in rel_path.components() {
        match c {
            Component::Normal(_) | Component::CurDir => {}
            _ => return Err(OUTSIDE.into()),
        }
    }
    let target = root.join(rel_path);
    let canon = target
        .canonicalize()
        .map_err(|_| "caminho inexistente".to_string())?;
    if !canon.starts_with(root) {
        return Err(OUTSIDE.into());
    }
    Ok(canon)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Contador único por fixture: testes paralelos podem sortear o mesmo instante, e dois diretórios iguais
    /// fariam um teste apagar o workspace do outro.
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    fn fixture() -> (tempdir::Guard, PathBuf, PathBuf) {
        let base = std::env::temp_dir().join(format!(
            "oa-confine-{}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst),
            rand_suffix()
        ));
        let root = base.join("ws");
        let outside = base.join("fora");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(root.join("sub/in.txt"), "dentro").unwrap();
        std::fs::write(outside.join("segredo.txt"), "fora").unwrap();
        let root = root.canonicalize().unwrap();
        (tempdir::Guard(base), root, outside.canonicalize().unwrap())
    }

    fn rand_suffix() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    }

    pub mod tempdir {
        pub struct Guard(pub std::path::PathBuf);
        impl Drop for Guard {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }

    #[test]
    fn inside_paths_are_allowed_including_root_and_nested() {
        let (_g, root, _) = fixture();
        assert_eq!(confine(&root, "").unwrap(), root);
        assert_eq!(confine(&root, ".").unwrap(), root);
        assert!(confine(&root, "sub/in.txt")
            .unwrap()
            .ends_with("sub/in.txt"));
    }

    #[test]
    fn traversal_absolute_and_backslash_are_refused() {
        let (_g, root, outside) = fixture();
        for bad in [
            "../fora/segredo.txt",
            "sub/../../fora/segredo.txt",
            "/etc/passwd",
            "..",
            "a\\..\\b",
        ] {
            assert_eq!(confine(&root, bad).unwrap_err(), OUTSIDE, "{bad}");
        }
        assert!(outside.join("segredo.txt").exists());
    }

    #[test]
    fn symlink_pointing_outside_is_refused_even_with_inside_name() {
        let (_g, root, outside) = fixture();
        std::os::unix::fs::symlink(outside.join("segredo.txt"), root.join("atalho.txt")).unwrap();
        assert_eq!(confine(&root, "atalho.txt").unwrap_err(), OUTSIDE);
        // symlink para dentro continua permitido
        std::os::unix::fs::symlink(root.join("sub/in.txt"), root.join("interno.txt")).unwrap();
        assert!(confine(&root, "interno.txt").is_ok());
    }

    #[test]
    fn missing_paths_are_reported_as_missing() {
        let (_g, root, _) = fixture();
        assert_eq!(
            confine(&root, "nao-existe.txt").unwrap_err(),
            "caminho inexistente"
        );
    }
}
