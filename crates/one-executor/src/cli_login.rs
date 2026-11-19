//! Login oficial das CLIs executado pelo OneExecutor, iniciado pelo OneCoding Web.
//!
//! Regras: o processo roda sob o usuário do executor, sem shell, com ambiente limpo e sem abrir navegador.
//! Só links HTTPS de domínios oficiais e códigos de dispositivo chegam ao Central. Nenhum token, cookie ou
//! conteúdo de credencial é lido, guardado ou enviado: a CLI grava as próprias credenciais no seu diretório.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    Codex,
    Claude,
    Cursor,
}

impl Engine {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "codex" => Some(Engine::Codex),
            "claude" => Some(Engine::Claude),
            "cursor" => Some(Engine::Cursor),
            _ => None,
        }
    }

    /// Nome do binário procurado no PATH do executor (nunca caminho vindo do Central).
    pub fn binary(self) -> &'static str {
        match self {
            Engine::Codex => "codex",
            Engine::Claude => "claude",
            Engine::Cursor => "cursor-agent",
        }
    }

    pub fn login_args(self) -> &'static [&'static str] {
        match self {
            Engine::Codex => &["login", "--device-auth"],
            Engine::Claude => &["auth", "login", "--claudeai"],
            Engine::Cursor => &["login"],
        }
    }

    pub fn status_args(self) -> &'static [&'static str] {
        match self {
            Engine::Codex => &["login", "status"],
            Engine::Claude => &["auth", "status"],
            Engine::Cursor => &["status"],
        }
    }

    pub fn logout_args(self) -> &'static [&'static str] {
        match self {
            Engine::Codex => &["logout"],
            Engine::Claude => &["auth", "logout"],
            Engine::Cursor => &["logout"],
        }
    }

    /// Domínios aceitos para o link de autorização. Qualquer outro link é descartado.
    pub fn allowed_hosts(self) -> &'static [&'static str] {
        match self {
            Engine::Codex => &["auth.openai.com"],
            Engine::Claude => &["claude.ai", "console.anthropic.com"],
            Engine::Cursor => &["cursor.com"],
        }
    }

    /// Claude Code pede para colar na CLI o código mostrado no navegador. Os outros só aguardam a aprovação.
    pub fn needs_pasted_code(self) -> bool {
        matches!(self, Engine::Claude)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Hint {
    /// Link oficial para o usuário abrir no próprio navegador.
    Url(String),
    /// Código curto de dispositivo (formato XXXX-XXXX), mostrado ao usuário.
    DeviceCode(String),
    /// A CLI está aguardando um código colado pelo usuário.
    AwaitingPastedCode,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// O processo de login terminou com sucesso. A autenticação real ainda precisa ser confirmada pelo status.
    Exited,
    Failed(String),
    TimedOut,
    Cancelled,
}

/// Remove sequências de controle ANSI (cor e hiperlink OSC 8) antes de procurar links e códigos.
pub fn strip_ansi(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            if !c.is_control() || c == '\t' {
                out.push(c);
            }
            continue;
        }
        match chars.peek() {
            Some('[') => {
                chars.next();
                for d in chars.by_ref() {
                    if d.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            Some(']') => {
                chars.next();
                // OSC termina em BEL ou ESC \
                while let Some(d) = chars.next() {
                    if d == '\u{7}' {
                        break;
                    }
                    if d == '\u{1b}' {
                        chars.next();
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// Extrai dicas de uma linha de saída da CLI. Não guarda nada além do que é devolvido.
pub fn parse_hints(engine: Engine, raw: &str) -> Vec<Hint> {
    let line = strip_ansi(raw);
    let mut hints = Vec::new();
    for token in line.split_whitespace() {
        if let Some(url) = official_url(engine, token) {
            hints.push(Hint::Url(url));
        }
    }
    if engine == Engine::Codex {
        for token in line.split(|c: char| !c.is_ascii_alphanumeric() && c != '-') {
            if is_device_code(token) {
                hints.push(Hint::DeviceCode(token.to_string()));
            }
        }
    }
    let lower = line.to_lowercase();
    if engine.needs_pasted_code() && (lower.contains("paste code") || lower.contains("cole o código")) {
        hints.push(Hint::AwaitingPastedCode);
    }
    hints
}

fn official_url(engine: Engine, token: &str) -> Option<String> {
    let rest = token.strip_prefix("https://")?;
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let host = &rest[..end];
    if !engine.allowed_hosts().contains(&host) {
        return None;
    }
    // remove pontuação final colada ao link (ex.: "...ponto.)")
    let clean = token.trim_end_matches(['.', ',', ';', ')', '"', '\'']);
    if clean.len() > 2048 || clean.chars().any(|c| c.is_control()) {
        return None;
    }
    Some(clean.to_string())
}

fn is_device_code(token: &str) -> bool {
    let parts: Vec<&str> = token.split('-').collect();
    parts.len() == 2
        && (4..=5).contains(&parts[0].len())
        && (4..=5).contains(&parts[1].len())
        && parts.iter().all(|p| p.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()))
}

/// Executa o login oficial e reporta dicas à medida que a CLI as imprime.
/// `pasted` recebe o código colado pelo usuário (Claude) e o repassa à entrada padrão da CLI.
pub fn run_login(
    engine: Engine,
    bin: &Path,
    home: &Path,
    timeout: Duration,
    cancel: &AtomicBool,
    pasted: Option<Receiver<String>>,
    on_hint: &mut dyn FnMut(Hint),
) -> Outcome {
    let mut child = match Command::new(bin)
        .args(engine.login_args())
        .env_clear()
        .env("HOME", home)
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("NO_OPEN_BROWSER", "1")
        .env("BROWSER", "true")
        .env("TERM", "dumb")
        .stdin(if pasted.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(_) => return Outcome::Failed("CLI indisponível para iniciar o login".into()),
    };

    let (tx, rx) = mpsc::channel::<String>();
    for stream in [
        child.stdout.take().map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
        child.stderr.take().map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
    ]
    .into_iter()
    .flatten()
    {
        let tx = tx.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stream).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
    }
    drop(tx);
    let mut stdin = child.stdin.take();
    let deadline = Instant::now() + timeout;
    let mut seen: Vec<Hint> = Vec::new();

    loop {
        if cancel.load(Ordering::SeqCst) {
            let _ = child.kill();
            let _ = child.wait();
            return Outcome::Cancelled;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Outcome::TimedOut;
        }
        if let Some(rx_pasted) = &pasted {
            while let Ok(code) = rx_pasted.try_recv() {
                if let Some(w) = stdin.as_mut() {
                    let _ = writeln!(w, "{code}");
                    let _ = w.flush();
                }
            }
        }
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(line) => {
                for hint in parse_hints(engine, &line) {
                    if !seen.contains(&hint) {
                        seen.push(hint.clone());
                        on_hint(hint);
                    }
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                // leitores terminaram: espera o fim do processo
                return match child.wait() {
                    Ok(status) if status.success() => Outcome::Exited,
                    Ok(_) => Outcome::Failed("login não concluído pela CLI".into()),
                    Err(_) => Outcome::Failed("falha ao aguardar a CLI".into()),
                };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn only_official_https_links_are_extracted() {
        let codex = parse_hints(Engine::Codex, "abra https://auth.openai.com/codex/device agora");
        assert_eq!(codex, vec![Hint::Url("https://auth.openai.com/codex/device".into())]);
        // domínio fora da lista, http e link disfarçado são descartados
        assert!(parse_hints(Engine::Codex, "https://evil.example/codex/device").is_empty());
        assert!(parse_hints(Engine::Codex, "http://auth.openai.com/codex/device").is_empty());
        assert!(parse_hints(Engine::Codex, "https://auth.openai.com.evil.example/x").is_empty());
        // pontuação final não entra no link
        let c = parse_hints(Engine::Cursor, "Abra: https://cursor.com/loginDeepControl?x=1.");
        assert_eq!(c, vec![Hint::Url("https://cursor.com/loginDeepControl?x=1".into())]);
    }

    #[test]
    fn device_code_is_found_inside_ansi_colour_codes() {
        let line = "\u{1b}[94mABCD-1234\u{1b}[0m";
        assert_eq!(parse_hints(Engine::Codex, line), vec![Hint::DeviceCode("ABCD-1234".into())]);
        // outras CLIs não tratam padrão de código como código de dispositivo
        assert!(parse_hints(Engine::Cursor, "ABCD-1234").is_empty());
    }

    #[test]
    fn osc_hyperlinks_are_removed_before_matching() {
        let line = "\u{1b}]8;;https://claude.ai/oauth/x\u{7}https://claude.ai/oauth/x\u{1b}]8;;\u{7}";
        let hints = parse_hints(Engine::Claude, line);
        assert!(hints.iter().any(|h| matches!(h, Hint::Url(u) if u.starts_with("https://claude.ai/"))));
    }

    #[test]
    fn claude_paste_prompt_is_reported() {
        assert!(parse_hints(Engine::Claude, "Paste code here if prompted >")
            .contains(&Hint::AwaitingPastedCode));
    }

    #[test]
    fn unknown_engines_are_refused_and_binaries_are_fixed_names() {
        assert_eq!(Engine::parse("bash"), None);
        assert_eq!(Engine::parse("cursor"), Some(Engine::Cursor));
        assert_eq!(Engine::Cursor.binary(), "cursor-agent");
    }

    fn fake_cli(dir: &Path, body: &str) -> std::path::PathBuf {
        let p = dir.join("fake-cli");
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    #[test]
    fn login_reports_hints_then_exits_successfully() {
        let dir = std::env::temp_dir().join(format!("oneagent-login-ok-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let bin = fake_cli(&dir, "echo 'abra https://auth.openai.com/codex/device'; echo 'codigo WXYZ-9876'; exit 0");
        let cancel = AtomicBool::new(false);
        let mut seen = Vec::new();
        let out = run_login(Engine::Codex, &bin, &dir, Duration::from_secs(10), &cancel, None, &mut |h| seen.push(h));
        assert_eq!(out, Outcome::Exited);
        assert!(seen.contains(&Hint::DeviceCode("WXYZ-9876".into())));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn login_times_out_and_the_process_is_killed() {
        let dir = std::env::temp_dir().join(format!("oneagent-login-to-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let bin = fake_cli(&dir, "exec sleep 30");
        let cancel = AtomicBool::new(false);
        let started = Instant::now();
        let out = run_login(Engine::Codex, &bin, &dir, Duration::from_millis(400), &cancel, None, &mut |_| {});
        assert_eq!(out, Outcome::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(10));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cancel_stops_a_waiting_login_immediately() {
        let dir = std::env::temp_dir().join(format!("oneagent-login-cancel-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let bin = fake_cli(&dir, "exec sleep 30");
        let cancel = std::sync::Arc::new(AtomicBool::new(false));
        let c2 = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            c2.store(true, Ordering::SeqCst);
        });
        let started = Instant::now();
        let out = run_login(Engine::Codex, &bin, &dir, Duration::from_secs(60), &cancel, None, &mut |_| {});
        assert_eq!(out, Outcome::Cancelled);
        assert!(started.elapsed() < Duration::from_secs(10));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
