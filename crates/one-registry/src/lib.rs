//! OneRegistry: catálogo validado e busca progressiva. Contratos de escopo, classificação de documentos e
//! varredura de segredos ficam em módulos próprios. Nenhum módulo deste crate faz I/O.

pub mod agent;
pub mod delegation;
pub mod entry;
pub mod sanitize;
pub mod scope;
pub mod source;

use std::collections::HashSet;

pub use entry::{Entry, Kind, Permission, Summary};

#[derive(Debug, Default)]
pub struct Registry {
    entries: Vec<Entry>,
}

impl Registry {
    /// Carrega e valida o catálogo inteiro. Qualquer erro impede o uso do catálogo.
    pub fn from_entries(entries: Vec<Entry>) -> Result<Self, Vec<String>> {
        let mut errors = Vec::new();
        let mut seen = HashSet::new();
        for e in &entries {
            if let Err(mut errs) = e.validate() {
                errors.append(&mut errs);
            }
            if !seen.insert((e.kind, e.id.clone())) {
                errors.push(format!("recurso duplicado: {:?} `{}`", e.kind, e.id));
            }
        }
        if errors.is_empty() {
            Ok(Registry { entries })
        } else {
            Err(errors)
        }
    }

    pub fn from_json(text: &str) -> Result<Self, Vec<String>> {
        let entries: Vec<Entry> = serde_json::from_str(text).map_err(|e| vec![format!("catálogo inválido: {e}")])?;
        Self::from_entries(entries)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Busca por termos (id, descrição, capacidades). Retorna só resumos, ordenados por relevância e id.
    /// `kinds` restringe o tipo. `limit` impede que o prompt receba o catálogo inteiro.
    pub fn search(&self, query: &str, kinds: Option<&[Kind]>, limit: usize) -> Vec<Summary> {
        let terms: Vec<String> = query
            .split(|c: char| !c.is_alphanumeric())
            .filter(|t| t.len() >= 2)
            .map(|t| t.to_lowercase())
            .collect();
        let mut scored: Vec<(usize, &Entry)> = self
            .entries
            .iter()
            .filter(|e| kinds.is_none_or(|ks| ks.contains(&e.kind)))
            .filter_map(|e| {
                let hay = format!("{} {} {}", e.id, e.description.to_lowercase(), e.capabilities.join(" "));
                let score = terms.iter().filter(|t| hay.contains(t.as_str())).count();
                if terms.is_empty() || score > 0 { Some((score, e)) } else { None }
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.id.cmp(&b.1.id)));
        scored.into_iter().take(limit).map(|(_, e)| e.summary()).collect()
    }

    pub fn get(&self, kind: Kind, id: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.kind == kind && e.id == id)
    }
}

/// Caminho mínimo do OneOrchestrator: seleciona o agente pela intenção, carrega as instruções só para ele
/// e planeja a delegação com os fatos verificados. Não executa nada.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Orchestration {
    pub delegation: Option<delegation::Delegation>,
    pub instructions: Option<&'static str>,
    /// Quando nenhum agente casa a intenção, o orquestrador responde isso em vez de inventar um especialista.
    pub no_agent_reason: Option<String>,
}

pub fn orchestrate(agents: &agent::AgentRegistry, intent: &str, env: &delegation::Environment) -> Orchestration {
    match agents.select(intent) {
        None => Orchestration {
            delegation: None,
            instructions: None,
            no_agent_reason: Some("Nenhum agente cadastrado corresponde a esta intenção.".into()),
        },
        Some(spec) => {
            let instructions = agents.instructions(&spec.id);
            Orchestration {
                delegation: Some(delegation::plan(spec, intent, env)),
                instructions,
                no_agent_reason: None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(id: &str, kind: Kind, desc: &str, caps: &[&str]) -> Entry {
        Entry {
            id: id.into(),
            kind,
            version: "1.0.0".into(),
            description: desc.into(),
            capabilities: caps.iter().map(|c| c.to_string()).collect(),
            requires: vec![],
            permissions: vec![Permission::Read],
            compatibility: vec![],
        }
    }

    #[test]
    fn catalog_rejects_duplicates_and_invalid_entries_all_at_once() {
        let bad = Entry { version: "1.0".into(), ..e("Bad_Id", Kind::Skill, "", &["ok"]) };
        let errs = Registry::from_entries(vec![e("auth", Kind::Skill, "x", &[]), e("auth", Kind::Skill, "x", &[]), bad]).unwrap_err();
        assert!(errs.iter().any(|m| m.contains("duplicado")));
        assert!(errs.iter().any(|m| m.contains("id inválido")));
        assert!(errs.iter().any(|m| m.contains("versão inválida")));
        assert!(errs.iter().any(|m| m.contains("descrição")));
    }

    #[test]
    fn same_id_in_different_kinds_is_allowed() {
        assert!(Registry::from_entries(vec![e("deploy", Kind::Skill, "x", &[]), e("deploy", Kind::Tool, "y", &[])]).is_ok());
    }

    #[test]
    fn search_returns_summaries_only_ranked_and_limited() {
        let r = Registry::from_entries(vec![
            e("auth-review", Kind::Skill, "revisão de autenticação e sessão", &["auth", "seguranca"]),
            e("postgres-tools", Kind::Tool, "consultas em PostgreSQL", &["database"]),
            e("qa-runner", Kind::Agent, "executa testes e valida critérios", &["testes"]),
        ])
        .unwrap();
        let hits = r.search("corrigir autenticação do ERP", None, 5);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "auth-review");
        assert!(r.search("teste", Some(&[Kind::Tool]), 5).is_empty(), "filtro por tipo é respeitado");
        assert_eq!(r.search("", None, 2).len(), 2, "sem termo, o limite ainda vale");
    }

    #[test]
    fn json_catalog_is_validated_on_load() {
        let ok = r#"[{"id":"qa-runner","kind":"agent","version":"1.0.0","description":"testes"}]"#;
        assert_eq!(Registry::from_json(ok).unwrap().len(), 1);
        let secret_perm = r#"[{"id":"x-tool","kind":"tool","version":"1.0.0","description":"d","permissions":["secrets"]}]"#;
        assert!(Registry::from_json(secret_perm).is_err(), "segredo não é permissão de catálogo");
    }

    #[test]
    fn orchestrate_selects_loads_and_blocks_honestly_without_an_engine() {
        use crate::agent::{builtin, AgentRegistry};
        use crate::delegation::{EngineState, Environment, Status};
        let reg = AgentRegistry::new(builtin()).unwrap();
        let env = Environment {
            engines: vec![EngineState { name: "codex".into(), installed: true, authenticated: false, approved: true }],
            tools_available: vec!["onehub-node-admin".into(), "provisioner-install".into()],
            effective_permissions: vec![Permission::Network],
        };
        let o = orchestrate(&reg, "quero adicionar uma VPS", &env);
        let d = o.delegation.expect("há agente para a intenção");
        assert_eq!(d.agent_id, "onenode-configuration");
        assert!(matches!(d.status, Status::Blocked { .. }), "sem motor autenticado, não há execução");
        assert!(o.instructions.unwrap().contains("Critérios de aceite"));
        let none = orchestrate(&reg, "corrija o login do ERP", &env);
        assert!(none.delegation.is_none() && none.instructions.is_none());
        assert!(none.no_agent_reason.unwrap().contains("Nenhum agente"));
    }
}
