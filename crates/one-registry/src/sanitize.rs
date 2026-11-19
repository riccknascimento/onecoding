//! Varredura de documentos de máquina (ONENODE.md) antes de virarem contexto de IA.
//! Procura padrões de segredo e informa só a linha e o tipo, nunca o valor encontrado.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub line: usize,
    pub kind: &'static str,
}

const RULES: &[(&str, &str)] = &[
    ("-----BEGIN ", "chave privada"),
    ("AKIA", "chave de acesso de nuvem"),
    ("sk-", "chave de API"),
    ("postgres://", "URL de banco com credencial"),
    ("password=", "senha em texto"),
    ("passwd=", "senha em texto"),
    ("secret=", "segredo em texto"),
    ("token=", "token em texto"),
    ("senha=", "senha em texto"),
];

pub fn scan_document(text: &str) -> Vec<Finding> {
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let lower = line.to_lowercase();
        for (needle, kind) in RULES {
            let hit = if needle.chars().all(|c| c.is_ascii_uppercase()) || needle.contains("BEGIN") {
                line.contains(needle)
            } else {
                lower.contains(needle)
            };
            // `sk-` só conta quando parece chave (não é só um hífen no meio de uma palavra)
            let hit = hit && (*needle != "sk-" || line.split(|c: char| !c.is_ascii_alphanumeric() && c != '-').any(|w| w.starts_with("sk-") && w.len() >= 20));
            if hit {
                out.push(Finding { line: i + 1, kind });
                break;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_secrets_by_line_without_echoing_values() {
        let doc = "# OneNode\nSistema: Ubuntu\npassword=abc123\nurl: postgres://u:p@h/db\n-----BEGIN OPENSSH PRIVATE KEY-----\n";
        let f = scan_document(doc);
        assert_eq!(f.iter().map(|x| x.line).collect::<Vec<_>>(), vec![3, 4, 5]);
        assert!(f.iter().all(|x| !format!("{x:?}").contains("abc123")), "o valor nunca aparece no achado");
    }

    #[test]
    fn ordinary_documentation_is_clean() {
        let doc = "Projetos: SystemFlow, Atendimento. Docker e Caddy.\nVeja docs/token-de-exemplo.md sobre o fluxo.\nsk-curto";
        assert!(scan_document(doc).is_empty());
    }

    #[test]
    fn long_sk_key_is_flagged() {
        let doc = "chave: sk-abcdefghijklmnopqrstuvwx";
        assert_eq!(scan_document(doc).len(), 1);
    }
}
