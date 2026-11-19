//! Entrada do OneRegistry. Metadados apenas: o corpo das skills é carregado depois, sob demanda.
//! Permissões de catálogo descrevem o que um recurso PODE exigir; nunca concedem acesso. Concessão é do OneHub.

use serde::{Deserialize, Serialize};

pub const MAX_DESCRIPTION: usize = 240;
pub const MAX_TOKENS: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Agent,
    Skill,
    Mcp,
    Plugin,
    Tool,
    Executor,
}

/// Permissões que um recurso pode declarar como requisito. Segredos não entram no catálogo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    Read,
    Write,
    Execute,
    Deploy,
    Network,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    pub kind: Kind,
    pub version: String,
    pub description: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub requires: Vec<String>,
    #[serde(default)]
    pub permissions: Vec<Permission>,
    #[serde(default)]
    pub compatibility: Vec<String>,
}

/// Visão mínima usada na busca: o que o orquestrador lê antes de decidir carregar o recurso.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Summary {
    pub id: String,
    pub kind: Kind,
    pub version: String,
    pub description: String,
}

impl Entry {
    pub fn summary(&self) -> Summary {
        Summary {
            id: self.id.clone(),
            kind: self.kind,
            version: self.version.clone(),
            description: self.description.clone(),
        }
    }

    /// Valida o registro. Retorna a lista completa de problemas, para corrigir de uma vez.
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();
        if !valid_id(&self.id) {
            errors.push(format!("id inválido `{}`: use minúsculas, dígitos e hífen (2 a 63 caracteres)", self.id));
        }
        if !valid_version(&self.version) {
            errors.push(format!("versão inválida `{}`: use MAJOR.MINOR.PATCH", self.version));
        }
        if self.description.trim().is_empty() || self.description.chars().count() > MAX_DESCRIPTION {
            errors.push(format!("descrição deve ter de 1 a {MAX_DESCRIPTION} caracteres"));
        }
        for list in [&self.capabilities, &self.requires, &self.compatibility] {
            if list.len() > MAX_TOKENS {
                errors.push(format!("no máximo {MAX_TOKENS} itens por lista (`{}`)", self.id));
            }
            if list.iter().any(|t| !valid_token(t)) {
                errors.push(format!("token inválido em `{}`: use minúsculas, dígitos, `-`, `_`, `.`", self.id));
            }
        }
        if errors.is_empty() { Ok(()) } else { Err(errors) }
    }
}

pub fn valid_id(id: &str) -> bool {
    let len = id.len();
    (2..=63).contains(&len)
        && id.starts_with(|c: char| c.is_ascii_lowercase())
        && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

pub fn valid_token(t: &str) -> bool {
    !t.is_empty()
        && t.len() <= 40
        && t.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'_' | b'.'))
}

pub fn valid_version(v: &str) -> bool {
    let parts: Vec<&str> = v.split('.').collect();
    parts.len() == 3 && parts.iter().all(|p| !p.is_empty() && p.len() <= 6 && p.bytes().all(|b| b.is_ascii_digit()))
}
