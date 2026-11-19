//! Escopo de contexto de uma conversa. Três escopos distintos:
//! - Central: administração da plataforma, sem análise de projeto.
//! - Node: infraestrutura de uma máquina. Não exige projeto. Ferramentas só se o OneNode as oferece e o OneHub autoriza.
//! - Project: um projeto concedido a um OneNode. Exige concessão ativa.
//!
//! A decisão é pura: recebe fatos já verificados pelo OneHub e devolve o que é permitido.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "scope", rename_all = "snake_case")]
pub enum Scope {
    Central,
    Node { node_id: String },
    Project { node_id: String, project_id: String },
}

/// Capacidades que um escopo pode exercer. Só existem as que têm ferramenta registrada.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    /// Listar e localizar projetos e arquivos dentro das áreas autorizadas.
    Discover,
    /// Ler arquivos de um projeto concedido.
    ReadProject,
    /// Executar análises de um projeto por um OneExecutor.
    ExecuteProject,
    /// Criar um projeto novo dentro de uma área de trabalho autorizada.
    CreateProject,
}

/// Fatos do OneNode já verificados pelo OneHub (estado e concessões ativas).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeFacts {
    pub online: bool,
    pub suspended: bool,
    pub revoked: bool,
    pub granted_projects: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolution {
    pub allowed: Vec<Capability>,
    /// Motivo curto e acionável quando nada pode ser executado neste escopo.
    pub reason: Option<String>,
}

/// Resolve o escopo. `available` são as capacidades que têm ferramenta registrada e disponível agora.
/// Sem ferramenta, a capacidade não é permitida, mesmo que o escopo exista.
pub fn resolve(scope: &Scope, node: Option<&NodeFacts>, available: &[Capability]) -> Resolution {
    let none = |reason: &str| Resolution { allowed: vec![], reason: Some(reason.to_string()) };
    match scope {
        Scope::Central => none("Contexto OneHub Central: administração do ambiente, sem análise de projeto."),
        Scope::Node { .. } => {
            let Some(n) = node else { return none("OneNode não encontrado.") };
            if n.revoked {
                return none("Este OneNode foi revogado.");
            }
            if n.suspended {
                return none("Este OneNode está suspenso até ser retomado.");
            }
            let allowed: Vec<Capability> = [Capability::Discover, Capability::CreateProject]
                .into_iter()
                .filter(|c| available.contains(c))
                .collect();
            if allowed.is_empty() {
                return none("Nenhuma ferramenta de infraestrutura está disponível neste OneNode ainda.");
            }
            Resolution { allowed, reason: None }
        }
        Scope::Project { project_id, .. } => {
            let Some(n) = node else { return none("OneNode não encontrado.") };
            if n.revoked {
                return none("Este OneNode foi revogado.");
            }
            if n.suspended {
                return none("Este OneNode está suspenso até ser retomado.");
            }
            if !n.granted_projects.iter().any(|p| p == project_id) {
                return none("Este projeto não tem concessão ativa neste OneNode.");
            }
            let allowed: Vec<Capability> = [Capability::ReadProject, Capability::ExecuteProject]
                .into_iter()
                .filter(|c| available.contains(c))
                .filter(|c| *c != Capability::ExecuteProject || n.online)
                .collect();
            if allowed.is_empty() {
                return none("Nenhuma ferramenta disponível para este projeto agora.");
            }
            Resolution { allowed, reason: None }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(online: bool, suspended: bool, revoked: bool, projects: &[&str]) -> NodeFacts {
        NodeFacts { online, suspended, revoked, granted_projects: projects.iter().map(|p| p.to_string()).collect() }
    }

    #[test]
    fn infrastructure_scope_needs_no_project() {
        let node = Scope::Node { node_id: "apliflow".into() };
        let f = facts(true, false, false, &[]);
        // sem ferramentas registradas: não promete nada, mas também não exige projeto
        let r = resolve(&node, Some(&f), &[]);
        assert!(r.allowed.is_empty());
        assert!(r.reason.unwrap().contains("Nenhuma ferramenta de infraestrutura"));
        // com a ferramenta registrada, a capacidade passa a existir sem nenhum projeto cadastrado
        let r = resolve(&node, Some(&f), &[Capability::Discover]);
        assert_eq!(r.allowed, vec![Capability::Discover]);
        assert!(r.reason.is_none());
    }

    #[test]
    fn project_scope_requires_an_active_grant() {
        let project = Scope::Project { node_id: "apliflow".into(), project_id: "erp".into() };
        let all = [Capability::ReadProject, Capability::ExecuteProject];
        let without = resolve(&project, Some(&facts(true, false, false, &["outro"])), &all);
        assert!(without.allowed.is_empty());
        assert!(without.reason.unwrap().contains("concessão"));
        let with = resolve(&project, Some(&facts(true, false, false, &["erp"])), &all);
        assert_eq!(with.allowed, vec![Capability::ReadProject, Capability::ExecuteProject]);
    }

    #[test]
    fn execution_needs_an_online_node_but_reading_does_not() {
        let project = Scope::Project { node_id: "n".into(), project_id: "p".into() };
        let r = resolve(&project, Some(&facts(false, false, false, &["p"])), &[Capability::ReadProject, Capability::ExecuteProject]);
        assert_eq!(r.allowed, vec![Capability::ReadProject]);
    }

    #[test]
    fn suspended_and_revoked_nodes_allow_nothing() {
        let node = Scope::Node { node_id: "n".into() };
        let all = [Capability::Discover, Capability::CreateProject];
        assert!(resolve(&node, Some(&facts(true, true, false, &[])), &all).allowed.is_empty());
        assert!(resolve(&node, Some(&facts(true, false, true, &[])), &all).allowed.is_empty());
    }

    #[test]
    fn central_scope_never_grants_node_or_project_capabilities() {
        let r = resolve(&Scope::Central, None, &[Capability::Discover]);
        assert!(r.allowed.is_empty());
        assert!(r.reason.unwrap().contains("OneHub Central"));
    }

    #[test]
    fn create_project_is_a_separate_capability_and_needs_its_own_tool() {
        let node = Scope::Node { node_id: "n".into() };
        let f = facts(true, false, false, &[]);
        assert!(!resolve(&node, Some(&f), &[Capability::Discover]).allowed.contains(&Capability::CreateProject));
        assert!(resolve(&node, Some(&f), &[Capability::CreateProject]).allowed.contains(&Capability::CreateProject));
    }
}
