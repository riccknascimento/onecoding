//! OneAgent: especialista com instruções próprias, skills, ferramentas exigidas, permissões e motores.
//! O registro guarda a especificação (leve). As instruções são lidas somente para o agente selecionado,
//! então o prompt inicial nunca recebe o corpo de todos os agentes.

use std::collections::HashSet;

use serde::Serialize;

use crate::entry::{valid_id, valid_token, Permission};

/// Motores conhecidos. Um motor é um nome; a disponibilidade real é informada por quem executa.
pub const KNOWN_ENGINES: &[&str] = &["codex", "claude", "cursor"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EnginePlan {
    pub preferred: String,
    pub fallbacks: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentSpec {
    pub id: String,
    pub version: String,
    pub name: String,
    pub description: String,
    /// Frases ou palavras que indicam a intenção do usuário (minúsculas).
    pub triggers: Vec<String>,
    pub skills: Vec<String>,
    /// Ferramentas que o agente precisa. Sem todas elas disponíveis, a delegação fica bloqueada.
    pub tools: Vec<String>,
    /// Permissões efetivas exigidas. Nunca concedidas pelo agente: vêm do OneHub/política local.
    pub permissions: Vec<Permission>,
    pub engines: EnginePlan,
    pub acceptance: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct AgentDef {
    pub spec: AgentSpec,
    /// Instruções completas. Não aparecem em listagens: só via `AgentRegistry::instructions`.
    instructions: &'static str,
}

#[derive(Debug, Default)]
pub struct AgentRegistry {
    agents: Vec<AgentDef>,
}

impl AgentRegistry {
    pub fn new(agents: Vec<AgentDef>) -> Result<Self, Vec<String>> {
        let mut errors = Vec::new();
        let mut seen = HashSet::new();
        for a in &agents {
            errors.extend(validate(&a.spec));
            if !seen.insert(a.spec.id.clone()) {
                errors.push(format!("agente duplicado: `{}`", a.spec.id));
            }
        }
        if errors.is_empty() { Ok(AgentRegistry { agents }) } else { Err(errors) }
    }

    pub fn len(&self) -> usize {
        self.agents.len()
    }

    pub fn is_empty(&self) -> bool {
        self.agents.is_empty()
    }

    pub fn get(&self, id: &str) -> Option<&AgentSpec> {
        self.agents.iter().find(|a| a.spec.id == id).map(|a| &a.spec)
    }

    /// Seleciona o agente cuja intenção casa melhor com o pedido. Empate: menor id. Sem casamento: `None`.
    pub fn select(&self, intent: &str) -> Option<&AgentSpec> {
        // gatilho casa quando TODAS as suas palavras aparecem na intenção, em qualquer ordem e com palavras no meio
        let words: HashSet<String> = intent
            .to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .map(str::to_string)
            .collect();
        self.agents
            .iter()
            .map(|a| {
                let score = a
                    .spec
                    .triggers
                    .iter()
                    .filter(|t| t.split_whitespace().all(|w| words.contains(&w.to_lowercase())))
                    .map(|t| t.len())
                    .sum::<usize>();
                (score, &a.spec)
            })
            .filter(|(score, _)| *score > 0)
            .max_by(|a, b| a.0.cmp(&b.0).then_with(|| b.1.id.cmp(&a.1.id)))
            .map(|(_, spec)| spec)
    }

    /// Instruções completas de um agente. Chamada só depois da seleção.
    pub fn instructions(&self, id: &str) -> Option<&'static str> {
        self.agents.iter().find(|a| a.spec.id == id).map(|a| a.instructions)
    }
}

fn validate(s: &AgentSpec) -> Vec<String> {
    let mut e = Vec::new();
    if !valid_id(&s.id) {
        e.push(format!("id de agente inválido `{}`", s.id));
    }
    if !crate::entry::valid_version(&s.version) {
        e.push(format!("versão inválida no agente `{}`", s.id));
    }
    if s.name.trim().is_empty() || s.description.trim().is_empty() {
        e.push(format!("agente `{}` precisa de nome e descrição", s.id));
    }
    if s.triggers.is_empty() || s.triggers.iter().any(|t| t.trim().is_empty() || t != &t.to_lowercase()) {
        e.push(format!("agente `{}`: gatilhos vazios ou não normalizados (minúsculas)", s.id));
    }
    if s.tools.is_empty() {
        e.push(format!("agente `{}` precisa declarar ao menos uma ferramenta", s.id));
    }
    if s.acceptance.is_empty() {
        e.push(format!("agente `{}` precisa de critérios de aceite", s.id));
    }
    if !KNOWN_ENGINES.contains(&s.engines.preferred.as_str()) {
        e.push(format!("motor preferido desconhecido em `{}`: {}", s.id, s.engines.preferred));
    }
    for f in &s.engines.fallbacks {
        if !KNOWN_ENGINES.contains(&f.as_str()) || *f == s.engines.preferred {
            e.push(format!("motor alternativo inválido em `{}`: {f}", s.id));
        }
    }
    if s.skills.iter().chain(s.tools.iter()).any(|t| !valid_token(t)) {
        e.push(format!("skill ou ferramenta com nome inválido em `{}`", s.id));
    }
    e
}

/// Agentes embutidos. Hoje só existe o primeiro caso real: configuração de OneNodes.
pub fn builtin() -> Vec<AgentDef> {
    vec![AgentDef {
        spec: AgentSpec {
            id: "onenode-configuration".into(),
            version: "1.0.0".into(),
            name: "Configuração de OneNode".into(),
            description: "Configura, instala, pareia, valida e acompanha a instalação de novos OneNodes.".into(),
            triggers: vec![
                "onenode".into(),
                "adicionar máquina".into(),
                "adicionar maquina".into(),
                "nova vps".into(),
                "adicionar vps".into(),
                "instalar onenode".into(),
                "instale onenode".into(),
                "adicione maquina".into(),
                "adicione máquina".into(),
                "adicione vps".into(),
                "pareie onenode".into(),
                "parear".into(),
            ],
            skills: vec!["ssh-verification".into(), "onenode-install".into()],
            tools: vec!["onehub-node-admin".into(), "provisioner-install".into()],
            permissions: vec![Permission::Network],
            engines: EnginePlan { preferred: "codex".into(), fallbacks: vec!["claude".into(), "cursor".into()] },
            acceptance: vec![
                "Sistema operacional e requisitos identificados antes de qualquer instalação.".into(),
                "Identidade SSH confirmada pelo usuário antes de qualquer autenticação.".into(),
                "Instalação feita somente pelo provisionador autorizado, nunca pelo agente.".into(),
                "OneNode pareado e com heartbeat recente registrado pelo OneHub.".into(),
                "Nenhuma credencial aparece em prompt, log ou evento.".into(),
            ],
        },
        instructions: include_str!("../agents/onenode-configuration.md"),
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_agent_is_valid_and_selectable_by_intent() {
        let r = AgentRegistry::new(builtin()).unwrap();
        assert_eq!(r.select("quero adicionar uma nova VPS na Apliflow").unwrap().id, "onenode-configuration");
        assert!(r.select("corrija o login do ERP").is_none(), "intenção sem gatilho não seleciona agente");
        assert!(r.select("adicione uma nova máquina").is_some(), "palavras intermediárias não impedem o casamento");
    }

    #[test]
    fn instructions_are_never_part_of_the_spec_listing() {
        let r = AgentRegistry::new(builtin()).unwrap();
        let spec = r.select("instalar onenode").unwrap();
        let json = serde_json::to_string(spec).unwrap();
        assert!(!json.contains("Missão") && !json.contains("Procedimento"), "o corpo das instruções não entra na especificação");
        assert!(r.instructions("onenode-configuration").unwrap().contains("Critérios de aceite"));
        assert!(r.instructions("inexistente").is_none());
    }

    #[test]
    fn invalid_agents_are_refused_with_every_problem_listed() {
        let mut bad = builtin().remove(0);
        bad.spec.engines = EnginePlan { preferred: "chatgpt".into(), fallbacks: vec!["gemini".into()] };
        bad.spec.triggers = vec!["Adicionar".into()];
        bad.spec.acceptance.clear();
        let errs = AgentRegistry::new(vec![bad]).unwrap_err();
        assert!(errs.iter().any(|m| m.contains("motor preferido")));
        assert!(errs.iter().any(|m| m.contains("motor alternativo")));
        assert!(errs.iter().any(|m| m.contains("normalizados")));
        assert!(errs.iter().any(|m| m.contains("critérios de aceite")));
    }

    #[test]
    fn duplicate_agent_ids_are_refused() {
        let errs = AgentRegistry::new(vec![builtin().remove(0), builtin().remove(0)]).unwrap_err();
        assert!(errs.iter().any(|m| m.contains("duplicado")));
    }
}
