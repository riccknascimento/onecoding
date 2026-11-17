//! Detecção SOMENTE LEITURA das CLIs locais (Fase 1).
//!
//! Executa apenas consultas inofensivas (`--version`, status de login, listagem de modelos),
//! sem shell, com timeout, ambiente reduzido e sem guardar saída bruta (que pode conter e-mail
//! ou outros dados pessoais). Execução de tarefas só entra na Fase 2.

mod acl;
pub mod guard;
mod host;
mod parse;
pub mod proc;
pub mod trust;

use std::time::Duration;

use one_protocol::{
    AuthState, EngineKind, EngineStatus, GatewayInfo, GatewayKind, GatewayStatus, ModelInfo,
    ModelSource, TrustState, PROTOCOL_VERSION,
};

pub use host::{Host, SystemHost};
pub use proc::{Limits, Outcome, RunOutput};
pub use trust::{Identity, Inspected, TrustStore, TrustedExe};

const QUICK: Limits = Limits {
    timeout: Duration::from_secs(5),
    max_output: 64 * 1024,
};
const SLOW: Limits = Limits {
    timeout: Duration::from_secs(12),
    max_output: 4 * 1024 * 1024,
};

/// Apelidos aceitos pelo `claude --model`; a CLI não tem comando de listagem (ver DECISIONS D-013).
const CLAUDE_ALIASES: &[&str] = &["fable", "opus", "sonnet"];

fn binary_name(engine: EngineKind) -> &'static str {
    match engine {
        EngineKind::Codex => "codex",
        EngineKind::Claude => "claude",
        EngineKind::Cursor => "cursor-agent",
    }
}

/// Detecta uma CLI. Só executa se o executável for `Trusted`; caso contrário apenas informa que
/// existe e o que falta (aprovação, nova validação ou correção de permissões).
/// `with_models = false` evita as chamadas mais lentas (podem usar rede).
pub fn detect_engine(
    engine: EngineKind,
    host: &dyn Host,
    store: &dyn TrustStore,
    with_models: bool,
) -> EngineStatus {
    let mut st = EngineStatus {
        engine,
        installed: false,
        path: None,
        version: None,
        auth: AuthState::Unknown,
        trust: TrustState::Unknown,
        fingerprint: None,
        models: vec![],
        note: None,
    };
    let Some(insp) = host.inspect(binary_name(engine)) else {
        st.note = Some("CLI não encontrada".into());
        return st;
    };
    st.installed = true;
    st.path = Some(insp.found.display().to_string());
    st.trust = trust::evaluate(&insp, store, engine);
    st.fingerprint = insp.identity.as_ref().ok().map(|i| i.fingerprint.clone());
    if st.trust != TrustState::Trusted {
        st.note = Some(match (&st.trust, &insp.identity) {
            (TrustState::Unknown, _) => "executável ainda não aprovado".to_string(),
            (TrustState::Changed, _) => {
                "o executável mudou desde a aprovação; valide de novo".to_string()
            }
            (_, Err(reason)) => format!("reprovado: {reason}"),
            _ => "não confiável".to_string(),
        });
        return st;
    }
    let Ok(id) = insp.identity else { return st };
    let exe = TrustedExe::new(id.canonical);

    st.version = parse::version(&host.run(&exe, &["--version"], QUICK).stdout);

    let (auth_args, parse_auth): (&[&str], fn(&str) -> AuthState) = match engine {
        EngineKind::Codex => (&["login", "status"], parse::auth_text),
        EngineKind::Claude => (&["auth", "status"], parse::auth_claude_json),
        EngineKind::Cursor => (&["status"], parse::auth_text),
    };
    let auth = host.run(&exe, auth_args, QUICK);
    st.auth = if auth.timed_out() {
        AuthState::Unknown
    } else {
        parse_auth(&auth.stdout)
    };

    if with_models {
        match engine {
            EngineKind::Codex => {
                let out = host.run(&exe, &["debug", "models"], SLOW);
                if out.timed_out() {
                    st.note = Some("timeout ao listar modelos".into());
                } else {
                    st.models = parse::codex_models(&out.stdout);
                }
            }
            EngineKind::Cursor => {
                let out = host.run(&exe, &["--list-models"], SLOW);
                if out.timed_out() {
                    st.note = Some("timeout ao listar modelos".into());
                } else {
                    st.models = parse::cursor_models(&out.stdout);
                }
            }
            EngineKind::Claude => {
                st.models = CLAUDE_ALIASES
                    .iter()
                    .map(|a| ModelInfo {
                        id: (*a).into(),
                        label: None,
                        source: ModelSource::Alias,
                        confirmed: false,
                    })
                    .collect();
                st.note = Some("Claude Code não lista modelos: apelidos não confirmados".into());
            }
        }
        if st.models.is_empty() && st.note.is_none() {
            st.note = Some("nenhum modelo retornado".into());
        }
    }
    st
}

/// Detecta as três CLIs em paralelo.
pub fn detect_all(host: &dyn Host, store: &dyn TrustStore, with_models: bool) -> Vec<EngineStatus> {
    let engines = [EngineKind::Codex, EngineKind::Claude, EngineKind::Cursor];
    std::thread::scope(|s| {
        let handles: Vec<_> = engines
            .iter()
            .map(|e| s.spawn(move || detect_engine(*e, host, store, with_models)))
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("thread de detecção"))
            .collect()
    })
}

/// Executável candidato a aprovação (para mostrar ao usuário antes de decidir).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub path: String,
    pub fingerprint: String,
    pub state: TrustState,
}

pub fn candidate(engine: EngineKind, host: &dyn Host, store: &dyn TrustStore) -> Option<Candidate> {
    let insp = host.inspect(binary_name(engine))?;
    let state = trust::evaluate(&insp, store, engine);
    let id = insp.identity.ok()?;
    Some(Candidate {
        path: id.canonical.display().to_string(),
        fingerprint: id.fingerprint,
        state,
    })
}

#[derive(Debug, PartialEq, Eq)]
pub enum ApproveError {
    NotFound,
    /// Reprovado nas verificações (dono/permissões/interpretador); não é aprovável.
    Rejected(String),
    /// O executável mudou entre o que o usuário viu e o momento de gravar.
    FingerprintMismatch,
    Store(String),
}

/// Torna o texto seguro para exibição: controles (quebra de linha), formatação invisível e
/// caracteres bidirecionais viram `\u{XXXX}`, e o tamanho é limitado. Um nome de arquivo hostil não
/// consegue fingir outra linha nem inverter a leitura do caminho no diálogo.
fn display_safe(s: &str) -> String {
    const MAX: usize = 300;
    let mut out = String::new();
    for (n, c) in s.chars().enumerate() {
        if n >= MAX {
            out.push('…');
            break;
        }
        let u = c as u32;
        let hidden = c.is_control()
            || matches!(u, 0x200B..=0x200F | 0x202A..=0x202E | 0x2060..=0x2064 | 0x2066..=0x2069 | 0xFEFF | 0x2028 | 0x2029);
        if hidden {
            out.push_str(&format!("\\u{{{u:04X}}}"));
        } else {
            out.push(c);
        }
    }
    out
}

/// Texto mostrado ao usuário antes de aprovar. Só informações verificáveis do executável: sem
/// variáveis de ambiente, sem conteúdo de arquivo, sem dados da conta.
pub fn approval_prompt(engine: EngineKind, c: &Candidate) -> String {
    format!(
        "Permitir que o OneAgent execute esta CLI?\n\nMotor: {}\nArquivo: {}\nImpressão digital (SHA-256): {}\nSituação: {}\n\n\
         Só aprove se você reconhece este arquivo.",
        binary_name(engine),
        display_safe(&c.path),
        c.fingerprint,
        match c.state {
            TrustState::Changed => "já aprovado antes, mas o arquivo mudou",
            _ => "nunca aprovado",
        }
    )
}

#[derive(Debug, PartialEq, Eq)]
pub enum ApprovalError {
    /// Executável ausente ou reprovado nas verificações (não é aprovável).
    Unavailable(String),
    /// A impressão digital mostrada ao usuário não é mais a do arquivo.
    Changed,
    /// O usuário recusou.
    Declined,
    Store(String),
}

/// Fluxo completo: confere o que a UI mostrou, pergunta ao usuário (`confirm` recebe o texto) e só
/// então grava, revalidando a identidade no momento da gravação. Já confiável = nada a fazer.
pub fn request_approval(
    engine: EngineKind,
    host: &dyn Host,
    store: &dyn TrustStore,
    expected: &str,
    confirm: impl FnOnce(&str) -> bool,
) -> Result<(), ApprovalError> {
    let cand = candidate(engine, host, store).ok_or_else(|| {
        ApprovalError::Unavailable("executável não encontrado ou reprovado nas verificações".into())
    })?;
    if cand.fingerprint != expected {
        return Err(ApprovalError::Changed);
    }
    if cand.state == TrustState::Trusted {
        return Ok(());
    }
    if !confirm(&approval_prompt(engine, &cand)) {
        return Err(ApprovalError::Declined);
    }
    approve_engine(engine, host, store, expected).map_err(|e| match e {
        ApproveError::NotFound => ApprovalError::Unavailable("executável não encontrado".into()),
        ApproveError::Rejected(r) => ApprovalError::Unavailable(format!("reprovado: {r}")),
        ApproveError::FingerprintMismatch => ApprovalError::Changed,
        ApproveError::Store(e) => ApprovalError::Store(e),
    })
}

/// Grava a aprovação SÓ se a identidade atual for exatamente a que o usuário viu (`expected`).
pub fn approve_engine(
    engine: EngineKind,
    host: &dyn Host,
    store: &dyn TrustStore,
    expected: &str,
) -> Result<(), ApproveError> {
    let insp = host
        .inspect(binary_name(engine))
        .ok_or(ApproveError::NotFound)?;
    let id = insp.identity.map_err(ApproveError::Rejected)?;
    if id.fingerprint != expected {
        return Err(ApproveError::FingerprintMismatch);
    }
    store
        .approve(engine, &id.fingerprint, &id.canonical.display().to_string())
        .map_err(ApproveError::Store)
}

/// Descreve o "Gateway local" (este Mac) com as CLIs detectadas. Na Fase 1 roda dentro do app;
/// na Fase 2 o mesmo contrato passa a ser servido pelo binário do Gateway.
pub fn local_gateway(host: &dyn Host, store: &dyn TrustStore, with_models: bool) -> GatewayInfo {
    GatewayInfo {
        id: "local".into(),
        name: "Este Mac".into(),
        kind: GatewayKind::Local,
        status: GatewayStatus::Online,
        os: format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
        protocol_version: PROTOCOL_VERSION,
        engines: detect_all(host, store, with_models),
        demo: false,
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::trust::tests::{script, tmp, MemStore};
    use super::*;

    /// Host falso: respostas por "binário args"; conta execuções para provar que nada roda sem confiança.
    struct Fake {
        bins: HashMap<&'static str, Result<Identity, String>>,
        out: HashMap<String, RunOutput>,
        runs: AtomicUsize,
    }

    fn identity(name: &str) -> Identity {
        Identity {
            canonical: PathBuf::from(format!("/x/{name}")),
            interpreter: None,
            sha256: "h".into(),
            owner_uid: 0,
            fingerprint: format!("fp-{name}"),
        }
    }

    impl Fake {
        fn new(bins: &[&'static str], out: HashMap<String, RunOutput>) -> Self {
            let bins = bins.iter().map(|b| (*b, Ok(identity(b)))).collect();
            Self {
                bins,
                out,
                runs: AtomicUsize::new(0),
            }
        }
        fn trusting(&self, store: &MemStore, engine: EngineKind, bin: &str) {
            store.approve(engine, &format!("fp-{bin}"), "x").unwrap();
        }
    }

    impl Host for Fake {
        fn inspect(&self, name: &str) -> Option<Inspected> {
            self.bins.get(name).map(|r| Inspected {
                found: PathBuf::from(format!("/found/{name}")),
                identity: r.clone(),
            })
        }
        fn run(&self, exe: &TrustedExe, args: &[&str], _l: Limits) -> RunOutput {
            self.runs.fetch_add(1, Ordering::SeqCst);
            let key = format!(
                "{} {}",
                exe.path().file_name().unwrap().to_string_lossy(),
                args.join(" ")
            );
            self.out.get(&key).cloned().unwrap_or(RunOutput {
                stdout: String::new(),
                outcome: Outcome::TimedOut,
            })
        }
    }

    fn ok(s: &str) -> RunOutput {
        RunOutput {
            stdout: s.into(),
            outcome: Outcome::Exited(Some(0)),
        }
    }

    #[test]
    fn missing_cli_is_reported_not_installed() {
        let h = Fake::new(&[], HashMap::new());
        let st = detect_engine(EngineKind::Codex, &h, &MemStore::default(), true);
        assert!(!st.installed);
        assert_eq!(st.auth, AuthState::Unknown);
        assert!(st.models.is_empty());
    }

    #[test]
    fn unknown_executable_is_found_but_never_executed() {
        let h = Fake::new(&["codex"], HashMap::new());
        let st = detect_engine(EngineKind::Codex, &h, &MemStore::default(), true);
        assert!(st.installed);
        assert_eq!(st.trust, TrustState::Unknown);
        assert_eq!(st.fingerprint.as_deref(), Some("fp-codex"));
        assert!(st.version.is_none() && st.models.is_empty());
        assert_eq!(
            h.runs.load(Ordering::SeqCst),
            0,
            "nada pode executar antes da aprovação"
        );
    }

    #[test]
    fn changed_and_rejected_executables_are_not_executed() {
        let store = MemStore::default();
        store.approve(EngineKind::Codex, "fp-antigo", "x").unwrap();
        let mut h = Fake::new(&["codex"], HashMap::new());
        let st = detect_engine(EngineKind::Codex, &h, &store, true);
        assert_eq!(st.trust, TrustState::Changed);
        h.bins
            .insert("codex", Err("gravável por qualquer usuário".into()));
        let st = detect_engine(EngineKind::Codex, &h, &store, true);
        assert_eq!(st.trust, TrustState::Rejected);
        assert!(st.note.unwrap().contains("reprovado"));
        assert_eq!(h.runs.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn trusted_codex_lists_visible_models_only() {
        let mut out = HashMap::new();
        out.insert("codex --version".into(), ok("codex-cli 0.169.0\n"));
        out.insert("codex login status".into(), ok("Logged in using ChatGPT\n"));
        out.insert(
            "codex debug models".into(),
            ok(r#"{"models":[{"slug":"a","display_name":"A","visibility":"list"},{"slug":"b","visibility":"hide"}]}"#),
        );
        let h = Fake::new(&["codex"], out);
        let store = MemStore::default();
        h.trusting(&store, EngineKind::Codex, "codex");
        let st = detect_engine(EngineKind::Codex, &h, &store, true);
        assert_eq!(st.trust, TrustState::Trusted);
        assert_eq!(st.version.as_deref(), Some("0.169.0"));
        assert_eq!(st.auth, AuthState::LoggedIn);
        assert_eq!(st.models.len(), 1);
        assert!(st.models[0].confirmed);
    }

    #[test]
    fn claude_models_are_unconfirmed_aliases_and_email_is_never_kept() {
        let mut out = HashMap::new();
        out.insert("claude --version".into(), ok("2.1.326 (Claude Code)\n"));
        out.insert(
            "claude auth status".into(),
            ok(r#"{"loggedIn":true,"email":"secret@example.com"}"#),
        );
        let h = Fake::new(&["claude"], out);
        let store = MemStore::default();
        h.trusting(&store, EngineKind::Claude, "claude");
        let st = detect_engine(EngineKind::Claude, &h, &store, true);
        assert_eq!(st.auth, AuthState::LoggedIn);
        assert!(st
            .models
            .iter()
            .all(|m| !m.confirmed && m.source == ModelSource::Alias));
        assert!(!serde_json::to_string(&st)
            .unwrap()
            .contains("secret@example.com"));
    }

    #[test]
    fn timeout_degrades_gracefully() {
        let h = Fake::new(&["cursor-agent"], HashMap::new());
        let store = MemStore::default();
        h.trusting(&store, EngineKind::Cursor, "cursor-agent");
        let st = detect_engine(EngineKind::Cursor, &h, &store, true);
        assert!(st.installed);
        assert_eq!(st.auth, AuthState::Unknown);
        assert!(st.note.as_deref().unwrap().contains("timeout"));
    }

    #[test]
    fn skipping_models_skips_slow_calls() {
        let mut out = HashMap::new();
        out.insert("cursor-agent --version".into(), ok("2026.11.13-eacade1\n"));
        out.insert("cursor-agent status".into(), ok("Not logged in\n"));
        let h = Fake::new(&["cursor-agent"], out);
        let store = MemStore::default();
        h.trusting(&store, EngineKind::Cursor, "cursor-agent");
        let st = detect_engine(EngineKind::Cursor, &h, &store, false);
        assert_eq!(st.auth, AuthState::LoggedOut);
        assert!(st.models.is_empty() && st.note.is_none());
    }

    #[test]
    fn local_gateway_lists_three_engines_even_when_none_installed() {
        let h = Fake::new(&[], HashMap::new());
        let gw = local_gateway(&h, &MemStore::default(), false);
        assert_eq!(gw.id, "local");
        assert!(!gw.demo);
        assert_eq!(gw.engines.len(), 3);
        assert!(gw.engines.iter().all(|e| !e.installed));
    }

    #[test]
    fn approve_requires_matching_fingerprint_and_rejects_unapprovable() {
        let store = MemStore::default();
        let mut h = Fake::new(&["codex"], HashMap::new());
        assert_eq!(
            approve_engine(EngineKind::Codex, &h, &store, "fp-outro"),
            Err(ApproveError::FingerprintMismatch)
        );
        assert_eq!(store.approved_fingerprint(EngineKind::Codex), None);
        assert_eq!(
            approve_engine(EngineKind::Codex, &h, &store, "fp-codex"),
            Ok(())
        );
        assert_eq!(
            store.approved_fingerprint(EngineKind::Codex).as_deref(),
            Some("fp-codex")
        );
        h.bins.insert("codex", Err("dono diferente".into()));
        assert!(matches!(
            approve_engine(EngineKind::Codex, &h, &store, "fp-codex"),
            Err(ApproveError::Rejected(_))
        ));
        assert_eq!(
            approve_engine(EngineKind::Claude, &h, &store, "x"),
            Err(ApproveError::NotFound)
        );
    }

    /// Ponta a ponta com arquivos reais: o marcador só aparece se o script for de fato executado.
    #[test]
    fn real_binary_runs_only_after_approval_and_not_after_replacement() {
        let d = tmp("e2e");
        let marker = d.join("MARKER");
        let bin = d.join("codex");
        let body = |extra: &str| {
            format!(
                "#!/bin/sh\ntouch '{}'\n{extra}\necho 'codex-cli 9.9.9'\n",
                marker.display()
            )
        };
        script(&bin, &body(""), 0o755);
        let host = SystemHost::with_dirs(vec![
            d.clone(),
            PathBuf::from("/bin"),
            PathBuf::from("/usr/bin"),
        ]);
        let store = MemStore::default();

        let st = detect_engine(EngineKind::Codex, &host, &store, false);
        assert_eq!(st.trust, TrustState::Unknown);
        assert!(
            !Path::new(&marker).exists(),
            "binário desconhecido NÃO pode ser executado"
        );

        let fp = st.fingerprint.unwrap();
        approve_engine(EngineKind::Codex, &host, &store, &fp).unwrap();
        let st = detect_engine(EngineKind::Codex, &host, &store, false);
        assert_eq!(st.trust, TrustState::Trusted);
        assert_eq!(st.version.as_deref(), Some("9.9.9"));
        assert!(marker.exists());

        std::fs::remove_file(&marker).unwrap();
        script(&bin, &body("echo substituido"), 0o755);
        let st = detect_engine(EngineKind::Codex, &host, &store, false);
        assert_eq!(st.trust, TrustState::Changed);
        assert!(
            !marker.exists(),
            "binário substituído NÃO pode ser executado"
        );
    }

    #[test]
    fn approval_flow_prompt_is_verifiable_and_decline_blocks() {
        let store = MemStore::default();
        let h = Fake::new(&["codex"], HashMap::new());
        let mut shown = String::new();
        let r = request_approval(EngineKind::Codex, &h, &store, "fp-codex", |m| {
            shown = m.to_string();
            false
        });
        assert_eq!(r, Err(ApprovalError::Declined));
        assert_eq!(
            store.approved_fingerprint(EngineKind::Codex),
            None,
            "recusa não grava"
        );
        assert!(
            shown.contains("/x/codex")
                && shown.contains("fp-codex")
                && shown.contains("nunca aprovado")
        );
        assert!(!shown.contains("HOME") && !shown.contains("TOKEN"));
        // aprovar grava; depois já é confiável e não pergunta de novo
        assert_eq!(
            request_approval(EngineKind::Codex, &h, &store, "fp-codex", |_| true),
            Ok(())
        );
        assert_eq!(
            store.approved_fingerprint(EngineKind::Codex).as_deref(),
            Some("fp-codex")
        );
        assert_eq!(
            request_approval(EngineKind::Codex, &h, &store, "fp-codex", |_| panic!(
                "não deve perguntar"
            )),
            Ok(())
        );
    }

    #[test]
    fn approval_flow_rejects_stale_fingerprint_and_unapprovable() {
        let store = MemStore::default();
        let mut h = Fake::new(&["codex"], HashMap::new());
        let r = request_approval(
            EngineKind::Codex,
            &h,
            &store,
            "fp-que-o-usuario-viu",
            |_| panic!("não deve perguntar"),
        );
        assert_eq!(r, Err(ApprovalError::Changed));
        h.bins
            .insert("codex", Err("tem ACL que concede escrita".into()));
        let r = request_approval(EngineKind::Codex, &h, &store, "fp-codex", |_| {
            panic!("não deve perguntar")
        });
        assert!(matches!(r, Err(ApprovalError::Unavailable(_))));
        assert_eq!(store.approved_fingerprint(EngineKind::Codex), None);
    }

    /// Um `bash` plantado em pasta do usuário não pode sombrear o do sistema: a resolução do
    /// interpretador começa pelos diretórios do sistema.
    #[test]
    fn system_interpreter_wins_over_user_dir_shadowing() {
        let d = tmp("shadow");
        script(&d.join("bash"), "#!/bin/sh\n", 0o755); // "bash" falso na pasta do usuário
        let bin = d.join("cursor-agent");
        script(&bin, "#!/usr/bin/env bash\necho x\n", 0o755);
        let host = SystemHost::with_dirs(vec![d.clone()]);
        let id = host.inspect("cursor-agent").unwrap().identity.unwrap();
        let interp = id.interpreter.unwrap();
        assert!(
            interp.starts_with("/bin") || interp.starts_with("/usr/bin"),
            "{interp:?}"
        );
    }

    #[test]
    fn approval_prompt_neutralizes_hostile_paths() {
        let c = Candidate {
            path: "/tmp/ok\nImpressão digital (SHA-256): 0000\u{202E}gpj.exe\u{200B}".into(),
            fingerprint: "ab".repeat(32),
            state: TrustState::Unknown,
        };
        let m = approval_prompt(EngineKind::Codex, &c);
        let path_line = m.lines().find(|l| l.starts_with("Arquivo: ")).unwrap();
        assert!(
            path_line.contains("\\u{000A}")
                && path_line.contains("\\u{202E}")
                && path_line.contains("\\u{200B}"),
            "{path_line}"
        );
        assert_eq!(
            m.matches("Impressão digital").count(),
            2,
            "uma linha real + o texto escapado dentro do caminho"
        );
        assert_eq!(
            m.lines()
                .filter(|l| l.starts_with("Impressão digital"))
                .count(),
            1,
            "nenhuma linha falsa"
        );
        assert!(!m.contains('\u{202E}') && !m.contains('\u{200B}'));
        let long = Candidate {
            path: "a".repeat(5000),
            ..c
        };
        assert!(approval_prompt(EngineKind::Codex, &long).len() < 1000);
    }

    /// Manual: `cargo test -p one-adapters -- --ignored --nocapture real_clis`.
    /// Confirma que as instalações reais (npm/nvm, `~/.local`) passam nas verificações.
    #[test]
    #[ignore]
    fn real_clis_pass_the_trust_checks() {
        let host = SystemHost::new();
        for name in ["codex", "claude", "cursor-agent"] {
            let insp = host
                .inspect(name)
                .unwrap_or_else(|| panic!("{name} não encontrado"));
            match &insp.identity {
                Ok(id) => println!(
                    "{name}: ok  {} -> {}",
                    insp.found.display(),
                    id.canonical.display()
                ),
                Err(e) => panic!("{name} reprovado: {e}"),
            }
        }
    }
}
