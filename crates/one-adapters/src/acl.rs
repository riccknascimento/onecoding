//! ACLs estendidas: os bits de modo (`0o755`) não mostram tudo. No macOS uma ACL pode conceder
//! escrita a "everyone" num arquivo `0755`, e então qualquer usuário trocaria o executável.
//!
//! Regra conservadora: qualquer entrada ALLOW com permissão de escrita/exclusão/mudança de
//! dono ou de permissões reprova o arquivo ou diretório, seja quem for o beneficiário (não
//! resolvemos o UUID para usuário). Entradas DENY não concedem nada e são ignoradas (o diretório
//! pessoal no macOS traz `group:everyone deny delete`). Se a ACL não puder ser lida, também reprova.

use std::path::Path;

/// `Ok(true)` = existe ACL que concede escrita; `Err` = não foi possível verificar.
#[cfg(target_os = "macos")]
pub fn allows_write(path: &Path) -> Result<bool, String> {
    use std::ffi::{c_char, c_int, c_void, CString};
    use std::os::unix::ffi::OsStrExt;

    type Acl = *mut c_void;
    type Entry = *mut c_void;
    extern "C" {
        fn acl_get_file(path: *const c_char, ty: c_int) -> Acl;
        fn acl_get_entry(acl: Acl, id: c_int, entry: *mut Entry) -> c_int;
        fn acl_get_tag_type(entry: Entry, tag: *mut c_int) -> c_int;
        fn acl_get_permset_mask_np(entry: Entry, mask: *mut u64) -> c_int;
        fn acl_free(obj: *mut c_void) -> c_int;
    }
    const ACL_TYPE_EXTENDED: c_int = 0x100;
    const ACL_FIRST_ENTRY: c_int = 0;
    const ACL_NEXT_ENTRY: c_int = -1;
    const ACL_EXTENDED_ALLOW: c_int = 1;
    // write_data/add_file, delete, append_data/add_subdirectory, delete_child,
    // write_attributes, write_extattributes, write_security, change_owner
    const WRITE_LIKE: u64 =
        (1 << 2) | (1 << 4) | (1 << 5) | (1 << 6) | (1 << 8) | (1 << 10) | (1 << 12) | (1 << 13);

    let c = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| "caminho com byte nulo".to_string())?;
    // SAFETY: `c` é uma string C válida durante a chamada.
    let acl = unsafe { acl_get_file(c.as_ptr(), ACL_TYPE_EXTENDED) };
    if acl.is_null() {
        // ENOENT = sem ACL; ENOTSUP = sistema de arquivos sem ACLs. Qualquer outro erro: não dá para afirmar.
        return match std::io::Error::last_os_error().raw_os_error() {
            Some(e) if e == libc::ENOENT || e == libc::ENOTSUP => Ok(false),
            other => Err(format!(
                "não foi possível ler a ACL de {} (errno {other:?})",
                path.display()
            )),
        };
    }
    let mut found = false;
    let mut failed = false;
    let mut id = ACL_FIRST_ENTRY;
    loop {
        let mut entry: Entry = std::ptr::null_mut();
        // No macOS, `acl_get_entry` devolve 0 ao entregar uma entrada e -1 ao acabar a lista
        // (errno EINVAL); NÃO é a convenção POSIX.1e (1 = entrada). Qualquer outro erro falha fechado.
        // SAFETY: `acl` é válido até o `acl_free` abaixo; `entry` recebe um ponteiro interno dele.
        if unsafe { acl_get_entry(acl, id, &mut entry) } != 0 {
            if std::io::Error::last_os_error().raw_os_error() != Some(libc::EINVAL) {
                failed = true;
            }
            break;
        }
        id = ACL_NEXT_ENTRY;
        let (mut tag, mut mask) = (0 as c_int, 0u64);
        // SAFETY: `entry` veio de `acl_get_entry` e `acl` ainda não foi liberado.
        let ok = unsafe {
            acl_get_tag_type(entry, &mut tag) == 0 && acl_get_permset_mask_np(entry, &mut mask) == 0
        };
        if !ok {
            failed = true;
            break;
        }
        if tag == ACL_EXTENDED_ALLOW && mask & WRITE_LIKE != 0 {
            found = true;
            break;
        }
    }
    // SAFETY: libera exatamente o objeto retornado por `acl_get_file`.
    unsafe { acl_free(acl) };
    if failed {
        return Err(format!("ACL de {} ilegível", path.display()));
    }
    Ok(found)
}

/// Linux: ACLs POSIX ficam nos atributos estendidos do kernel (`system.posix_acl_access` e
/// `system.posix_acl_default`). Sem atributo = sem ACL; atributo ilegível = reprova (falha fechada).
#[cfg(target_os = "linux")]
pub fn allows_write(path: &Path) -> Result<bool, String> {
    use std::ffi::{c_void, CString};
    use std::os::unix::ffi::OsStrExt;

    let c = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| "caminho com byte nulo".to_string())?;
    for name in ["system.posix_acl_access", "system.posix_acl_default"] {
        let n = CString::new(name).expect("nome fixo");
        let mut buf = vec![0u8; 4096];
        // SAFETY: `c` e `n` são strings C válidas durante a chamada; `buf` tem o tamanho informado.
        let len = unsafe { libc::getxattr(c.as_ptr(), n.as_ptr(), buf.as_mut_ptr() as *mut c_void, buf.len()) };
        if len < 0 {
            return match std::io::Error::last_os_error().raw_os_error() {
                // sem esse atributo, ou sistema de arquivos sem ACL: nada a conceder
                Some(e) if e == libc::ENODATA || e == libc::ENOTSUP => continue,
                Some(libc::ERANGE) => Err(format!("ACL de {} grande demais para verificar", path.display())),
                other => Err(format!("não foi possível ler a ACL de {} (errno {other:?})", path.display())),
            };
        }
        if grants_write(&buf[..len as usize])? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Interpreta o formato POSIX ACL v2 do kernel Linux (cabeçalho de 4 bytes; entradas de 8 bytes, little-endian).
/// Concede escrita quando uma entrada nomeada de usuário (0x02), de grupo (0x08) ou "outros" (0x20) tem escrita.
/// As entradas do dono (0x01) e do grupo dono (0x04) são julgadas pelos bits de modo, já verificados.
pub fn grants_write(acl: &[u8]) -> Result<bool, String> {
    const WRITE: u16 = 0x2;
    const NAMED_USER: u16 = 0x02;
    const NAMED_GROUP: u16 = 0x08;
    const OTHER: u16 = 0x20;
    if acl.len() < 4 || (acl.len() - 4) % 8 != 0 {
        return Err("ACL malformada".into());
    }
    if u32::from_le_bytes([acl[0], acl[1], acl[2], acl[3]]) != 2 {
        return Err("versão de ACL desconhecida".into());
    }
    for e in acl[4..].chunks_exact(8) {
        let tag = u16::from_le_bytes([e[0], e[1]]);
        let perm = u16::from_le_bytes([e[2], e[3]]);
        if matches!(tag, NAMED_USER | NAMED_GROUP | OTHER) && perm & WRITE != 0 {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Outras plataformas: sem verificação. Falha fechada: o executável não é aprovável.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn allows_write(_path: &Path) -> Result<bool, String> {
    Err("verificação de ACL não implementada nesta plataforma".into())
}

#[cfg(test)]
mod tests {
    use super::grants_write;

    /// Monta uma ACL v2 com as entradas (tag, permissão).
    fn acl(entries: &[(u16, u16)]) -> Vec<u8> {
        let mut v = 2u32.to_le_bytes().to_vec();
        for (tag, perm) in entries {
            v.extend_from_slice(&tag.to_le_bytes());
            v.extend_from_slice(&perm.to_le_bytes());
            v.extend_from_slice(&0u32.to_le_bytes());
        }
        v
    }

    #[test]
    fn owner_and_group_entries_are_left_to_the_mode_bits() {
        assert!(!grants_write(&acl(&[(0x01, 7), (0x04, 5), (0x10, 7), (0x20, 5)])).unwrap());
    }

    #[test]
    fn named_user_group_or_other_with_write_is_refused() {
        assert!(grants_write(&acl(&[(0x01, 7), (0x02, 6), (0x20, 5)])).unwrap(), "usuário nomeado com escrita");
        assert!(grants_write(&acl(&[(0x08, 2), (0x20, 5)])).unwrap(), "grupo nomeado com escrita");
        assert!(grants_write(&acl(&[(0x20, 2)])).unwrap(), "outros com escrita");
        assert!(!grants_write(&acl(&[(0x02, 4), (0x20, 4)])).unwrap(), "leitura não concede escrita");
    }

    #[test]
    fn malformed_or_unknown_acl_fails_closed() {
        assert!(grants_write(&[1, 0, 0]).is_err());
        assert!(grants_write(&[3, 0, 0, 0]).is_err(), "versão desconhecida");
        let mut bad = acl(&[(0x01, 7)]);
        bad.push(0);
        assert!(grants_write(&bad).is_err(), "entrada incompleta");
    }
}
