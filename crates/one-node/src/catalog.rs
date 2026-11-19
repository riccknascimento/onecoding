//! Sincronização do catálogo de projetos. O OneHub informa projetos e raízes pedidas; o nó valida cada raiz
//! contra a política local, aplica o resultado na memória (sem reiniciar o serviço) e só então confirma ao
//! OneHub a versão efetivamente aplicada. Revogação de concessão aparece como projeto removido no próximo ciclo.

use std::path::Path;

use crate::agent::Roots;
use crate::central::{Central, CentralError};
use crate::policy;

#[derive(Default)]
pub struct Catalog {
    /// Versão do catálogo confirmada ao OneHub.
    pub applied_version: Option<i64>,
    /// Digest da política local usada na última aplicação: mudar a política também reaplica.
    pub applied_policy: Option<u64>,
    /// Raízes em uso agora (já validadas). Vazio = nenhum projeto acessível.
    pub roots: Roots,
}

/// Sincroniza se houve mudança (no OneHub ou na política local). Retorna `true` quando aplicou algo novo.
pub fn sync(central: &Central, policy_path: &Path, catalog: &mut Catalog) -> Result<bool, CentralError> {
    let policy = policy::load(policy_path).unwrap_or_default();
    let remote = central.catalog()?;
    if catalog.applied_version == Some(remote.version) && catalog.applied_policy == Some(policy.digest) {
        return Ok(false);
    }
    let mut accepted: Vec<String> = Vec::new();
    let mut rejected: Vec<(String, String)> = Vec::new();
    let mut roots: Vec<(String, std::path::PathBuf)> = Vec::new();
    for p in &remote.projects {
        match policy::validate_root(&policy, &p.root_path) {
            Ok(canon) => {
                accepted.push(p.project_id.clone());
                roots.push((p.project_id.clone(), canon));
            }
            Err(reason) => rejected.push((p.project_id.clone(), reason.to_string())),
        }
    }
    // Só troca o estado em uso depois que o OneHub registrou a versão. Se a versão mudou no meio
    // (409), nada é trocado e o próximo ciclo sincroniza de novo.
    central.report_applied(remote.version, &accepted, &rejected)?;
    catalog.roots = Roots(roots);
    catalog.applied_version = Some(remote.version);
    catalog.applied_policy = Some(policy.digest);
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_catalog_has_no_roots_and_no_applied_version() {
        let c = Catalog::default();
        assert!(c.roots.0.is_empty());
        assert_eq!(c.applied_version, None);
    }
}
