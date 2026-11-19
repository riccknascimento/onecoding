//! Ponta a ponta: OneHub Central REAL (PostgreSQL de teste na VPS, via túnel) + OneNodes reais (binário
//! de biblioteca, polling HTTP autenticado). Marcado `#[ignore]`. Exige `ONEAGENT_CENTRAL_TEST_DATABASE_URL`
//! apontando para um banco cujo nome termina em `_test`: o schema `public` desse banco é recriado.

use std::path::{Path, PathBuf};

use one_central::{bootstrap, db, router};
use one_node::agent::{run_once, Cycle, Roots};
use one_node::catalog::{self, Catalog};
use one_node::central::Central;
use one_node::{config, state};
use serde_json::{json, Value};
use tokio::net::TcpListener;

const ADMIN: &str = "admin@teste.local";
const ADMIN_PASS: &str = "senha-admin-teste-123";
const MEMBER: &str = "membro@teste.local";
const MEMBER_PASS: &str = "senha-membro-teste-123";
const KEYWORD: &str = "ONEAGENT-VALIDADO-2026";

async fn api(
    http: &reqwest::Client,
    base: &str,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> (u16, Value) {
    let url = format!("{base}{path}");
    let mut req = match method {
        "GET" => http.get(&url),
        _ => http.post(&url),
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

/// Cria workspace com `in.txt`, um `.env` (sensível), um symlink para fora e um segredo fora da raiz.
fn make_workspace(base: &Path) -> (PathBuf, PathBuf) {
    let root = base.join("ws");
    let outside = base.join("fora");
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(
        root.join("sub/in.txt"),
        format!("Projeto de teste. Palavra-chave: {KEYWORD}.\n"),
    )
    .unwrap();
    std::fs::write(root.join(".env"), "TOKEN=nao-ler\n").unwrap();
    std::fs::write(outside.join("segredo.txt"), "CONTEUDO-FORA-DO-WORKSPACE\n").unwrap();
    std::os::unix::fs::symlink(outside.join("segredo.txt"), root.join("atalho.txt")).unwrap();
    (
        root.canonicalize().unwrap(),
        outside.canonicalize().unwrap(),
    )
}

fn node_config(base_url: &str, state_file: &Path) -> String {
    json!({ "central_url": base_url, "state_file": state_file, "projects": [] }).to_string()
}

/// Dados de um OneNode de teste. O cliente HTTP bloqueante é criado e destruído dentro de `spawn_blocking`
/// (um cliente bloqueante não pode ser destruído num contexto assíncrono).
#[derive(Clone)]
struct NodeHandle {
    base: String,
    token: String,
    roots: Roots,
}

async fn pair_node(
    base: &str,
    dir: &Path,
    node_id: &str,
    code: &str,
    allowed_root: &Path,
) -> NodeHandle {
    let state_file = dir.join("identidade.json");
    let cfg_text = node_config(base, &state_file);
    let cfg = config::parse(&cfg_text).expect("configuração do OneNode");
    let (b, n, c) = (base.to_string(), node_id.to_string(), code.to_string());
    let sf = state_file.clone();
    let token = tokio::task::spawn_blocking(move || {
        let token = Central::pair(&b, &n, &c).expect("pareamento");
        state::save(
            &sf,
            &state::NodeIdentity {
                node_id: n.clone(),
                token: token.clone(),
            },
        )
        .unwrap();
        token
    })
    .await
    .unwrap();
    let policy_path = dir.join("policy.json");
    std::fs::write(
        &policy_path,
        json!({ "allowed_roots": [allowed_root] }).to_string(),
    )
    .unwrap();
    let (base_url, auth_token) = (cfg.central_url, token.clone());
    let roots = tokio::task::spawn_blocking(move || {
        let central = Central::new(&base_url, &auth_token).unwrap();
        let mut catalog = Catalog::default();
        catalog::sync(&central, &policy_path, &mut catalog).expect("catálogo autorizado");
        catalog.roots
    })
    .await
    .unwrap();
    NodeHandle {
        base: base.to_string(),
        token,
        roots,
    }
}

/// Executa ciclos do nó até ficar ocioso. O cliente nasce e morre dentro da thread bloqueante.
async fn drain(node: &NodeHandle) -> Vec<String> {
    let (base, token, roots) = (node.base.clone(), node.token.clone(), node.roots.clone());
    tokio::task::spawn_blocking(move || {
        let central = Central::new(&base, &token).unwrap();
        let mut out = Vec::new();
        for _ in 0..20 {
            match run_once(&central, &roots) {
                Ok(Cycle::Idle) => break,
                Ok(Cycle::Handled { status, .. }) => out.push(status.to_string()),
                Err(e) => {
                    out.push(format!("erro:{e:?}"));
                    break;
                }
            }
        }
        out
    })
    .await
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "banco PostgreSQL de teste na VPS (ver cabeçalho)"]
async fn onenode_full_cycle_with_authorization_and_isolation() {
    let (base, pool) = start_central().await;
    let http = reqwest::Client::new();
    let tmp = std::env::temp_dir().join(format!("oa-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let (ws, _outside) = make_workspace(&tmp);

    // 1. administrador (CLI de bootstrap), login e usuário membro
    bootstrap::bootstrap_admin(&pool, ADMIN, ADMIN_PASS)
        .await
        .unwrap();
    let (s, login) = api(
        &http,
        &base,
        "POST",
        "/v1/login",
        None,
        Some(json!({"email": ADMIN, "password": ADMIN_PASS})),
    )
    .await;
    assert_eq!(s, 200);
    let admin = login["token"].as_str().unwrap().to_string();
    let (s, _) = api(
        &http,
        &base,
        "POST",
        "/v1/users",
        Some(&admin),
        Some(json!({"email": MEMBER, "password": MEMBER_PASS})),
    )
    .await;
    assert_eq!(s, 201);
    let (_, login) = api(
        &http,
        &base,
        "POST",
        "/v1/login",
        None,
        Some(json!({"email": MEMBER, "password": MEMBER_PASS})),
    )
    .await;
    let member = login["token"].as_str().unwrap().to_string();

    // 2. dois OneNodes, dois projetos: só projeto1 é concedido ao nó A
    let (_, a) = api(
        &http,
        &base,
        "POST",
        "/v1/onenodes",
        Some(&admin),
        Some(json!({"name": "OneNode A (teste)"})),
    )
    .await;
    let (_, b) = api(
        &http,
        &base,
        "POST",
        "/v1/onenodes",
        Some(&admin),
        Some(json!({"name": "OneNode B (teste)"})),
    )
    .await;
    let (id_a, id_b) = (
        a["id"].as_str().unwrap().to_string(),
        b["id"].as_str().unwrap().to_string(),
    );
    let (_, p1) = api(
        &http,
        &base,
        "POST",
        "/v1/projects",
        Some(&admin),
        Some(json!({"name": "Projeto 1"})),
    )
    .await;
    let (_, p2) = api(
        &http,
        &base,
        "POST",
        "/v1/projects",
        Some(&admin),
        Some(json!({"name": "Projeto 2"})),
    )
    .await;
    let (pid1, pid2) = (
        p1["id"].as_str().unwrap().to_string(),
        p2["id"].as_str().unwrap().to_string(),
    );
    let (s, _) = api(
        &http,
        &base,
        "POST",
        &format!("/v1/projects/{pid1}/grants"),
        Some(&admin),
        Some(json!({"node_id": id_a, "root_path": ws})),
    )
    .await;
    assert_eq!(s, 201);

    // 3. pareamento: código de uso único; nó A recebe a identidade dele
    let node_a = pair_node(
        &base,
        &tmp.join("node-a"),
        &id_a,
        a["pairing_code"].as_str().unwrap(),
        &tmp,
    )
    .await;
    let node_b = pair_node(
        &base,
        &tmp.join("node-b"),
        &id_b,
        b["pairing_code"].as_str().unwrap(),
        &tmp,
    )
    .await;

    // 4. criação de tarefas pelo usuário (como faria a interface Web)
    let (s, _) = api(
        &http,
        &base,
        "POST",
        "/v1/tasks",
        Some(&member),
        Some(json!({
            "node_id": id_b, "project_id": pid2, "tool": "files_list", "args": {"path": "."},
            "prompt": "listar", "idempotency_key": "chave-sem-grant-01"
        })),
    )
    .await;
    assert_eq!(s, 403, "sem concessão OneNode B × projeto 2: nada criado");
    let (s, _) = api(
        &http,
        &base,
        "POST",
        "/v1/tasks",
        Some(&member),
        Some(json!({
            "node_id": id_a, "project_id": pid2, "tool": "files_list", "args": {"path": "."},
            "prompt": "listar", "idempotency_key": "chave-outro-proj-01"
        })),
    )
    .await;
    assert_eq!(
        s, 403,
        "OneNode A não recebe tarefa de projeto que não lhe foi concedido"
    );
    let mk = |tool: &str, path: &str, key: &str| {
        json!({
            "node_id": id_a, "project_id": pid1, "tool": tool, "args": {"path": path},
            "prompt": format!("{tool} {path}"), "idempotency_key": key
        })
    };
    let (s, t_ok) = api(
        &http,
        &base,
        "POST",
        "/v1/tasks",
        Some(&member),
        Some(mk("files_read", "sub/in.txt", "tarefa-leitura-ok-1")),
    )
    .await;
    assert_eq!(s, 201);
    let t_ok = t_ok["id"].as_str().unwrap().to_string();
    let (_, t_trav) = api(
        &http,
        &base,
        "POST",
        "/v1/tasks",
        Some(&member),
        Some(mk(
            "files_read",
            "../fora/segredo.txt",
            "tarefa-traversal-01",
        )),
    )
    .await;
    let t_trav = t_trav["id"].as_str().unwrap().to_string();
    let (_, t_link) = api(
        &http,
        &base,
        "POST",
        "/v1/tasks",
        Some(&member),
        Some(mk("files_read", "atalho.txt", "tarefa-symlink-0001")),
    )
    .await;
    let t_link = t_link["id"].as_str().unwrap().to_string();
    let (_, t_env) = api(
        &http,
        &base,
        "POST",
        "/v1/tasks",
        Some(&member),
        Some(mk("files_read", ".env", "tarefa-env-sensivel1")),
    )
    .await;
    let t_env = t_env["id"].as_str().unwrap().to_string();
    let (s, _) = api(
        &http,
        &base,
        "POST",
        "/v1/tasks",
        Some(&member),
        Some(mk("shell", "x", "tarefa-ferramenta-ruim")),
    )
    .await;
    assert_eq!(
        s, 400,
        "ferramenta fora do catálogo é recusada pelo Central"
    );
    let (s, _) = api(
        &http,
        &base,
        "POST",
        "/v1/tasks",
        Some(&member),
        Some(mk("files_read", "../x", "tarefa-idem-conflito")),
    )
    .await;
    assert_eq!(s, 201);

    // 5. OneNode B não recebe nada do nó A (e nada foi criado para ele)
    let statuses_b = drain(&node_b).await;
    assert!(
        statuses_b.is_empty(),
        "nó B não pode receber tarefas do nó A: {statuses_b:?}"
    );

    // 6. nó A executa: leitura autorizada, e as negações acontecem no nó
    let statuses_a = drain(&node_a).await;
    assert!(statuses_a.len() >= 4, "tarefas executadas: {statuses_a:?}");
    let (_, ok) = api(
        &http,
        &base,
        "GET",
        &format!("/v1/tasks/{t_ok}"),
        Some(&member),
        None,
    )
    .await;
    assert_eq!(ok["status"], "succeeded");
    assert!(
        ok["result"]["content"].as_str().unwrap().contains(KEYWORD),
        "resultado persistido com o conteúdo lido"
    );
    assert_eq!(
        ok["events"].as_array().unwrap().len(),
        2,
        "eventos started e finished persistidos"
    );
    for t in [&t_trav, &t_link] {
        let (_, v) = api(
            &http,
            &base,
            "GET",
            &format!("/v1/tasks/{t}"),
            Some(&member),
            None,
        )
        .await;
        assert_eq!(v["status"], "failed", "fora do workspace: falha");
        assert_eq!(v["result"]["error"], "caminho fora do workspace autorizado");
        assert!(
            !v.to_string().contains("CONTEUDO-FORA-DO-WORKSPACE"),
            "conteúdo de fora nunca aparece no resultado"
        );
    }
    let (_, env) = api(
        &http,
        &base,
        "GET",
        &format!("/v1/tasks/{t_env}"),
        Some(&member),
        None,
    )
    .await;
    assert_eq!(env["status"], "failed");
    assert!(env["result"]["error"]
        .as_str()
        .unwrap()
        .contains("sensível"));

    // 7. reconexão: o nó reinicia e não repete tarefas já concluídas
    let restarted = NodeHandle {
        base: base.clone(),
        token: central_a_token(&tmp),
        roots: node_a.roots.clone(),
    };
    let again = drain(&restarted).await;
    assert!(again.is_empty(), "nada pendente após reconexão: {again:?}");
    let (_, after) = api(
        &http,
        &base,
        "GET",
        &format!("/v1/tasks/{t_ok}"),
        Some(&member),
        None,
    )
    .await;
    assert_eq!(
        after["events"].as_array().unwrap().len(),
        2,
        "sem execução duplicada"
    );

    // 8. revogação: o nó B, revogado, perde acesso imediatamente
    let (s, _) = api(
        &http,
        &base,
        "POST",
        &format!("/v1/onenodes/{id_b}/revoke"),
        Some(&admin),
        None,
    )
    .await;
    assert_eq!(s, 204);
    let statuses_rev = drain(&node_b).await;
    assert!(
        statuses_rev.iter().any(|s| s.contains("Unauthorized")),
        "nó revogado é recusado: {statuses_rev:?}"
    );
    let _ = std::fs::remove_dir_all(&tmp);
}

fn central_a_token(tmp: &Path) -> String {
    state::load(&tmp.join("node-a/identidade.json"))
        .unwrap()
        .token
}
