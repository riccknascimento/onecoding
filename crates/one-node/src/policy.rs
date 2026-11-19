//! Política local do OneNode: quais pastas ESTE servidor aceita como raiz de projeto.
//! O arquivo fica no servidor (root, 0644), fora do alcance do OneHub. O OneHub pede raízes; o nó só usa
//! as que passam aqui. Cadastrar um projeto na Web nunca amplia esta política.

use std::path::{Component, Path, PathBuf};

use serde::Deserialize;

/// Prefixos de sistema: nunca são raiz de projeto, mesmo que a política os liste.
const DENIED_PREFIXES: &[&str] = &[
    "/", "/bin", "/boot", "/dev", "/etc", "/lib", "/lib64", "/proc", "/root", "/run", "/sbin", "/snap",
    "/sys", "/usr", "/var/backups", "/var/lib", "/var/log",
];

/// Nomes de pasta com credenciais ou metadados de Git: nunca entram numa raiz.
const DENIED_COMPONENTS: &[&str] = &[".ssh", ".gnupg", ".aws", ".kube", ".docker", ".git"];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    /// Pastas-pai autorizadas neste servidor. Cada raiz de projeto precisa estar dentro de uma delas.
    pub allowed_roots: Vec<PathBuf>,
}

/// Política já carregada e canonicalizada.
#[derive(Debug, Clone, Default)]
pub struct Loaded {
    pub roots: Vec<PathBuf>,
    pub digest: u64,
}

pub fn load(path: &Path) -> Result<Loaded, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        // sem arquivo, nada é aceito: a ausência de política é o estado seguro
        Err(_) => return Ok(Loaded::default()),
    };
    let policy: Policy = serde_json::from_str(&text).map_err(|_| "política local inválida".to_string())?;
    let mut roots = Vec::new();
    for p in &policy.allowed_roots {
        if !p.is_absolute() {
            return Err("política local: pastas autorizadas precisam ser absolutas".into());
        }
        let canon = std::fs::canonicalize(p)
            .map_err(|_| "política local: pasta autorizada não existe".to_string())?;
        if is_denied_prefix(&canon) {
            return Err("política local: pasta de sistema não pode ser autorizada".into());
        }
        roots.push(canon);
    }
    Ok(Loaded { roots, digest: fnv1a(text.as_bytes()) })
}

/// Valida uma raiz pedida pelo OneHub. Retorna o caminho canonicalizado ou um motivo curto.
/// Links simbólicos são resolvidos antes da comparação, então não há escape por link.
pub fn validate_root(policy: &Loaded, requested: &str) -> Result<PathBuf, &'static str> {
    if policy.roots.is_empty() {
        return Err("sem pasta autorizada na política local");
    }
    if !requested.starts_with('/') || requested.contains('\0') {
        return Err("raiz precisa ser um caminho absoluto");
    }
    let req = Path::new(requested);
    if req.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err("raiz não pode conter ..");
    }
    let canon = std::fs::canonicalize(req).map_err(|_| "pasta não existe")?;
    if !canon.is_dir() {
        return Err("não é uma pasta");
    }
    if is_denied_prefix(&canon) {
        return Err("pasta de sistema");
    }
    if canon.components().any(|c| matches!(c, Component::Normal(n) if DENIED_COMPONENTS.iter().any(|d| n == std::ffi::OsStr::new(d)))) {
        return Err("pasta sensível (credenciais ou Git)");
    }
    if !policy.roots.iter().any(|allowed| canon.starts_with(allowed)) {
        return Err("fora da política local");
    }
    Ok(canon)
}

fn is_denied_prefix(p: &Path) -> bool {
    DENIED_PREFIXES.iter().any(|d| {
        let d = Path::new(d);
        let canonical = d.canonicalize().unwrap_or_else(|_| d.to_path_buf());
        // "/" só bloqueia a própria raiz do sistema, não tudo abaixo dela
        if *d == *Path::new("/") {
            p == d || p == &canonical
        } else {
            p.starts_with(d) || p.starts_with(&canonical)
        }
    })
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("oa-policy-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::canonicalize(&d).unwrap()
    }

    fn policy_for(allowed: &Path) -> Loaded {
        Loaded { roots: vec![allowed.to_path_buf()], digest: 1 }
    }

    #[test]
    fn root_inside_allowed_folder_is_accepted() {
        let base = tmp("ok");
        let proj = base.join("projeto");
        std::fs::create_dir_all(&proj).unwrap();
        let got = validate_root(&policy_for(&base), proj.to_str().unwrap()).unwrap();
        assert_eq!(got, std::fs::canonicalize(&proj).unwrap());
    }

    #[test]
    fn root_outside_allowed_folder_is_refused() {
        let base = tmp("fora");
        let outro = tmp("outro-lado");
        let r = validate_root(&policy_for(&base), outro.to_str().unwrap());
        assert_eq!(r, Err("fora da política local"));
    }

    #[test]
    fn symlink_escaping_the_allowed_folder_is_refused() {
        let base = tmp("link");
        let outro = tmp("alvo-link");
        let link = base.join("atalho");
        std::os::unix::fs::symlink(&outro, &link).unwrap();
        let r = validate_root(&policy_for(&base), link.to_str().unwrap());
        assert_eq!(r, Err("fora da política local"));
    }

    #[test]
    fn parent_traversal_and_relative_paths_are_refused() {
        let base = tmp("trav");
        let p = policy_for(&base);
        assert!(validate_root(&p, &format!("{}/../x", base.display())).is_err());
        assert!(validate_root(&p, "relativo").is_err());
    }

    #[test]
    fn system_and_credential_folders_are_refused_even_when_listed() {
        assert!(is_denied_prefix(Path::new("/etc")));
        assert!(is_denied_prefix(Path::new("/root/x")));
        assert!(is_denied_prefix(Path::new("/")));
        assert!(!is_denied_prefix(Path::new("/opt/onenode")));
        let base = tmp("sens");
        let ssh = base.join(".ssh");
        std::fs::create_dir_all(&ssh).unwrap();
        let r = validate_root(&policy_for(&base), ssh.to_str().unwrap());
        assert_eq!(r, Err("pasta sensível (credenciais ou Git)"));
    }

    #[test]
    fn without_policy_nothing_is_accepted() {
        let base = tmp("vazia");
        assert_eq!(
            validate_root(&Loaded::default(), base.to_str().unwrap()),
            Err("sem pasta autorizada na política local")
        );
    }

    #[test]
    fn policy_refuses_system_roots_and_missing_file_is_empty() {
        let dir = tmp("pol");
        let file = dir.join("policy.json");
        std::fs::write(&file, r#"{"allowed_roots":["/etc"]}"#).unwrap();
        assert!(load(&file).is_err());
        let none = load(&dir.join("inexistente.json")).unwrap();
        assert!(none.roots.is_empty());
    }
}
