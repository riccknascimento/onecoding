//! Pipeline do Codex com um binário falso (sem chamada de IA): aprovação, argumentos confinados, stdin e parser.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use one_adapters::{approve_engine, candidate, SystemHost};
use one_exec::StoreTrust;
use one_executor::codex::run_codex;
use one_protocol::EngineKind;
use one_store::Store;

fn fake_codex(dir: &Path) {
    let script = format!(
        "#!/bin/sh\necho \"$@\" > '{d}/argv.txt'\ncat > '{d}/stdin.txt'\n\
         echo '{{\"type\":\"thread.started\",\"thread_id\":\"t-fake\"}}'\n\
         echo '{{\"type\":\"item.completed\",\"item\":{{\"type\":\"agent_message\",\"text\":\"Resposta de teste com evidência.\"}}}}'\n\
         echo '{{\"type\":\"turn.completed\",\"usage\":{{\"input_tokens\":3,\"output_tokens\":2}}}}'\n",
        d = dir.display()
    );
    let p = dir.join("codex");
    std::fs::write(&p, script).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn setup(name: &str, approve: bool) -> (PathBuf, SystemHost, StoreTrust) {
    let base = std::fs::canonicalize(std::env::temp_dir())
        .unwrap()
        .join(format!("oa-codexfake-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("bin")).unwrap();
    std::fs::create_dir_all(base.join("snapshot")).unwrap();
    std::fs::write(base.join("snapshot/README.md"), "Projeto de teste").unwrap();
    fake_codex(&base.join("bin"));
    let host = SystemHost::with_dirs(vec![base.join("bin"), "/bin".into(), "/usr/bin".into()]);
    let trust = StoreTrust(Arc::new(Mutex::new(Store::open_in_memory().unwrap())));
    if approve {
        let c = candidate(EngineKind::Codex, &host, &trust).expect("codex falso encontrado");
        approve_engine(EngineKind::Codex, &host, &trust, &c.fingerprint).unwrap();
    }
    (base, host, trust)
}

#[test]
fn approved_codex_runs_on_the_snapshot_with_prompt_on_stdin_and_returns_the_answer() {
    let (base, host, trust) = setup("ok", true);
    let answer = run_codex(
        &host,
        &trust,
        &base.join("snapshot"),
        "O que o README diz?",
        &AtomicBool::new(false),
        &mut |_, _| {},
    )
    .unwrap();
    assert_eq!(answer, "Resposta de teste com evidência.");
    // a pergunta vai pelo stdin, não pelos argumentos
    let stdin = std::fs::read_to_string(base.join("bin/stdin.txt")).unwrap();
    assert!(stdin.contains("O que o README diz?"));
    let argv = std::fs::read_to_string(base.join("bin/argv.txt")).unwrap();
    assert!(
        !argv.contains("O que o README diz?"),
        "pergunta nunca vai em argv"
    );
    assert!(
        argv.contains("default_permissions=\"oa_workspace_ro\""),
        "perfil confinado do Codex: {argv}"
    );
    assert!(
        !argv.contains("danger") && !argv.contains("workspace-write"),
        "{argv}"
    );
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn unapproved_codex_is_refused_before_any_execution() {
    let (base, host, trust) = setup("naoaprovado", false);
    let err = run_codex(
        &host,
        &trust,
        &base.join("snapshot"),
        "pergunta",
        &AtomicBool::new(false),
        &mut |_, _| {},
    )
    .unwrap_err();
    assert!(err.contains("não aprovado"), "{err}");
    assert!(
        !base.join("bin/argv.txt").exists(),
        "nada pode executar sem aprovação"
    );
    let _ = std::fs::remove_dir_all(&base);
}

/// Codex falso com um roteiro próprio (linhas de saída e um comando final), aprovado como os demais.
fn setup_script(name: &str, script_tail: &str) -> (PathBuf, SystemHost, StoreTrust) {
    let base = std::fs::canonicalize(std::env::temp_dir())
        .unwrap()
        .join(format!("oa-codexfake-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("bin")).unwrap();
    std::fs::create_dir_all(base.join("snapshot")).unwrap();
    std::fs::write(base.join("snapshot/README.md"), "Projeto de teste").unwrap();
    let script = format!("#!/bin/sh\ncat > /dev/null\n{script_tail}\n");
    let p = base.join("bin/codex");
    std::fs::write(&p, script).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    let host = SystemHost::with_dirs(vec![base.join("bin"), "/bin".into(), "/usr/bin".into()]);
    let trust = StoreTrust(Arc::new(Mutex::new(Store::open_in_memory().unwrap())));
    let c = candidate(EngineKind::Codex, &host, &trust).expect("codex falso encontrado");
    approve_engine(EngineKind::Codex, &host, &trust, &c.fingerprint).unwrap();
    (base, host, trust)
}

#[test]
fn codex_tool_progress_is_forwarded_and_the_answer_stays_separate() {
    let (base, host, trust) = setup_script(
        "progresso",
        "echo '{\"type\":\"thread.started\",\"thread_id\":\"t\"}'\n\
         echo '{\"type\":\"item.started\",\"item\":{\"type\":\"command_execution\",\"command\":\"cat README.md\"}}'\n\
         echo '{\"type\":\"item.completed\",\"item\":{\"type\":\"command_execution\",\"exit_code\":0}}'\n\
         echo '{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"Resposta final.\"}}'",
    );
    let mut seen: Vec<(String, serde_json::Value)> = Vec::new();
    let answer = run_codex(
        &host,
        &trust,
        &base.join("snapshot"),
        "pergunta",
        &AtomicBool::new(false),
        &mut |kind, payload| seen.push((kind.to_string(), payload)),
    )
    .unwrap();
    assert_eq!(answer, "Resposta final.");
    assert_eq!(
        seen.len(),
        1,
        "só o progresso de ferramenta é encaminhado: {seen:?}"
    );
    assert_eq!(seen[0].0, "tool");
    assert_eq!(seen[0].1["summary"], "cat README.md");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn cancel_flag_stops_the_codex_process_group_promptly() {
    let (base, host, trust) = setup_script("cancelar", "sleep 30");
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(300));
        flag.store(true, std::sync::atomic::Ordering::SeqCst);
    });
    let started = std::time::Instant::now();
    let err = run_codex(
        &host,
        &trust,
        &base.join("snapshot"),
        "pergunta",
        &cancel,
        &mut |_, _| {},
    )
    .unwrap_err();
    assert_eq!(err, "cancelado");
    assert!(
        started.elapsed() < std::time::Duration::from_secs(10),
        "encerrado sem esperar o sleep"
    );
    let _ = std::fs::remove_dir_all(&base);
}
