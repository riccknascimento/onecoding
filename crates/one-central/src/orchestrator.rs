//! Contratos do OneOrchestrator no Central.
//!
//! - `InferenceProvider`: o provedor de IA (ChatGPT Plus, em outro momento) fica atrás deste contrato.
//!   As credenciais NUNCA ficam no Central: cada provedor as obtém do seu próprio ambiente seguro (no Mac,
//!   o Keychain; no futuro, um executor isolado). O Central só recebe texto e devolve texto.
//! - Comandos (`/investigar`, `/corrigir`, `/melhorar`, `/testar`) são roteados de forma DETERMINÍSTICA,
//!   sem IA. Só `/investigar` cria tarefa, e só de ferramentas somente leitura do OneNode.
//!
//! Enquanto não houver provedor configurado, texto livre recebe `Unavailable` explícito. Nenhuma resposta
//! simulada é gerada.

#[derive(Debug, PartialEq, Eq)]
pub enum InferenceOutcome {
    Reply(String),
    Unavailable(&'static str),
}

pub trait InferenceProvider: Send + Sync {
    fn id(&self) -> &'static str;
    fn reply(&self, history: &[(String, String)], text: &str) -> InferenceOutcome;
}

/// Provedor padrão do Central: sem credenciais, sempre indisponível (honesto, sem resposta fictícia).
pub struct UnavailableProvider;

impl InferenceProvider for UnavailableProvider {
    fn id(&self) -> &'static str {
        "indisponivel"
    }
    fn reply(&self, _history: &[(String, String)], _text: &str) -> InferenceOutcome {
        InferenceOutcome::Unavailable("O provedor de IA não está configurado neste OneHub. Use um comando, como /investigar, para consultar o OneNode.")
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    /// `/investigar [caminho]`: lista a pasta (caminho vazio ou terminado em `/`) ou lê o arquivo.
    Investigar {
        path: String,
    },
    /// Comandos que exigem escrita ou execução de CLI: reconhecidos e recusados nesta fase.
    NotEnabled(&'static str),
    Unknown,
}

pub fn parse_command(text: &str) -> Option<Command> {
    let t = text.trim();
    let body = t.strip_prefix('/')?;
    let mut parts = body.splitn(2, char::is_whitespace);
    let name = parts.next().unwrap_or("");
    let arg = parts.next().unwrap_or("").trim().to_string();
    Some(match name {
        "investigar" => {
            let path = if arg.is_empty() { ".".to_string() } else { arg };
            Command::Investigar { path }
        }
        "corrigir" => Command::NotEnabled("corrigir (escrita) ainda não está habilitado"),
        "melhorar" => Command::NotEnabled("melhorar (escrita) ainda não está habilitado"),
        "testar" => Command::NotEnabled("testar (execução de testes) ainda não está habilitado"),
        _ => Command::Unknown,
    })
}

/// Ferramenta para um caminho de `/investigar`: pasta se for `.` ou terminar em `/`; arquivo caso contrário.
pub fn tool_for_path(path: &str) -> &'static str {
    if path == "." || path.ends_with('/') {
        "files_list"
    } else {
        "files_read"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_are_routed_deterministically() {
        assert_eq!(
            parse_command("/investigar"),
            Some(Command::Investigar { path: ".".into() })
        );
        assert_eq!(
            parse_command("/investigar  sub/in.txt "),
            Some(Command::Investigar {
                path: "sub/in.txt".into()
            })
        );
        assert!(matches!(
            parse_command("/corrigir bug"),
            Some(Command::NotEnabled(_))
        ));
        assert_eq!(parse_command("/inexistente"), Some(Command::Unknown));
        assert_eq!(parse_command("texto livre"), None);
    }

    #[test]
    fn folder_or_file_tool_is_chosen_by_path_shape() {
        assert_eq!(tool_for_path("."), "files_list");
        assert_eq!(tool_for_path("src/"), "files_list");
        assert_eq!(tool_for_path("sub/in.txt"), "files_read");
    }

    #[test]
    fn without_a_configured_provider_free_text_is_explicitly_unavailable() {
        match UnavailableProvider.reply(&[], "oi") {
            InferenceOutcome::Unavailable(m) => assert!(m.contains("não está configurado")),
            other => panic!("não deve haver resposta simulada: {other:?}"),
        }
    }
}
