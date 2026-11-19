//! Identifica o projeto pelo nome citado na mensagem, usando SOMENTE os projetos com concessão ativa
//! para o OneNode escolhido. Sem invenção: nome que não bate com nenhum projeto autorizado não resolve nada.

use uuid::Uuid;

use deadpool_postgres::GenericClient;

use crate::routes::ApiError;

/// Nomes muito curtos casam palavras comuns ("de", "a"); exigimos um tamanho mínimo.
const MIN_NAME_CHARS: usize = 3;

fn fold(c: char) -> char {
    match c {
        'á' | 'à' | 'â' | 'ã' | 'ä' => 'a',
        'é' | 'è' | 'ê' | 'ë' => 'e',
        'í' | 'ì' | 'î' | 'ï' => 'i',
        'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
        'ú' | 'ù' | 'û' | 'ü' => 'u',
        'ç' => 'c',
        'ñ' => 'n',
        other => other,
    }
}

/// Palavras em minúsculas, sem acento e sem pontuação.
pub fn tokens(s: &str) -> Vec<String> {
    s.to_lowercase()
        .chars()
        .map(fold)
        .collect::<String>()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(String::from)
        .collect()
}

/// Verdadeiro quando a sequência de palavras do nome aparece inteira no texto (palavra inteira).
pub fn mentions(text_tokens: &[String], name: &str) -> bool {
    let name_tokens = tokens(name);
    if name_tokens.is_empty() || name.trim().chars().count() < MIN_NAME_CHARS {
        return false;
    }
    text_tokens
        .windows(name_tokens.len())
        .any(|w| w == name_tokens.as_slice())
}

/// Projetos com concessão ativa para o OneNode cujo nome aparece no texto.
pub async fn mentioned_projects<C: GenericClient>(
    client: &C,
    node: Uuid,
    text: &str,
) -> Result<Vec<(Uuid, String)>, ApiError> {
    let rows = client
        .query(
            "SELECT p.id, p.name FROM node_project_grants g \
             JOIN projects p ON p.id = g.project_id JOIN onenodes o ON o.id = g.node_id \
             WHERE g.node_id = $1 AND g.revoked_at IS NULL AND o.revoked_at IS NULL",
            &[&node],
        )
        .await?;
    let text_tokens = tokens(text);
    Ok(rows
        .iter()
        .map(|r| (r.get::<_, Uuid>(0), r.get::<_, String>(1)))
        .filter(|(_, name)| mentions(&text_tokens, name))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(text: &str, name: &str) -> bool {
        mentions(&tokens(text), name)
    }

    #[test]
    fn matches_whole_name_ignoring_case_and_accents() {
        assert!(hit("Investigue o desempenho do SystemFlow.", "SystemFlow"));
        assert!(hit("analise o projeto FLÁVIO-app agora", "flavio app"));
    }

    #[test]
    fn does_not_match_inside_another_word() {
        assert!(!hit("analise o systemflowx", "SystemFlow"));
        assert!(!hit("o sistema flow novo", "SystemFlow"));
    }

    #[test]
    fn short_names_never_match() {
        assert!(!hit("de a o", "a"));
        assert!(!hit("de a o", "de"));
    }

    #[test]
    fn multi_word_name_needs_the_sequence() {
        assert!(hit("veja o apliflow web", "Apliflow Web"));
        assert!(!hit("veja o apliflow", "Apliflow Web"));
    }
}
