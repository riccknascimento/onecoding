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

/// Outras plataformas: ainda sem verificação (Linux usa ACLs POSIX; entra na fase do Gateway Linux).
/// Falha fechada: sem verificação, o executável não é aprovável.
#[cfg(not(target_os = "macos"))]
pub fn allows_write(_path: &Path) -> Result<bool, String> {
    Err("verificação de ACL não implementada nesta plataforma".into())
}
