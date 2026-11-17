//! Parsers puros (testáveis sem CLI). Nunca devolvem texto bruto da CLI.

use one_protocol::{AuthState, ModelInfo, ModelSource};

/// Primeiro token que começa com dígito e contém ponto (`0.169.0`, `2026.11.13-eacade1`).
pub fn version(out: &str) -> Option<String> {
    out.split_whitespace()
        .find(|t| t.chars().next().is_some_and(|c| c.is_ascii_digit()) && t.contains('.'))
        .map(str::to_string)
}

/// Saída textual tipo "Logged in using ChatGPT" / "Not logged in".
pub fn auth_text(out: &str) -> AuthState {
    let l = out.to_lowercase();
    if l.contains("not logged in") || l.contains("logged out") {
        AuthState::LoggedOut
    } else if l.contains("logged in") {
        AuthState::LoggedIn
    } else {
        AuthState::Unknown
    }
}

/// `claude auth status` devolve JSON; só o campo `loggedIn` é lido (e-mail/org são ignorados).
pub fn auth_claude_json(out: &str) -> AuthState {
    match serde_json::from_str::<serde_json::Value>(out)
        .ok()
        .and_then(|v| v.get("loggedIn")?.as_bool())
    {
        Some(true) => AuthState::LoggedIn,
        Some(false) => AuthState::LoggedOut,
        None => AuthState::Unknown,
    }
}

/// `codex debug models`: `{"models":[{"slug","display_name","visibility"}]}`; só os visíveis.
pub fn codex_models(out: &str) -> Vec<ModelInfo> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(out) else {
        return vec![];
    };
    v.get("models")
        .and_then(|m| m.as_array())
        .map(|arr| {
            arr.iter()
                .filter(|m| {
                    m.get("visibility")
                        .and_then(|x| x.as_str())
                        .is_none_or(|x| x == "list")
                })
                .filter_map(|m| {
                    Some(ModelInfo {
                        id: m.get("slug")?.as_str()?.to_string(),
                        label: m
                            .get("display_name")
                            .and_then(|x| x.as_str())
                            .map(str::to_string),
                        source: ModelSource::Catalog,
                        confirmed: true,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// `cursor-agent --list-models`: linhas `id - Rótulo` após o título "Available models".
pub fn cursor_models(out: &str) -> Vec<ModelInfo> {
    out.lines()
        .filter_map(|line| {
            let (id, label) = line.split_once(" - ")?;
            let id = id.trim();
            let valid = !id.is_empty()
                && id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-._".contains(c));
            valid.then(|| ModelInfo {
                id: id.to_string(),
                label: Some(label.trim().to_string()),
                source: ModelSource::Catalog,
                confirmed: true,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert_eq!(version("codex-cli 0.169.0").as_deref(), Some("0.169.0"));
        assert_eq!(version("2.1.326 (Claude Code)").as_deref(), Some("2.1.326"));
        assert_eq!(
            version("2026.11.13-eacade1").as_deref(),
            Some("2026.11.13-eacade1")
        );
        assert_eq!(version("sem versão"), None);
    }

    #[test]
    fn auth_variants() {
        assert_eq!(auth_text("✓ Logged in as x"), AuthState::LoggedIn);
        assert_eq!(auth_text("Not logged in"), AuthState::LoggedOut);
        assert_eq!(auth_text("???"), AuthState::Unknown);
        assert_eq!(
            auth_claude_json(r#"{"loggedIn":false}"#),
            AuthState::LoggedOut
        );
        assert_eq!(auth_claude_json("lixo"), AuthState::Unknown);
    }

    #[test]
    fn cursor_models_parse_and_ignore_titles() {
        let out = "Available models\n\nauto - Auto (current, default)\ngpt-5.2 - GPT-5.2\nTip: use --model\n";
        let m = cursor_models(out);
        assert_eq!(
            m.iter().map(|x| x.id.as_str()).collect::<Vec<_>>(),
            ["auto", "gpt-5.2"]
        );
    }

    #[test]
    fn codex_models_tolerate_garbage() {
        assert!(codex_models("não é json").is_empty());
        assert!(codex_models(r#"{"x":1}"#).is_empty());
    }
}
