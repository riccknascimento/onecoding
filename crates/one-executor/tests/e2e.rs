//! Ponta a ponta com PROVEDORES SIMULADOS: OneHub Central real (PostgreSQL de teste na VPS, via túnel),
//! OneNode real (polling, mesma confinação) e OneExecutor real com um Codex FALSO aprovado. Nenhuma chamada
//! de IA. Marcado `#[ignore]`: exige `ONEAGENT_CENTRAL_TEST_DATABASE_URL` com nome terminando em `_test`.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use one_adapters::{approve_engine, candidate, SystemHost};
use one_central::{bootstrap, db, router};
use one_exec::StoreTrust;
use one_executor::agent::run_once as executor_run_once;
use one_executor::central::Central as ExecCentral;
use one_executor::job::SnapshotPolicy;
use one_node::agent::{run_once as node_run_once, Roots};
use one_node::central::Central as NodeCentral;
use one_protocol::EngineKind;
use one_store::Store;
use serde_json::{json, Value};
use tokio::net::TcpListener;

const ADMIN: &str = "admin@teste.local";
const ADMIN_PASS: &str = "senha-admin-teste-123";
const MEMBER: &str = "membro@teste.local";
const MEMBER_PASS: &str = "senha-membro-teste-123";

async fn api(
    http: &reqwest::Client,
    base: &str,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> (u16, Value) {
    let url = format!("{base}{path}");
    let mut req = if method == "GET" {
        http.get(&url)
    } else {
        http.post(&url)
    };
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    if let Some(b) = body {
        req = req.json(&b);
    }
    let res = req.send().await.expect("requisição ao Central");
    let status = res.status().as_u16();
    let text = res.text().await.unwrap_or_default();
    (status, serde_json::from_str(&text).unwrap_or(Value::Null))
}

async fn start_central() -> (String, db::DbPool) {
    let url = std::env::var("ONEAGENT_CENTRAL_TEST_DATABASE_URL")
        .expect("defina ONEAGENT_CENTRAL_TEST_DATABASE_URL");
    assert!(
        url.rsplit('/').next().unwrap_or("").ends_with("_test"),
        "recusado: banco precisa terminar em _test"
    );
    let pool = db::connect(&url).unwrap();
    {
        let c = pool.get().await.unwrap();
        c.batch_execute("DROP SCHEMA public CASCADE; CREATE SCHEMA public;")
            .await
            .unwrap();
    }
    db::migrate(&pool).await.unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = router(pool.clone());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}"), pool)
}

fn fake_codex(dir: &Path) {
    let script = format!(
        "#!/bin/sh\ncat > '{d}/stdin.txt'\n\
         echo '{{\"type\":\"thread.started\",\"thread_id\":\"t-fake\"}}'\n\
         echo '{{\"type\":\"item.completed\",\"item\":{{\"type\":\"agent_message\",\"text\":\"O projeto é o SystemFlow (README.md).\"}}}}'\n\
         echo '{{\"type\":\"turn.completed\",\"usage\":{{\"input_tokens\":3,\"output_tokens\":2}}}}'\n",
        d = dir.display()
    );
    let p = dir.join("codex");
    std::fs::write(&p, script).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "banco PostgreSQL de teste na VPS (ver cabeçalho)"]
async fn chat_request_reaches_fake_codex_through_onenode_and_returns_evidence() {
    let (base, pool) = start_central().await;
    let http = reqwest::Client::new();
    let tmp: PathBuf = std::fs::canonicalize(std::env::temp_dir())
        .unwrap()
        .join(format!("oa-exec-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let ws = tmp.join("ws");
    let bin = tmp.join("bin");
    let work = tmp.join("work");
    for d in [&ws, &bin, &work] {
        std::fs::create_dir_all(d).unwrap();
    }
    std::fs::write(
        ws.join("README.md"),
        "Projeto SystemFlow. Mede o desempenho dos processos.",
    )
    .unwrap();
    std::fs::write(ws.join(".env"), "TOKEN=nao-ler\n").unwrap();
    fake_codex(&bin);

    // administrador, membro, OneNode (pareado), projeto com concessão
    bootstrap::bootstrap_admin(&pool, ADMIN, ADMIN_PASS)
        .await
        .unwrap();
    let (_, l) = api(
        &http,
        &base,
        "POST",
        "/v1/login",
        None,
        Some(json!({"email": ADMIN, "password": ADMIN_PASS})),
    )
    .await;
    let admin = l["token"].as_str().unwrap().to_string();
    api(
        &http,
        &base,
        "POST",
        "/v1/users",
        Some(&admin),
        Some(json!({"email": MEMBER, "password": MEMBER_PASS})),
    )
    .await;
    let (_, l) = api(
        &http,
        &base,
        "POST",
        "/v1/login",
        None,
        Some(json!({"email": MEMBER, "password": MEMBER_PASS})),
    )
    .await;
    let member = l["token"].as_str().unwrap().to_string();
    let (_, node) = api(
        &http,
        &base,
        "POST",
        "/v1/onenodes",
        Some(&admin),
        Some(json!({"name": "OneNode teste"})),
    )
    .await;
    let nid = node["id"].as_str().unwrap().to_string();
    let (b, n, c) = (
        base.clone(),
        nid.clone(),
        node["pairing_code"].as_str().unwrap().to_string(),
    );
    let node_token = tokio::task::spawn_blocking(move || NodeCentral::pair(&b, &n, &c).unwrap())
        .await
        .unwrap();
    let (_, p) = api(
        &http,
        &base,
        "POST",
        "/v1/projects",
        Some(&admin),
        Some(json!({"name": "SystemFlow"})),
    )
    .await;
    let pid = p["id"].as_str().unwrap().to_string();
    api(
        &http,
        &base,
        "POST",
        &format!("/v1/projects/{pid}/grants"),
        Some(&admin),
        Some(json!({"node_id": nid})),
    )
    .await;

    // OneNode atendendo em paralelo (polling), até o fim do teste
    let stop = Arc::new(AtomicBool::new(false));
    let node_roots = Roots(vec![(pid.clone(), ws.canonicalize().unwrap())]);
    let (nb, nt, ns) = (base.clone(), node_token.clone(), stop.clone());
    let node_thread = std::thread::spawn(move || {
        let central = NodeCentral::new(&nb, &nt).unwrap();
        while !ns.load(Ordering::SeqCst) {
            let _ = node_run_once(&central, &node_roots);
            std::thread::sleep(Duration::from_millis(30));
        }
    });

    // OneExecutor pareado (identidade própria) e aprovação do Codex falso (mesmo mecanismo do app)
    let (_, ex) = api(
        &http,
        &base,
        "POST",
        "/v1/executors",
        Some(&admin),
        Some(json!({"name": "OneExecutor Mac (teste)"})),
    )
    .await;
    let eid = ex["id"].as_str().unwrap().to_string();
    let (b, e, c) = (
        base.clone(),
        eid.clone(),
        ex["pairing_code"].as_str().unwrap().to_string(),
    );
    let ex_token = tokio::task::spawn_blocking(move || ExecCentral::pair(&b, &e, &c).unwrap())
        .await
        .unwrap();
    let host = SystemHost::with_dirs(vec![bin.clone(), "/bin".into(), "/usr/bin".into()]);
    let trust = StoreTrust(Arc::new(Mutex::new(Store::open_in_memory().unwrap())));
    let cand = candidate(EngineKind::Codex, &host, &trust).unwrap();
    approve_engine(EngineKind::Codex, &host, &trust, &cand.fingerprint).unwrap();

    // o usuário pergunta no chat, com OneNode e projeto escolhidos
    let (_, conv) = api(
        &http,
        &base,
        "POST",
        "/v1/conversations",
        Some(&member),
        Some(json!({"title": "Análise"})),
    )
    .await;
    let cid = conv["id"].as_str().unwrap().to_string();
    let (s, r) = api(
        &http,
        &base,
        "POST",
        &format!("/v1/conversations/{cid}/turns"),
        Some(&member),
        Some(json!({"text": "Investigue o projeto SystemFlow", "node_id": nid, "project_id": pid})),
    )
    .await;
    assert_eq!(s, 201);
    assert_eq!(r["assistant_message"]["state"], "task");
    let job = r["assistant_message"]["job_id"]
        .as_str()
        .unwrap()
        .to_string();

    // o executor pega o job e executa (snapshot pelo OneNode + Codex falso)
    let policy = SnapshotPolicy {
        max_files: 12,
        max_file_bytes: 16 * 1024,
        poll: Duration::from_millis(20),
        task_timeout: Duration::from_secs(20),
    };
    let (eb, et, ew) = (base.clone(), ex_token.clone(), work.clone());
    let ran = tokio::task::spawn_blocking(move || {
        let central = ExecCentral::new(&eb, &et).unwrap();
        executor_run_once(&central, &host, &trust, &ew, &policy)
    })
    .await
    .unwrap();
    assert_eq!(ran, Ok(true));
    stop.store(true, Ordering::SeqCst);
    node_thread.join().unwrap();

    // resultado e evidências chegam ao chat; o .env nunca é lido nem devolvido
    let (_, j) = api(
        &http,
        &base,
        "GET",
        &format!("/v1/jobs/{job}"),
        Some(&member),
        None,
    )
    .await;
    assert_eq!(j["status"], "succeeded", "{j}");
    assert_eq!(j["answer"], "O projeto é o SystemFlow (README.md).");
    assert_eq!(j["evidence"], json!(["README.md"]));
    assert!(
        !j.to_string().contains("TOKEN=nao-ler"),
        "segredo não pode vazar"
    );
    let (_, msgs) = api(
        &http,
        &base,
        "GET",
        &format!("/v1/conversations/{cid}/messages"),
        Some(&member),
        None,
    )
    .await;
    let answer = msgs
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["job_id"] == job.as_str() && m["role"] == "assistant")
        .unwrap();
    assert_eq!(
        (answer["state"].as_str(), answer["body"].as_str()),
        (
            Some("completed"),
            Some("O projeto é o SystemFlow (README.md).")
        )
    );
    // a pasta do snapshot foi apagada ao final
    assert!(!work.join(format!("job-{job}")).exists());
    let _ = std::fs::remove_dir_all(&tmp);
}
