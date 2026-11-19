//! Classificação de fontes de conhecimento. Documento orienta; registro descreve; só permissão autoriza.
//! Regra central: nada que venha de um documento ou de um inventário pode conceder acesso.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceClass {
    /// Instruções: ONECODING.md, ONENODE.md, AGENTS.md, CLAUDE.md, SKILL.md. Orientam, não concedem.
    Guidance,
    /// Inventário estruturado (OneRegistry, descobertas confirmadas). Descreve; não concede.
    Record,
    /// Concessão efetiva gravada pelo OneHub (node_project_grants, políticas locais). Única que autoriza.
    Permission,
}

pub fn classify(name: &str) -> SourceClass {
    let base = name.rsplit('/').next().unwrap_or(name).to_ascii_uppercase();
    match base.as_str() {
        "ONECODING.MD" | "ONENODE.MD" | "AGENTS.MD" | "CLAUDE.MD" | "SKILL.MD" => SourceClass::Guidance,
        "REGISTRY.JSON" => SourceClass::Record,
        _ => SourceClass::Guidance,
    }
}

/// Só concessões do OneHub e políticas locais autorizam ações.
pub fn can_authorize(class: SourceClass) -> bool {
    class == SourceClass::Permission
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn documents_never_authorize_anything() {
        for name in ["ONECODING.md", "docs/ONENODE.md", "AGENTS.md", "skills/auth/SKILL.md"] {
            assert_eq!(classify(name), SourceClass::Guidance);
            assert!(!can_authorize(classify(name)), "{name} não pode autorizar");
        }
        assert!(!can_authorize(SourceClass::Record));
        assert!(can_authorize(SourceClass::Permission));
    }

    #[test]
    fn unknown_sources_default_to_guidance_not_permission() {
        assert_eq!(classify("qualquer-coisa.txt"), SourceClass::Guidance);
    }
}
