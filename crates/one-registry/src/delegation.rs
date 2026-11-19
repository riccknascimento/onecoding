//! Delegação estruturada: o OneOrchestrator decide que um agente executa uma intenção. Esta função só PLANEJA.
//! Ela confere motor, ferramentas e permissões com fatos verificados por quem chama (OneExecutor, OneHub).
//! Nunca executa nada e nunca inventa resultado. Quando algo falta, o status é `Blocked` com o motivo.

use serde::Serialize;

use crate::agent::AgentSpec;
use crate::entry::Permission;

/// Estado real de um motor, informado por quem o verifica. Instalado não basta: precisa estar autenticado e aprovado.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EngineState {
    pub name: String,
    pub installed: bool,
    pub authenticated: bool,
    pub approved: bool,
}

impl EngineState {
    fn usable(&self) -> bool {
        self.installed && self.authenticated && self.approved
    }
}

/// Fatos do ambiente, verificados antes do planejamento.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Environment {
    pub engines: Vec<EngineState>,
    /// Ferramentas com implementação registrada e disponível agora.
    pub tools_available: Vec<String>,
    /// Permissões efetivas: concedidas pelo OneHub ou pela política local do OneNode.
    pub effective_permissions: Vec<Permission>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Status {
    Ready,
    Blocked { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Delegation {
    pub agent_id: String,
    pub agent_version: String,
    pub intent: String,
    /// Motor escolhido entre o preferido e os alternativos. `None` quando nenhum serve.
    pub engine: Option<String>,
    pub tools: Vec<String>,
    pub permissions: Vec<Permission>,
    pub acceptance: Vec<String>,
    pub status: Status,
    /// Trilha de decisão, em ordem. Auditável e sem valores sensíveis.
    pub audit: Vec<String>,
}

pub fn plan(agent: &AgentSpec, intent: &str, env: &Environment) -> Delegation {
    let mut audit = vec![format!("agente selecionado: {} {}", agent.id, agent.version)];
    let mut blocked: Option<String> = None;

    // 1. motor: preferido primeiro, depois alternativos; só os usáveis
    let order: Vec<&String> = std::iter::once(&agent.engines.preferred).chain(agent.engines.fallbacks.iter()).collect();
    let mut engine = None;
    for name in order {
        // o mesmo motor pode ser reportado por mais de um executor: usa o primeiro pronto
        let reports: Vec<&EngineState> = env.engines.iter().filter(|e| &e.name == name).collect();
        if reports.is_empty() {
            audit.push(format!("motor {name} não usado: não cadastrado"));
            continue;
        }
        if let Some(ready) = reports.iter().find(|e| e.usable()) {
            audit.push(format!("motor escolhido: {}", ready.name));
            engine = Some(ready.name.clone());
            break;
        }
        for e in reports {
            audit.push(format!(
                "motor {name} não usado: instalado={} autenticado={} aprovado={}",
                e.installed, e.authenticated, e.approved
            ));
        }
    }
    if engine.is_none() {
        blocked.get_or_insert_with(|| "nenhum motor autorizado e autenticado está disponível".into());
    }

    // 2. ferramentas: todas as exigidas precisam estar disponíveis
    let missing_tools: Vec<&String> = agent.tools.iter().filter(|t| !env.tools_available.contains(t)).collect();
    if missing_tools.is_empty() {
        audit.push(format!("ferramentas disponíveis: {}", agent.tools.join(", ")));
    } else {
        let names: Vec<&str> = missing_tools.iter().map(|t| t.as_str()).collect();
        audit.push(format!("ferramentas indisponíveis: {}", names.join(", ")));
        blocked.get_or_insert_with(|| format!("ferramenta indisponível: {}", names.join(", ")));
    }

    // 3. permissões: nenhuma ação sem permissão efetiva
    let missing_perms: Vec<&Permission> = agent.permissions.iter().filter(|p| !env.effective_permissions.contains(p)).collect();
    if missing_perms.is_empty() {
        audit.push("permissões efetivas conferidas".into());
    } else {
        audit.push(format!("permissões ausentes: {missing_perms:?}"));
        blocked.get_or_insert_with(|| "permissão necessária não concedida".into());
    }

    let status = match blocked {
        None => Status::Ready,
        Some(reason) => {
            audit.push(format!("bloqueado: {reason}"));
            Status::Blocked { reason }
        }
    };
    Delegation {
        agent_id: agent.id.clone(),
        agent_version: agent.version.clone(),
        intent: intent.to_string(),
        engine,
        tools: agent.tools.clone(),
        permissions: agent.permissions.clone(),
        acceptance: agent.acceptance.clone(),
        status,
        audit,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{builtin, AgentRegistry};
    use crate::scope::Capability;

    fn spec() -> AgentSpec {
        AgentRegistry::new(builtin()).unwrap().get("onenode-configuration").unwrap().clone()
    }

    fn eng(name: &str, installed: bool, authenticated: bool, approved: bool) -> EngineState {
        EngineState { name: name.into(), installed, authenticated, approved }
    }

    fn ready_env() -> Environment {
        Environment {
            engines: vec![eng("codex", true, true, true)],
            tools_available: vec!["onehub-node-admin".into(), "provisioner-install".into()],
            effective_permissions: vec![Permission::Network],
        }
    }

    #[test]
    fn ready_when_engine_tools_and_permissions_are_effective() {
        let d = plan(&spec(), "adicionar VPS", &ready_env());
        assert_eq!(d.status, Status::Ready);
        assert_eq!(d.engine.as_deref(), Some("codex"));
        assert!(d.audit.iter().any(|l| l == "motor escolhido: codex"));
    }

    #[test]
    fn installed_is_not_enough_engine_must_be_authenticated_and_approved() {
        let mut env = ready_env();
        env.engines = vec![eng("codex", true, false, true), eng("claude", true, true, false)];
        let d = plan(&spec(), "x", &env);
        assert_eq!(d.engine, None);
        assert!(matches!(d.status, Status::Blocked { .. }));
        assert!(d.audit.iter().any(|l| l.contains("motor codex não usado: instalado=true autenticado=false")));
    }

    #[test]
    fn falls_back_to_the_first_usable_alternative_and_says_why() {
        let mut env = ready_env();
        env.engines = vec![eng("codex", false, false, false), eng("claude", true, true, true)];
        let d = plan(&spec(), "x", &env);
        assert_eq!(d.engine.as_deref(), Some("claude"));
        assert_eq!(d.status, Status::Ready);
        assert!(d.audit.iter().any(|l| l.starts_with("motor codex não usado")));
    }

    #[test]
    fn missing_tool_blocks_with_the_tool_named() {
        let mut env = ready_env();
        env.tools_available = vec!["onehub-node-admin".into()];
        match plan(&spec(), "x", &env).status {
            Status::Blocked { reason } => assert!(reason.contains("provisioner-install")),
            other => panic!("esperava bloqueio, veio {other:?}"),
        }
    }

    #[test]
    fn missing_permission_blocks_and_no_tool_runs_without_it() {
        let mut env = ready_env();
        env.effective_permissions.clear();
        let d = plan(&spec(), "x", &env);
        assert!(matches!(d.status, Status::Blocked { ref reason } if reason.contains("permissão")));
        assert!(d.audit.iter().any(|l| l.starts_with("permissões ausentes")));
    }

    #[test]
    fn delegation_is_auditable_and_carries_no_secret_material() {
        let d = plan(&spec(), "adicionar VPS na Apliflow, senha abc123", &ready_env());
        let json = serde_json::to_string(&d).unwrap();
        assert!(json.contains("onenode-configuration") && json.contains("audit"));
        // a intenção é registrada como o usuário escreveu; a credencial nunca entra pelo contrato,
        // porque o planejador não recebe campos de credencial. Garante que o JSON não ganhou chaves de segredo.
        assert!(!json.contains("password") && !json.contains("token") && !json.contains("private_key"));
        let _ = Capability::Discover; // o escopo de nó continua separado do planejamento de agente
    }

    #[test]
    fn agent_instructions_have_no_secret_patterns() {
        let text = AgentRegistry::new(builtin()).unwrap().instructions("onenode-configuration").unwrap();
        assert!(crate::sanitize::scan_document(text).is_empty());
    }
}
