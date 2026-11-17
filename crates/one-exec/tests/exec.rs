//! Pipeline completo com um `codex` falso (script real): aprovação, streaming de eventos,
//! cancelamento, timeout, trava por workspace e persistência. Nada aqui chama o Codex de verdade.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use one_adapters::proc::{Limits, RunOutput};
use one_adapters::{
    approve_engine, candidate, Host, Inspected, StreamSpec, SystemHost, TrustedExe,
};
use one_exec::{ExecLimits, ExecService, StoreTrust};
use one_protocol::{EngineKind, ErrorKind, EventPayload, RunStatus, RunView, StartRunRequest};
use one_store::Store;

struct Env {
    dir: PathBuf,
    ws: PathBuf,
    ws_id: String,
    store: Arc<Mutex<Store>>,
    svc: Arc<ExecService>,
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../one-adapters/tests/fixtures/codex")
        .join(name)
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("oa-exec-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::canonicalize(&d).unwrap()
}

fn fake_codex(dir: &Path, body: &str) {
    let script = format!(
        "#!/bin/sh\necho \"$@\" > '{d}/argv.txt'\ncat > '{d}/stdin.txt'\npwd > '{d}/cwd.txt'\n{body}\n",
        d = dir.display()
    );
    let p = dir.join("codex");
    std::fs::write(&p, script).unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn host(dir: &Path) -> Arc<SystemHost> {
    Arc::new(SystemHost::with_dirs(vec![
        dir.to_path_buf(),
        "/bin".into(),
        "/usr/bin".into(),
    ]))
}

fn setup(name: &str, body: &str, limits: ExecLimits, approve: bool) -> Env {
    let dir = tmp(name);
    let ws = dir.join("workspace");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(ws.join("README.md"), "# demo\n").unwrap();
    fake_codex(&dir, body);
    let store = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
    store.lock().unwrap().ensure_local_gateway().unwrap();
    let ws_id = store
        .lock()
        .unwrap()
        .workspace_add(
            "local",
            &std::fs::canonicalize(&ws).unwrap().display().to_string(),
            "workspace",
        )
        .unwrap()
        .id;
    let h = host(&dir);
    if approve {
        let trust = StoreTrust(store.clone());
        let c = candidate(EngineKind::Codex, h.as_ref(), &trust).expect("fake codex descoberto");
        approve_engine(EngineKind::Codex, h.as_ref(), &trust, &c.fingerprint).unwrap();
    }
    let svc = ExecService::new(store.clone(), h, limits);
    Env {
        dir,
        ws,
        ws_id,
        store,
        svc,
    }
}

fn req(env: &Env) -> StartRunRequest {
    StartRunRequest {
        workspace_id: env.ws_id.clone(),
        engine: EngineKind::Codex,
        model: None,
        prompt: "Resuma o projeto".into(),
    }
}

fn wait_final(env: &Env, run_id: &str) -> RunView {
    let t = Instant::now();
    loop {
        let v = env.store.lock().unwrap().run_get(run_id).unwrap().unwrap();
        if !matches!(v.status, RunStatus::Queued | RunStatus::Running) {
            // a thread libera a trava logo depois; espera para o teste seguinte não correr com ela
            while env.svc.is_active(run_id) && t.elapsed() < Duration::from_secs(20) {
                std::thread::sleep(Duration::from_millis(10));
            }
            return v;
        }
        assert!(
            t.elapsed() < Duration::from_secs(20),
            "execução não terminou: {v:?}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn events(env: &Env, run_id: &str) -> Vec<EventPayload> {
    env.store
        .lock()
        .unwrap()
        .events_after(run_id, 0)
        .unwrap()
        .into_iter()
        .map(|e| e.payload)
        .collect()
}

fn alive(pid: i32) -> bool {
    // SAFETY: sinal 0 só verifica existência.
    unsafe { libc_kill(pid, 0) == 0 }
}
extern "C" {
    #[link_name = "kill"]
    fn libc_kill(pid: i32, sig: i32) -> i32;
}

fn ok_body() -> String {
    format!("cat '{}'", fixture("success.jsonl").display())
}

const FAST: ExecLimits = ExecLimits {
    timeout: Duration::from_secs(30),
    max_output: 1 << 20,
};

#[test]
fn unapproved_cli_is_blocked_before_any_execution_and_audited() {
    let env = setup("unapproved", &ok_body(), FAST, false);
    let e = env.svc.start(&req(&env)).unwrap_err();
    assert_eq!(e.kind, ErrorKind::NotApproved);
    assert!(
        !env.dir.join("argv.txt").exists(),
        "o binário não aprovado NÃO pode rodar"
    );
    assert!(
        env.store.lock().unwrap().runs_list(10).unwrap().is_empty(),
        "nada de run para pedido bloqueado"
    );
    let n: i64 = env
        .store
        .lock()
        .unwrap()
        .conn()
        .query_row(
            "SELECT count(*) FROM audit_log WHERE action='run_blocked'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 1);
}

#[test]
fn approved_cli_runs_read_only_streams_events_and_records_metrics() {
    let env = setup("success", &ok_body(), FAST, true);
    let id = env.svc.start(&req(&env)).unwrap();
    let v = wait_final(&env, &id);
    assert_eq!(v.status, RunStatus::Succeeded, "{v:?}");
    assert_eq!(v.exit_code, Some(0));
    assert_eq!(
        v.engine_session_id.as_deref(),
        Some("01a1ea72-6665-76f2-91bd-9907d3957816")
    );
    assert_eq!(
        (
            v.usage.input_tokens,
            v.usage.cached_tokens,
            v.usage.output_tokens
        ),
        (Some(67838), Some(56704), Some(137))
    );
    assert_eq!(v.tool_calls, 3);
    assert_eq!(v.model_reported, None, "o Codex não informa o modelo");
    assert!(v.final_text.unwrap().contains("Demo"));
    assert!(v.duration_ms.is_some());
    let ev = events(&env, &id);
    assert!(matches!(
        ev.first(),
        Some(EventPayload::Status {
            status: RunStatus::Running
        })
    ));
    assert!(matches!(
        ev.last(),
        Some(EventPayload::Status {
            status: RunStatus::Succeeded
        })
    ));
    assert!(ev.iter().any(|e| matches!(e, EventPayload::Message { .. })));
    assert_eq!(
        ev.iter()
            .filter(|e| matches!(e, EventPayload::ToolFinished { ok: true, .. }))
            .count(),
        3
    );

    // argumentos e entrada: somente leitura, workspace explícito, prompt só por stdin
    let argv = std::fs::read_to_string(env.dir.join("argv.txt")).unwrap();
    assert!(
        argv.contains("default_permissions=\"oa_workspace_ro\"")
            && argv.contains("\":root\"=\"deny\"")
            && argv.contains(&format!("-C {}", env.ws.display())),
        "{argv}"
    );
    assert!(
        !argv.contains(" -s "),
        "sem sandbox legado, que anularia o perfil: {argv}"
    );
    assert!(
        !argv.contains("Resuma"),
        "o prompt não pode aparecer em argv"
    );
    assert_eq!(
        std::fs::read_to_string(env.dir.join("stdin.txt")).unwrap(),
        "Resuma o projeto"
    );
    assert_eq!(
        std::fs::read_to_string(env.dir.join("cwd.txt"))
            .unwrap()
            .trim(),
        env.ws.display().to_string()
    );
    assert_eq!(
        std::fs::read_to_string(env.ws.join("README.md")).unwrap(),
        "# demo\n",
        "workspace intacto"
    );
}

#[test]
fn unauthorized_workspace_and_bad_requests_are_rejected() {
    let env = setup("badreq", &ok_body(), FAST, true);
    let mut r = req(&env);
    r.workspace_id = "ws_inexistente".into();
    assert_eq!(
        env.svc.start(&r).unwrap_err().kind,
        ErrorKind::WorkspaceNotAuthorized
    );
    let mut r = req(&env);
    r.prompt = "   ".into();
    assert_eq!(
        env.svc.start(&r).unwrap_err().kind,
        ErrorKind::InvalidRequest
    );
    r.prompt = "x".repeat(17 * 1024);
    assert_eq!(
        env.svc.start(&r).unwrap_err().kind,
        ErrorKind::InvalidRequest
    );
    let mut r = req(&env);
    r.model = Some("--dangerously-bypass-approvals-and-sandbox".into());
    assert_eq!(
        env.svc.start(&r).unwrap_err().kind,
        ErrorKind::InvalidRequest
    );
    let mut r = req(&env);
    r.engine = EngineKind::Claude;
    assert_eq!(
        env.svc.start(&r).unwrap_err().kind,
        ErrorKind::InvalidRequest,
        "só o Codex nesta fase"
    );
    assert!(
        !env.dir.join("argv.txt").exists(),
        "nenhum pedido inválido chegou a executar"
    );
}

#[test]
fn workspace_path_swapped_for_a_symlink_is_refused() {
    let env = setup("swap", &ok_body(), FAST, true);
    let other = env.dir.join("outro");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::remove_dir_all(&env.ws).unwrap();
    std::os::unix::fs::symlink(&other, &env.ws).unwrap();
    assert_eq!(
        env.svc.start(&req(&env)).unwrap_err().kind,
        ErrorKind::WorkspaceNotAuthorized
    );
}

#[test]
fn changed_executable_loses_trust_and_does_not_run() {
    let env = setup("changed", &ok_body(), FAST, true);
    fake_codex(
        &env.dir,
        "echo SUBSTITUIDO; touch '/tmp/oa-nao-deve-existir'",
    );
    assert_eq!(
        env.svc.start(&req(&env)).unwrap_err().kind,
        ErrorKind::NotApproved
    );
    assert!(!env.dir.join("argv.txt").exists());
}

/// Host que devolve outra identidade na SEGUNDA inspeção: simula a troca do binário entre a checagem
/// do `start` e o `exec`. A execução precisa ser recusada sem iniciar o processo.
struct FlipHost {
    inner: SystemHost,
    calls: AtomicUsize,
    spawned: AtomicUsize,
}

impl Host for FlipHost {
    fn inspect(&self, name: &str) -> Option<Inspected> {
        let mut i = self.inner.inspect(name)?;
        if self.calls.fetch_add(1, Ordering::SeqCst) >= 1 {
            if let Ok(id) = i.identity.as_mut() {
                id.fingerprint = "identidade-trocada".into();
            }
        }
        Some(i)
    }
    fn run(&self, exe: &TrustedExe, args: &[&str], limits: Limits) -> RunOutput {
        self.inner.run(exe, args, limits)
    }
    fn run_stream(
        &self,
        exe: &TrustedExe,
        spec: &StreamSpec,
        cancel: Option<&std::sync::atomic::AtomicBool>,
        on: &mut dyn FnMut(&[u8]),
    ) -> RunOutput {
        self.spawned.fetch_add(1, Ordering::SeqCst);
        self.inner.run_stream(exe, spec, cancel, on)
    }
}

#[test]
fn identity_is_revalidated_immediately_before_exec() {
    let dir = tmp("flip");
    fake_codex(&dir, &ok_body());
    let ws = dir.join("w");
    std::fs::create_dir_all(&ws).unwrap();
    let store = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
    store.lock().unwrap().ensure_local_gateway().unwrap();
    let ws_id = store
        .lock()
        .unwrap()
        .workspace_add("local", &ws.display().to_string(), "w")
        .unwrap()
        .id;
    let real = host(&dir);
    let trust = StoreTrust(store.clone());
    let c = candidate(EngineKind::Codex, real.as_ref(), &trust).unwrap();
    approve_engine(EngineKind::Codex, real.as_ref(), &trust, &c.fingerprint).unwrap();

    let flip = Arc::new(FlipHost {
        inner: SystemHost::with_dirs(vec![dir.clone(), "/bin".into(), "/usr/bin".into()]),
        calls: AtomicUsize::new(0),
        spawned: AtomicUsize::new(0),
    });
    let svc = ExecService::new(store.clone(), flip.clone(), FAST);
    let id = svc
        .start(&StartRunRequest {
            workspace_id: ws_id,
            engine: EngineKind::Codex,
            model: None,
            prompt: "x".into(),
        })
        .expect("a checagem antecipada passa (1ª inspeção)");
    let t = Instant::now();
    let v = loop {
        let v = store.lock().unwrap().run_get(&id).unwrap().unwrap();
        if !matches!(v.status, RunStatus::Queued | RunStatus::Running) {
            break v;
        }
        assert!(t.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(v.status, RunStatus::Failed);
    assert_eq!(v.error_kind, Some(ErrorKind::NotApproved));
    assert_eq!(
        flip.spawned.load(Ordering::SeqCst),
        0,
        "o processo não pode ter sido iniciado"
    );
    assert!(!dir.join("argv.txt").exists());
}

#[test]
fn executor_failure_is_reported_with_the_engine_message() {
    let body = format!("cat '{}'\nexit 1", fixture("failure.jsonl").display());
    let env = setup("failure", &body, FAST, true);
    let id = env.svc.start(&req(&env)).unwrap();
    let v = wait_final(&env, &id);
    assert_eq!(v.status, RunStatus::Failed);
    assert_eq!(v.error_kind, Some(ErrorKind::ExecutorFailed));
    assert!(v.error_message.unwrap().contains("not supported"));
    assert_eq!(v.exit_code, Some(1));
    let errs = events(&env, &id)
        .into_iter()
        .filter(|e| matches!(e, EventPayload::Error { .. }))
        .count();
    assert_eq!(errs, 1, "um único evento de erro");
}

#[test]
fn cancel_stops_the_process_and_its_descendants() {
    let body = format!("sleep 300 &\necho $! > '{d}/pid.txt'\nwait", d = "{DIR}");
    let env = setup("cancel", "true", FAST, true);
    fake_codex(
        &env.dir,
        &body.replace("{DIR}", &env.dir.display().to_string()),
    );
    // o binário mudou: reaprova a nova identidade (o usuário aprovaria de novo)
    let trust = StoreTrust(env.store.clone());
    let h = host(&env.dir);
    let c = candidate(EngineKind::Codex, h.as_ref(), &trust).unwrap();
    approve_engine(EngineKind::Codex, h.as_ref(), &trust, &c.fingerprint).unwrap();
    let svc = ExecService::new(env.store.clone(), h, FAST);

    let id = svc.start(&req(&env)).unwrap();
    let t = Instant::now();
    while !env.dir.join("pid.txt").exists() {
        assert!(t.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(20));
    }
    std::thread::sleep(Duration::from_millis(100));
    let pid: i32 = std::fs::read_to_string(env.dir.join("pid.txt"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(alive(pid));
    assert!(svc.cancel(&id));
    let t = Instant::now();
    let v = loop {
        let v = env.store.lock().unwrap().run_get(&id).unwrap().unwrap();
        if v.status != RunStatus::Running && v.status != RunStatus::Queued {
            break v;
        }
        assert!(t.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(
        (v.status, v.error_kind),
        (RunStatus::Cancelled, Some(ErrorKind::Cancelled))
    );
    for _ in 0..100 {
        if !alive(pid) {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("o descendente continua vivo");
}

#[test]
fn timeout_is_enforced_and_reported() {
    let limits = ExecLimits {
        timeout: Duration::from_millis(400),
        max_output: 1 << 20,
    };
    let env = setup("timeout", "sleep 300", limits, true);
    let t = Instant::now();
    let id = env.svc.start(&req(&env)).unwrap();
    let v = wait_final(&env, &id);
    assert_eq!(
        (v.status, v.error_kind),
        (RunStatus::Failed, Some(ErrorKind::Timeout))
    );
    assert!(t.elapsed() < Duration::from_secs(10));
    assert!(v.error_message.unwrap().contains("tempo limite"));
}

#[test]
fn output_flood_is_stopped() {
    let limits = ExecLimits {
        timeout: Duration::from_secs(30),
        max_output: 2000,
    };
    let env = setup("flood", "yes '{\"type\":\"x\"}'", limits, true);
    let id = env.svc.start(&req(&env)).unwrap();
    let v = wait_final(&env, &id);
    assert_eq!(v.error_kind, Some(ErrorKind::OutputLimit));
}

#[test]
fn one_run_per_workspace_at_a_time() {
    let env = setup(
        "busy",
        "sleep 300",
        ExecLimits {
            timeout: Duration::from_secs(30),
            max_output: 1 << 20,
        },
        true,
    );
    let first = env.svc.start(&req(&env)).unwrap();
    let e = env.svc.start(&req(&env)).unwrap_err();
    assert_eq!(e.kind, ErrorKind::WorkspaceBusy);
    assert!(env.svc.cancel(&first));
    let v = wait_final(&env, &first);
    assert_eq!(v.status, RunStatus::Cancelled);
    // liberado: uma nova execução é aceita
    let second = env.svc.start(&req(&env)).unwrap();
    env.svc.cancel(&second);
    wait_final(&env, &second);
}

#[test]
fn history_survives_restart_and_interrupted_runs_are_reconciled() {
    let dir = tmp("history");
    let db = dir.join("db").join("one.db");
    let ws = dir.join("w");
    std::fs::create_dir_all(&ws).unwrap();
    fake_codex(&dir, &ok_body());
    let (done_id, hung_id) = {
        let store = Arc::new(Mutex::new(Store::open(&db).unwrap()));
        store.lock().unwrap().ensure_local_gateway().unwrap();
        let ws_id = store
            .lock()
            .unwrap()
            .workspace_add("local", &ws.display().to_string(), "w")
            .unwrap()
            .id;
        let h = host(&dir);
        let trust = StoreTrust(store.clone());
        let c = candidate(EngineKind::Codex, h.as_ref(), &trust).unwrap();
        approve_engine(EngineKind::Codex, h.as_ref(), &trust, &c.fingerprint).unwrap();
        let svc = ExecService::new(store.clone(), h, FAST);
        let r = StartRunRequest {
            workspace_id: ws_id.clone(),
            engine: EngineKind::Codex,
            model: None,
            prompt: "primeira".into(),
        };
        let done = svc.start(&r).unwrap();
        let t = Instant::now();
        while store
            .lock()
            .unwrap()
            .run_get(&done)
            .unwrap()
            .unwrap()
            .status
            != RunStatus::Succeeded
        {
            assert!(t.elapsed() < Duration::from_secs(20));
            std::thread::sleep(Duration::from_millis(20));
        }
        while svc.is_active(&done) {
            std::thread::sleep(Duration::from_millis(10));
        }
        // uma execução "no ar" quando o app cai: criada e marcada running, sem processo
        let hung = store
            .lock()
            .unwrap()
            .run_create(&one_store::NewRun {
                workspace_id: &ws_id,
                engine: "codex",
                model: None,
                mode: one_protocol::ExecMode::ReadOnly,
                prompt: "travada",
            })
            .unwrap();
        store.lock().unwrap().run_mark_running(&hung).unwrap();
        (done, hung)
    }; // "app fechado": Store e serviço descartados
    let store = Store::open(&db).unwrap();
    assert_eq!(store.reconcile_interrupted().unwrap(), 1);
    let done = store.run_get(&done_id).unwrap().unwrap();
    assert_eq!(done.status, RunStatus::Succeeded);
    assert_eq!(done.prompt, "primeira");
    assert!(
        !store.events_after(&done_id, 0).unwrap().is_empty(),
        "eventos persistidos"
    );
    let hung = store.run_get(&hung_id).unwrap().unwrap();
    assert_eq!(
        (hung.status, hung.error_kind),
        (RunStatus::Failed, Some(ErrorKind::Interrupted))
    );
    assert_eq!(store.runs_list(10).unwrap().len(), 2);
}
