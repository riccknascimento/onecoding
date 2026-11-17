//! Confiança em executáveis: descobrir ≠ executar.
//!
//! `inspect` só LÊ (metadados e conteúdo); nada é executado. Uma CLI só vira `TrustedExe`, a única
//! coisa que `Host::run` aceita, se a identidade atual for igual à que o usuário aprovou.
//!
//! Identidade = caminho canônico (symlinks resolvidos) + SHA-256 do conteúdo + dono + interpretador
//! do script (quando há `#!`). Hash ou permissões sozinhos não provam legitimidade: a base é a
//! aprovação humana; o resto detecta que "o que foi aprovado" mudou.
//!
//! Limite conhecido: o hash cobre o arquivo lançado, não o pacote inteiro (ex.: módulos do npm) nem
//! o conteúdo do interpretador; e há uma janela entre a verificação e o `exec` (sem `fexecve` no macOS).

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use one_protocol::{EngineKind, TrustState};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub canonical: PathBuf,
    pub interpreter: Option<PathBuf>,
    pub sha256: String,
    pub owner_uid: u32,
    /// Resumo exibido ao usuário e persistido: muda se qualquer parte da identidade mudar.
    pub fingerprint: String,
}

/// Resultado da inspeção (sem execução).
#[derive(Debug, Clone)]
pub struct Inspected {
    /// Caminho como foi encontrado (pode ser um symlink).
    pub found: PathBuf,
    /// `Err(motivo)` = reprovado nas verificações; não pode ser aprovado.
    pub identity: Result<Identity, String>,
}

/// Prova de que o executável passou na verificação. Só este módulo cria; `Host::run` exige.
#[derive(Debug, Clone)]
pub struct TrustedExe {
    path: PathBuf,
}

impl TrustedExe {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self { path }
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Persistência das aprovações (implementada sobre `one-store` pelo app).
pub trait TrustStore: Sync {
    fn approved_fingerprint(&self, engine: EngineKind) -> Option<String>;
    fn approve(
        &self,
        engine: EngineKind,
        fingerprint: &str,
        canonical_path: &str,
    ) -> Result<(), String>;
}

/// Estado de confiança de uma inspeção frente ao que foi aprovado.
pub fn evaluate(insp: &Inspected, store: &dyn TrustStore, engine: EngineKind) -> TrustState {
    match &insp.identity {
        Err(_) => TrustState::Rejected,
        Ok(id) => match store.approved_fingerprint(engine) {
            None => TrustState::Unknown,
            Some(fp) if fp == id.fingerprint => TrustState::Trusted,
            Some(_) => TrustState::Changed,
        },
    }
}

/// Inspeciona `found` sem executar nada. `search_dirs` resolve o interpretador de `#!/usr/bin/env x`.
#[cfg(unix)]
pub fn inspect(found: &Path, search_dirs: &[PathBuf]) -> Inspected {
    Inspected {
        found: found.to_path_buf(),
        identity: inspect_inner(found, search_dirs),
    }
}

#[cfg(not(unix))]
pub fn inspect(found: &Path, _search_dirs: &[PathBuf]) -> Inspected {
    Inspected {
        found: found.to_path_buf(),
        identity: Err(
            "verificação de dono e permissões ainda não implementada nesta plataforma".into(),
        ),
    }
}

#[cfg(unix)]
fn inspect_inner(found: &Path, search_dirs: &[PathBuf]) -> Result<Identity, String> {
    use std::os::unix::fs::MetadataExt;

    // SAFETY: geteuid não tem pré-condições.
    let me = unsafe { libc::geteuid() };
    let canonical = fs::canonicalize(found).map_err(|e| format!("caminho inválido: {e}"))?;
    check_file(&canonical, me)?;
    // O diretório onde o link/arquivo foi achado também conta: quem escreve nele troca o alvo.
    if let Some(dir) = found.parent().and_then(|d| fs::canonicalize(d).ok()) {
        check_dir_chain(&dir, me)?;
    }
    check_dir_chain(canonical.parent().ok_or("sem diretório pai")?, me)?;

    let interpreter = match shebang_interpreter(&canonical, search_dirs)? {
        Some(p) => {
            let c = fs::canonicalize(&p).map_err(|e| format!("interpretador inválido: {e}"))?;
            check_file(&c, me)?;
            check_dir_chain(c.parent().ok_or("sem diretório pai")?, me)?;
            Some(c)
        }
        None => None,
    };

    let sha256 = hash_file(&canonical)?;
    let owner_uid = fs::metadata(&canonical).map_err(|e| e.to_string())?.uid();
    let mut h = Sha256::new();
    h.update(canonical.as_os_str().as_encoded_bytes());
    h.update([0]);
    h.update(sha256.as_bytes());
    h.update([0]);
    h.update(owner_uid.to_string());
    h.update([0]);
    if let Some(i) = &interpreter {
        h.update(i.as_os_str().as_encoded_bytes());
    }
    Ok(Identity {
        canonical,
        interpreter,
        sha256,
        owner_uid,
        fingerprint: hex(&h.finalize()),
    })
}

/// Arquivo regular, executável, do usuário atual ou de root, sem escrita para grupo nem outros.
#[cfg(unix)]
fn check_file(path: &Path, me: u32) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;
    let m = fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if !m.is_file() {
        return Err(format!("{} não é arquivo regular", path.display()));
    }
    if m.mode() & 0o111 == 0 {
        return Err(format!("{} não é executável", path.display()));
    }
    if m.uid() != me && m.uid() != 0 {
        return Err(format!(
            "{} pertence a outro usuário (uid {})",
            path.display(),
            m.uid()
        ));
    }
    if m.mode() & 0o022 != 0 {
        return Err(format!(
            "{} é gravável por grupo ou outros usuários",
            path.display()
        ));
    }
    no_write_acl(path)
}

/// Cada diretório do caminho até a raiz: dono = usuário atual ou root; sem escrita para grupo nem
/// outros (exceto com sticky bit, como /tmp, onde ninguém renomeia entradas alheias).
#[cfg(unix)]
fn check_dir_chain(dir: &Path, me: u32) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;
    for d in dir.ancestors() {
        let m = fs::metadata(d).map_err(|e| format!("{}: {e}", d.display()))?;
        if m.uid() != me && m.uid() != 0 {
            return Err(format!(
                "diretório {} pertence a outro usuário (uid {})",
                d.display(),
                m.uid()
            ));
        }
        if m.mode() & 0o022 != 0 && m.mode() & 0o1000 == 0 {
            return Err(format!(
                "diretório {} é gravável por grupo ou outros usuários",
                d.display()
            ));
        }
        no_write_acl(d)?;
    }
    Ok(())
}

/// Reprova quem tem ACL concedendo escrita (os bits de modo sozinhos não bastam) ou ACL ilegível.
#[cfg(unix)]
fn no_write_acl(path: &Path) -> Result<(), String> {
    match crate::acl::allows_write(path) {
        Ok(false) => Ok(()),
        Ok(true) => Err(format!("{} tem ACL que concede escrita", path.display())),
        Err(e) => Err(e),
    }
}

/// Interpretador declarado na primeira linha `#!`; `env x` resolve `x` nos diretórios de busca.
#[cfg(unix)]
fn shebang_interpreter(path: &Path, search_dirs: &[PathBuf]) -> Result<Option<PathBuf>, String> {
    let mut head = [0u8; 256];
    let n = fs::File::open(path)
        .and_then(|mut f| f.read(&mut head))
        .map_err(|e| e.to_string())?;
    if n < 2 || &head[..2] != b"#!" {
        return Ok(None); // binário nativo
    }
    let line = String::from_utf8_lossy(&head[2..n]);
    let line = line.lines().next().unwrap_or("");
    let mut parts = line.split_whitespace();
    let prog = parts.next().ok_or("shebang vazio")?;
    let target = if Path::new(prog).file_name().is_some_and(|n| n == "env") {
        parts
            .find(|t| !t.starts_with('-'))
            .ok_or("shebang env sem programa")?
    } else {
        prog
    };
    if target.contains('/') {
        return Ok(Some(PathBuf::from(target)));
    }
    search_dirs
        .iter()
        .map(|d| d.join(target))
        .find(|p| p.is_file())
        .map(Some)
        .ok_or_else(|| format!("interpretador `{target}` não encontrado"))
}

fn hash_file(path: &Path) -> Result<String, String> {
    let mut f = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex(&h.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(all(test, unix))]
pub(crate) mod tests {
    use std::collections::HashMap;
    use std::os::unix::fs::{symlink, PermissionsExt};
    use std::sync::Mutex;

    use super::*;

    /// Diretório temporário do teste: 0700, do usuário atual.
    pub fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("oneagent-trust-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        fs::set_permissions(&d, fs::Permissions::from_mode(0o700)).unwrap();
        fs::canonicalize(&d).unwrap()
    }

    pub fn script(path: &Path, body: &str, mode: u32) {
        fs::write(path, body).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
    }

    #[derive(Default)]
    pub struct MemStore(pub Mutex<HashMap<String, String>>);
    impl TrustStore for MemStore {
        fn approved_fingerprint(&self, e: EngineKind) -> Option<String> {
            self.0.lock().unwrap().get(&format!("{e:?}")).cloned()
        }
        fn approve(&self, e: EngineKind, fp: &str, _p: &str) -> Result<(), String> {
            self.0.lock().unwrap().insert(format!("{e:?}"), fp.into());
            Ok(())
        }
    }

    fn id(p: &Path) -> Identity {
        inspect(p, &[PathBuf::from("/bin")])
            .identity
            .expect("deve passar")
    }

    #[test]
    fn identity_changes_when_content_changes() {
        let d = tmp("content");
        let f = d.join("tool");
        script(&f, "#!/bin/sh\necho a\n", 0o755);
        let a = id(&f);
        script(&f, "#!/bin/sh\necho b\n", 0o755);
        let b = id(&f);
        assert_ne!(a.fingerprint, b.fingerprint);
        assert_eq!(a.canonical, b.canonical);
    }

    #[test]
    fn symlink_resolves_to_canonical_and_retarget_changes_identity() {
        let d = tmp("symlink");
        let real1 = d.join("real1");
        let real2 = d.join("real2");
        script(&real1, "#!/bin/sh\necho 1\n", 0o755);
        script(&real2, "#!/bin/sh\necho 1\n", 0o755); // mesmo conteúdo, outro caminho
        let link = d.join("codex");
        symlink(&real1, &link).unwrap();
        let a = id(&link);
        assert_eq!(a.canonical, real1);
        fs::remove_file(&link).unwrap();
        symlink(&real2, &link).unwrap();
        let b = id(&link);
        assert_eq!(b.canonical, real2);
        assert_ne!(
            a.fingerprint, b.fingerprint,
            "hash igual não basta: caminho canônico também conta"
        );
    }

    #[test]
    fn rejects_world_writable_file_and_dir_and_non_executable() {
        let d = tmp("perms");
        let f = d.join("tool");
        script(&f, "#!/bin/sh\n", 0o757);
        assert!(inspect(&f, &[PathBuf::from("/bin")])
            .identity
            .unwrap_err()
            .contains("gravável"));
        script(&f, "#!/bin/sh\n", 0o644);
        assert!(inspect(&f, &[PathBuf::from("/bin")])
            .identity
            .unwrap_err()
            .contains("executável"));
        script(&f, "#!/bin/sh\n", 0o755);
        fs::set_permissions(&d, fs::Permissions::from_mode(0o777)).unwrap();
        assert!(inspect(&f, &[PathBuf::from("/bin")])
            .identity
            .unwrap_err()
            .contains("diretório"));
        fs::set_permissions(&d, fs::Permissions::from_mode(0o770)).unwrap();
        assert!(inspect(&f, &[PathBuf::from("/bin")])
            .identity
            .unwrap_err()
            .contains("grupo"));
        fs::set_permissions(&d, fs::Permissions::from_mode(0o700)).unwrap();
        script(&f, "#!/bin/sh\n", 0o775); // gravável pelo grupo
        assert!(inspect(&f, &[PathBuf::from("/bin")])
            .identity
            .unwrap_err()
            .contains("grupo"));
        script(&f, "#!/bin/sh\n", 0o755);
        assert!(inspect(&f, &[PathBuf::from("/bin")]).identity.is_ok());
    }

    #[test]
    fn records_env_interpreter_and_rejects_missing_one() {
        let d = tmp("interp");
        let node = d.join("node");
        script(&node, "#!/bin/sh\n", 0o755);
        let f = d.join("cli.js");
        script(&f, "#!/usr/bin/env node\nconsole.log(1)\n", 0o755);
        let ok = inspect(&f, std::slice::from_ref(&d)).identity.unwrap();
        assert_eq!(ok.interpreter, Some(node.clone()));
        // trocar o interpretador muda a identidade
        let d2 = tmp("interp2");
        let node2 = d2.join("node");
        script(&node2, "#!/bin/sh\n", 0o755);
        let other = inspect(&f, &[d2]).identity.unwrap();
        assert_ne!(ok.fingerprint, other.fingerprint);
        let err = inspect(&f, &[PathBuf::from("/nao/existe")])
            .identity
            .unwrap_err();
        assert!(err.contains("não encontrado"), "{err}");
    }

    #[test]
    fn evaluate_unknown_trusted_changed_rejected() {
        let d = tmp("eval");
        let f = d.join("tool");
        script(&f, "#!/bin/sh\necho a\n", 0o755);
        let store = MemStore::default();
        let e = EngineKind::Codex;
        let i1 = inspect(&f, &[PathBuf::from("/bin")]);
        assert_eq!(evaluate(&i1, &store, e), TrustState::Unknown);
        let fp = i1.identity.as_ref().unwrap().fingerprint.clone();
        store.approve(e, &fp, "x").unwrap();
        assert_eq!(evaluate(&i1, &store, e), TrustState::Trusted);
        script(&f, "#!/bin/sh\necho SUBSTITUIDO\n", 0o755);
        assert_eq!(
            evaluate(&inspect(&f, &[PathBuf::from("/bin")]), &store, e),
            TrustState::Changed
        );
        script(&f, "#!/bin/sh\n", 0o777);
        assert_eq!(
            evaluate(&inspect(&f, &[PathBuf::from("/bin")]), &store, e),
            TrustState::Rejected
        );
    }

    /// ACLs reais do macOS aplicadas com `chmod +a`. Provam que um executável `0755` com ACL
    /// permissiva não é aprovável, e que ACLs inofensivas não quebram instalações legítimas.
    #[cfg(target_os = "macos")]
    mod acl {
        use super::*;

        fn chmod(args: &[&str], path: &Path) {
            let st = std::process::Command::new("/bin/chmod")
                .args(args)
                .arg(path)
                .status()
                .unwrap();
            assert!(st.success(), "chmod {args:?} falhou");
        }

        fn verdict(f: &Path) -> Result<Identity, String> {
            inspect(f, &[PathBuf::from("/bin")]).identity
        }

        fn fresh(name: &str) -> (PathBuf, PathBuf) {
            let d = tmp(name);
            let f = d.join("tool");
            script(&f, "#!/bin/sh\n", 0o755); // modo "seguro": só a ACL pode abrir a escrita
            (d, f)
        }

        #[test]
        fn file_with_write_like_allow_acl_is_rejected() {
            for perm in [
                "write",
                "append",
                "delete",
                "writeattr",
                "writeextattr",
                "writesecurity",
                "chown",
            ] {
                let (_d, f) = fresh(&format!("aclf-{perm}"));
                assert!(verdict(&f).is_ok(), "base deve passar");
                chmod(&["+a", &format!("everyone allow {perm}")], &f);
                let err = verdict(&f).unwrap_err();
                assert!(err.contains("ACL"), "{perm}: {err}");
                chmod(&["-N"], &f);
                assert!(verdict(&f).is_ok(), "sem ACL volta a passar ({perm})");
            }
        }

        /// Várias entradas: o laço precisa passar da primeira (deny) e achar o allow que concede escrita.
        #[test]
        fn permissive_entry_after_harmless_ones_is_still_found() {
            let (_d, f) = fresh("aclmulti");
            chmod(&["+a", "everyone allow read"], &f);
            chmod(&["+a", "everyone deny delete"], &f);
            chmod(&["+a", "everyone allow execute"], &f);
            assert!(verdict(&f).is_ok(), "entradas inofensivas passam");
            chmod(&["+a", "everyone allow write"], &f); // última entrada da lista
            assert!(verdict(&f).unwrap_err().contains("ACL"));
        }

        #[test]
        fn acl_on_a_specific_user_also_rejects() {
            let (_d, f) = fresh("acluser");
            chmod(&["+a", "user:nobody allow write"], &f);
            assert!(verdict(&f).unwrap_err().contains("ACL"));
        }

        #[test]
        fn directory_acl_that_lets_others_add_or_delete_is_rejected() {
            for perm in ["add_file", "delete_child", "add_subdirectory"] {
                let (d, f) = fresh(&format!("acld-{perm}"));
                chmod(&["+a", &format!("everyone allow {perm}")], &d);
                let err = verdict(&f).unwrap_err();
                assert!(err.contains("ACL"), "{perm}: {err}");
                chmod(&["-N"], &d);
                assert!(verdict(&f).is_ok());
            }
        }

        #[test]
        fn harmless_acls_do_not_break_legitimate_installs() {
            let (d, f) = fresh("aclok");
            // como o diretório pessoal do macOS: só nega exclusão
            chmod(&["+a", "everyone deny delete"], &d);
            // permissões de leitura/execução para terceiros não permitem trocar o arquivo
            chmod(&["+a", "everyone allow read,execute"], &f);
            assert!(verdict(&f).is_ok());
        }

        #[test]
        fn unsafe_acl_executable_cannot_be_approved_or_executed() {
            use crate::{approve_engine, detect_engine, ApproveError, SystemHost};
            let (d, bin) = fresh("acl-e2e");
            let marker = d.join("MARKER");
            let bin = bin.with_file_name("codex");
            script(
                &bin,
                &format!(
                    "#!/bin/sh\ntouch '{}'\necho 'codex-cli 1.0.0'\n",
                    marker.display()
                ),
                0o755,
            );
            let host = SystemHost::with_dirs(vec![
                d.clone(),
                PathBuf::from("/bin"),
                PathBuf::from("/usr/bin"),
            ]);
            let store = MemStore::default();
            chmod(&["+a", "everyone allow write"], &bin);

            let st = detect_engine(EngineKind::Codex, &host, &store, false);
            assert_eq!(st.trust, TrustState::Rejected);
            assert!(
                st.fingerprint.is_none(),
                "reprovado não oferece impressão digital para aprovar"
            );
            assert!(matches!(
                approve_engine(EngineKind::Codex, &host, &store, "qualquer"),
                Err(ApproveError::Rejected(_))
            ));
            assert!(store.approved_fingerprint(EngineKind::Codex).is_none());
            assert!(!marker.exists(), "executável reprovado não pode rodar");

            // aprovado antes e depois ganhando uma ACL permissiva: perde a confiança e não executa
            chmod(&["-N"], &bin);
            let fp = detect_engine(EngineKind::Codex, &host, &store, false)
                .fingerprint
                .unwrap();
            approve_engine(EngineKind::Codex, &host, &store, &fp).unwrap();
            assert_eq!(
                detect_engine(EngineKind::Codex, &host, &store, false).trust,
                TrustState::Trusted
            );
            std::fs::remove_file(&marker).unwrap();
            chmod(&["+a", "everyone allow write,delete"], &bin);
            let st = detect_engine(EngineKind::Codex, &host, &store, false);
            assert_eq!(st.trust, TrustState::Rejected);
            assert!(!marker.exists());
        }
    }
}
