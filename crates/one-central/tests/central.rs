//! Testes de integração do OneHub Central contra PostgreSQL REAL (banco de teste na VPS, via túnel).
//! Marcados `#[ignore]`: rodar com `--ignored`, com `ONEAGENT_CENTRAL_TEST_DATABASE_URL` apontando para um
//! banco cujo nome termina em `_test`. O teste APAGA o schema `public` desse banco antes de começar.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use one_central::auth::sha256;
use one_central::{db, router};
use serde_json::{json, Value};
use tower::ServiceExt;

const ADMIN_EMAIL: &str = "admin@teste.local";
const ADMIN_PASS: &str = "senha-admin-teste-123";
const MEMBER_EMAIL: &str = "membro@teste.local";
const MEMBER_PASS: &str = "senha-membro-teste-123";

async fn call(
    app: &axum::Router,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut b = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json");
    if let Some(t) = token {
        b = b.header("authorization", format!("Bearer {t}"));
    }
    let req = b
        .body(Body::from(body.map(|v| v.to_string()).unwrap_or_default()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let v = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, v)
}

fn test_url() -> String {
    let url = std::env::var("ONEAGENT_CENTRAL_TEST_DATABASE_URL")
        .expect("defina ONEAGENT_CENTRAL_TEST_DATABASE_URL");
    let db_name = url.rsplit('/').next().unwrap_or("");
    assert!(
        db_name.ends_with("_test"),
        "recusado: o banco de teste precisa terminar em _test (era `{db_name}`)"
    );
    url
}

async fn fresh_app() -> (axum::Router, db::DbPool) {
    let pool = db::connect(&test_url()).unwrap();
    {
        let client = pool.get().await.unwrap();
        client
            .batch_execute("DROP SCHEMA public CASCADE; CREATE SCHEMA public;")
            .await
            .unwrap();
    }
    let applied = db::migrate(&pool).await.expect("migrations");
    assert!(applied >= 1, "as migrations deveriam ser aplicadas");
    (router(pool.clone()), pool)
}

#[tokio::test]
#[ignore = "banco PostgreSQL de teste na VPS (ver cabeçalho)"]
async fn central_end_to_end() {
    let (app, pool) = fresh_app().await;

    // saúde e migrations registradas
    let (s, h) = call(&app, "GET", "/health", None, None).await;
    assert_eq!((s, h["database"].as_str()), (StatusCode::OK, Some("ok")));
    let migrations: i64 = {
        let c = pool.get().await.unwrap();
        c.query_one("SELECT count(*) FROM schema_migrations", &[])
            .await
            .unwrap()
            .get(0)
    };
    assert!(migrations >= 1);

    // bootstrap: só uma vez, senha curta recusada
    assert!(
        one_central::bootstrap::bootstrap_admin(&pool, ADMIN_EMAIL, "curta")
            .await
            .is_err()
    );
    one_central::bootstrap::bootstrap_admin(&pool, ADMIN_EMAIL, ADMIN_PASS)
        .await
        .unwrap();
    assert_eq!(
        one_central::bootstrap::bootstrap_admin(&pool, "outro@teste.local", ADMIN_PASS).await,
        Err(one_central::bootstrap::BootstrapError::AlreadyInitialized)
    );

    // senha guardada como Argon2id, nunca em texto puro
    {
        let c = pool.get().await.unwrap();
        let hash: String = c
            .query_one(
                "SELECT password_hash FROM users WHERE email = $1",
                &[&ADMIN_EMAIL],
            )
            .await
            .unwrap()
            .get(0);
        assert!(hash.starts_with("$argon2id$") && !hash.contains(ADMIN_PASS));
    }

    // login: errado recusado; certo gera sessão cujo hash (e não o token) fica no banco
    let (s, _) = call(
        &app,
        "POST",
        "/v1/login",
        None,
        Some(json!({"email": ADMIN_EMAIL, "password": "errada-errada-123"})),
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, login) = call(
        &app,
        "POST",
        "/v1/login",
        None,
        Some(json!({"email": ADMIN_EMAIL, "password": ADMIN_PASS})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let admin = login["token"].as_str().unwrap().to_string();
    {
        let c = pool.get().await.unwrap();
        let stored: i64 = c
            .query_one(
                "SELECT count(*) FROM sessions WHERE token_hash = $1",
                &[&sha256(&admin)],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(stored, 1, "a sessão precisa estar guardada pelo hash");
        let plain: i64 = c
            .query_one(
                "SELECT count(*) FROM sessions WHERE encode(token_hash, 'hex') = $1",
                &[&admin],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(plain, 0);
    }
    let (s, _) = call(&app, "GET", "/v1/me", None, None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED, "sem token, nada");
    let (s, me) = call(&app, "GET", "/v1/me", Some(&admin), None).await;
    assert_eq!((s, me["role"].as_str()), (StatusCode::OK, Some("admin")));

    // membro: criado pelo admin; membro não cria OneNode nem projeto
    let (s, _) = call(
        &app,
        "POST",
        "/v1/users",
        Some(&admin),
        Some(json!({"email": MEMBER_EMAIL, "password": MEMBER_PASS})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    let (_, login) = call(
        &app,
        "POST",
        "/v1/login",
        None,
        Some(json!({"email": MEMBER_EMAIL, "password": MEMBER_PASS})),
    )
    .await;
    let member = login["token"].as_str().unwrap().to_string();
    let (s, _) = call(
        &app,
        "POST",
        "/v1/onenodes",
        Some(&member),
        Some(json!({"name": "x"})),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    // OneNode: pareamento com código de uso único e validade
    let (s, node) = call(
        &app,
        "POST",
        "/v1/onenodes",
        Some(&admin),
        Some(json!({"name": "Servidor Teste Linux"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    let node_id = node["id"].as_str().unwrap().to_string();
    let code = node["pairing_code"].as_str().unwrap().to_string();
    let (s, _) = call(
        &app,
        "POST",
        "/node/v1/pair",
        None,
        Some(json!({"node_id": node_id, "code": "codigo-errado"})),
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, paired) = call(
        &app,
        "POST",
        "/node/v1/pair",
        None,
        Some(json!({"node_id": node_id, "code": code})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let node_token = paired["token"].as_str().unwrap().to_string();
    let (s, _) = call(
        &app,
        "POST",
        "/node/v1/pair",
        None,
        Some(json!({"node_id": node_id, "code": code})),
    )
    .await;
    assert_eq!(
        s,
        StatusCode::UNAUTHORIZED,
        "código de pareamento é de uso único"
    );

    // heartbeat autenticado; status online
    let (s, _) = call(&app, "POST", "/node/v1/heartbeat", None, None).await;
    assert_eq!(
        s,
        StatusCode::UNAUTHORIZED,
        "heartbeat sem identidade do nó é recusado"
    );
    let (s, _) = call(&app, "POST", "/node/v1/heartbeat", Some(&node_token), None).await;
    assert_eq!(s, StatusCode::OK);
    let (_, list) = call(&app, "GET", "/v1/onenodes", Some(&member), None).await;
    let st = list
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["id"] == node_id.as_str())
        .unwrap()["status"]
        .clone();
    assert_eq!(st, "online");

    // projetos: tarefa sem concessão é recusada; com concessão, criada; idempotência
    let (s, proj) = call(
        &app,
        "POST",
        "/v1/projects",
        Some(&admin),
        Some(json!({"name": "Projeto Teste"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    let project_id = proj["id"].as_str().unwrap().to_string();
    let task_body = json!({
        "node_id": node_id, "project_id": project_id, "tool": "files_list", "args": {"path": "."},
        "prompt": "Leia in.txt e responda somente com a palavra-chave.", "idempotency_key": "tarefa-teste-0001"
    });
    let (s, _) = call(
        &app,
        "POST",
        "/v1/tasks",
        Some(&member),
        Some(task_body.clone()),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "sem concessão, nada é criado");
    let (s, _) = call(
        &app,
        "POST",
        "/v1/projects/{id}/grants"
            .replace("{id}", &project_id)
            .as_str(),
        Some(&admin),
        Some(json!({"node_id": node_id, "root_path": "/srv/teste"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    let (s, first) = call(
        &app,
        "POST",
        "/v1/tasks",
        Some(&member),
        Some(task_body.clone()),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    let task_id = first["id"].as_str().unwrap().to_string();
    let (s, again) = call(
        &app,
        "POST",
        "/v1/tasks",
        Some(&member),
        Some(task_body.clone()),
    )
    .await;
    assert_eq!(
        (s, again["id"].as_str()),
        (StatusCode::OK, Some(task_id.as_str())),
        "reenvio idempotente"
    );
    let mut changed = task_body.clone();
    changed["prompt"] = json!("outro conteúdo");
    let (s, body) = call(&app, "POST", "/v1/tasks", Some(&member), Some(changed)).await;
    assert_eq!(
        (s, body["error"].as_str()),
        (StatusCode::CONFLICT, Some("idempotency_key_reused"))
    );

    // pull: uma única entrega com lease; segunda consulta não recebe a mesma tarefa
    let (s, pulled) = call(&app, "GET", "/node/v1/tasks/next", Some(&node_token), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(pulled["id"].as_str(), Some(task_id.as_str()));
    let lease = pulled["lease_token"].as_str().unwrap().to_string();
    let (s, _) = call(&app, "GET", "/node/v1/tasks/next", Some(&node_token), None).await;
    assert_eq!(
        s,
        StatusCode::NO_CONTENT,
        "tarefa já em execução não é entregue de novo"
    );

    // eventos: seq duplicado recusado; lease errado recusado
    let (s, _) = call(&app, "POST", &format!("/node/v1/tasks/{task_id}/events"), Some(&node_token),
        Some(json!({"lease_token": lease, "seq": 0, "kind": "status", "payload": {"msg": "iniciou"}}))).await;
    assert_eq!(s, StatusCode::CREATED);
    let (s, _) = call(
        &app,
        "POST",
        &format!("/node/v1/tasks/{task_id}/events"),
        Some(&node_token),
        Some(json!({"lease_token": lease, "seq": 0, "kind": "status", "payload": {}})),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT);
    let (s, _) = call(&app, "POST", &format!("/node/v1/tasks/{task_id}/events"), Some(&node_token),
        Some(json!({"lease_token": "00000000-0000-4000-8000-000000000000", "seq": 1, "kind": "status", "payload": {}}))).await;
    assert_eq!(s, StatusCode::CONFLICT);

    // finalização com o lease correto; membro vê status, resultado e eventos
    let (s, _) = call(&app, "POST", &format!("/node/v1/tasks/{task_id}/finish"), Some(&node_token),
        Some(json!({"lease_token": lease, "status": "succeeded", "result": {"palavra_chave": "ONEAGENT-VALIDADO-2026"}}))).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _) = call(
        &app,
        "POST",
        &format!("/node/v1/tasks/{task_id}/finish"),
        Some(&node_token),
        Some(json!({"lease_token": lease, "status": "succeeded", "result": {}})),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT, "finalizar duas vezes é recusado");
    let (s, t) = call(
        &app,
        "GET",
        &format!("/v1/tasks/{task_id}"),
        Some(&member),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(t["status"], "succeeded");
    assert_eq!(t["result"]["palavra_chave"], "ONEAGENT-VALIDADO-2026");
    assert_eq!(t["events"].as_array().unwrap().len(), 1);

    // conversas: independentes por dono
    let (s, conv) = call(
        &app,
        "POST",
        "/v1/conversations",
        Some(&member),
        Some(json!({"title": "Teste"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    let conv_id = conv["id"].as_str().unwrap().to_string();
    let (s, _) = call(
        &app,
        "POST",
        &format!("/v1/conversations/{conv_id}/messages"),
        Some(&member),
        Some(json!({"body": "oi"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    let (s, msgs) = call(
        &app,
        "GET",
        &format!("/v1/conversations/{conv_id}/messages"),
        Some(&member),
        None,
    )
    .await;
    assert_eq!(
        (s, msgs.as_array().map(|a| a.len())),
        (StatusCode::OK, Some(1))
    );
    let (s, _) = call(
        &app,
        "GET",
        &format!("/v1/conversations/{conv_id}/messages"),
        Some(&admin),
        None,
    )
    .await;
    assert_eq!(
        s,
        StatusCode::NOT_FOUND,
        "conversa de outro usuário não aparece nem ao admin"
    );

    // revogação do OneNode: nó perde acesso, tarefas pendentes são canceladas
    let (s, _) = call(&app, "POST", "/v1/tasks", Some(&member), Some(json!({
        "node_id": node_id, "project_id": project_id, "tool": "files_list", "args": {"path": "."}, "prompt": "segunda tarefa", "idempotency_key": "tarefa-teste-0002"
    }))).await;
    assert_eq!(s, StatusCode::CREATED);
    let (s, _) = call(
        &app,
        "POST",
        &format!("/v1/onenodes/{node_id}/revoke"),
        Some(&admin),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _) = call(&app, "POST", "/node/v1/heartbeat", Some(&node_token), None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED, "nó revogado perde acesso");
    let (s, _) = call(&app, "GET", "/node/v1/tasks/next", Some(&node_token), None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (_, list) = call(&app, "GET", "/v1/onenodes", Some(&member), None).await;
    let st = list
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["id"] == node_id.as_str())
        .unwrap()["status"]
        .clone();
    assert_eq!(st, "revoked");
    {
        let c = pool.get().await.unwrap();
        let s2: String = c
            .query_one(
                "SELECT status FROM tasks WHERE idempotency_key = 'tarefa-teste-0002'",
                &[],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(s2, "cancelled");
    }

    // sessão revogada no logout; expiração também é recusada
    let (s, _) = call(&app, "POST", "/v1/logout", Some(&member), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _) = call(&app, "GET", "/v1/me", Some(&member), None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED, "sessão revogada não vale mais");
    {
        let c = pool.get().await.unwrap();
        c.execute(
            "UPDATE sessions SET expires_at = now() - interval '1 second' WHERE token_hash = $1",
            &[&sha256(&admin)],
        )
        .await
        .unwrap();
    }
    let (s, _) = call(&app, "GET", "/v1/me", Some(&admin), None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED, "sessão expirada não vale mais");
}

#[tokio::test]
#[ignore = "banco PostgreSQL de teste na VPS (ver cabeçalho)"]
async fn lease_expiry_requeues_and_stale_node_is_refused() {
    let (app, pool) = fresh_app().await;
    one_central::bootstrap::bootstrap_admin(&pool, ADMIN_EMAIL, ADMIN_PASS)
        .await
        .unwrap();
    let (_, login) = call(
        &app,
        "POST",
        "/v1/login",
        None,
        Some(json!({"email": ADMIN_EMAIL, "password": ADMIN_PASS})),
    )
    .await;
    let admin = login["token"].as_str().unwrap().to_string();
    let (_, node) = call(
        &app,
        "POST",
        "/v1/onenodes",
        Some(&admin),
        Some(json!({"name": "Nó A"})),
    )
    .await;
    let nid = node["id"].as_str().unwrap().to_string();
    let (_, p) = call(
        &app,
        "POST",
        "/node/v1/pair",
        None,
        Some(json!({"node_id": nid, "code": node["pairing_code"]})),
    )
    .await;
    let tok = p["token"].as_str().unwrap().to_string();
    let (_, proj) = call(
        &app,
        "POST",
        "/v1/projects",
        Some(&admin),
        Some(json!({"name": "P"})),
    )
    .await;
    let pid = proj["id"].as_str().unwrap().to_string();
    call(
        &app,
        "POST",
        &format!("/v1/projects/{pid}/grants"),
        Some(&admin),
        Some(json!({"node_id": nid, "root_path": "/srv/teste"})),
    )
    .await;
    let (_, t) = call(&app, "POST", "/v1/tasks", Some(&admin), Some(json!({
        "node_id": nid, "project_id": pid, "tool": "files_list", "args": {"path": "."}, "prompt": "tarefa de lease", "idempotency_key": "lease-teste-0001"
    }))).await;
    let tid = t["id"].as_str().unwrap().to_string();

    let (_, first) = call(&app, "GET", "/node/v1/tasks/next", Some(&tok), None).await;
    let old_lease = first["lease_token"].as_str().unwrap().to_string();
    // simula o lease vencido (nó parou no meio)
    {
        let c = pool.get().await.unwrap();
        let tid_uuid = uuid::Uuid::parse_str(&tid).unwrap();
        c.execute(
            "UPDATE tasks SET lease_expires = now() - interval '1 second' WHERE id = $1",
            &[&tid_uuid],
        )
        .await
        .unwrap();
    }
    // o nó antigo não pode mais registrar nada
    let (s, _) = call(
        &app,
        "POST",
        &format!("/node/v1/tasks/{tid}/finish"),
        Some(&tok),
        Some(json!({"lease_token": old_lease, "status": "succeeded", "result": {}})),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT, "lease vencido não finaliza");
    // nova consulta devolve a tarefa à fila e entrega com lease NOVO
    let (s, second) = call(&app, "GET", "/node/v1/tasks/next", Some(&tok), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(second["id"].as_str(), Some(tid.as_str()));
    let new_lease = second["lease_token"].as_str().unwrap().to_string();
    assert_ne!(new_lease, old_lease);
    let (s, _) = call(
        &app,
        "POST",
        &format!("/node/v1/tasks/{tid}/finish"),
        Some(&tok),
        Some(json!({"lease_token": old_lease, "status": "succeeded", "result": {}})),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT, "token antigo continua recusado");
    let (s, _) = call(
        &app,
        "POST",
        &format!("/node/v1/tasks/{tid}/finish"),
        Some(&tok),
        Some(json!({"lease_token": new_lease, "status": "failed", "result": {"motivo": "teste"}})),
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
}

#[tokio::test]
#[ignore = "banco PostgreSQL de teste na VPS (ver cabeçalho)"]
async fn concurrent_pulls_never_deliver_the_same_task_twice() {
    let (app, pool) = fresh_app().await;
    one_central::bootstrap::bootstrap_admin(&pool, ADMIN_EMAIL, ADMIN_PASS)
        .await
        .unwrap();
    let (_, login) = call(
        &app,
        "POST",
        "/v1/login",
        None,
        Some(json!({"email": ADMIN_EMAIL, "password": ADMIN_PASS})),
    )
    .await;
    let admin = login["token"].as_str().unwrap().to_string();
    let (_, node) = call(
        &app,
        "POST",
        "/v1/onenodes",
        Some(&admin),
        Some(json!({"name": "Nó B"})),
    )
    .await;
    let nid = node["id"].as_str().unwrap().to_string();
    let (_, p) = call(
        &app,
        "POST",
        "/node/v1/pair",
        None,
        Some(json!({"node_id": nid, "code": node["pairing_code"]})),
    )
    .await;
    let tok = p["token"].as_str().unwrap().to_string();
    let (_, proj) = call(
        &app,
        "POST",
        "/v1/projects",
        Some(&admin),
        Some(json!({"name": "P2"})),
    )
    .await;
    let pid = proj["id"].as_str().unwrap().to_string();
    call(
        &app,
        "POST",
        &format!("/v1/projects/{pid}/grants"),
        Some(&admin),
        Some(json!({"node_id": nid, "root_path": "/srv/teste"})),
    )
    .await;
    call(&app, "POST", "/v1/tasks", Some(&admin), Some(json!({
        "node_id": nid, "project_id": pid, "tool": "files_list", "args": {"path": "."}, "prompt": "unica", "idempotency_key": "concorrencia-0001"
    }))).await;
    // dez consultas simultâneas do mesmo nó: exatamente uma recebe a tarefa
    let mut handles = Vec::new();
    for _ in 0..10 {
        let app = app.clone();
        let tok = tok.clone();
        handles.push(tokio::spawn(async move {
            call(&app, "GET", "/node/v1/tasks/next", Some(&tok), None)
                .await
                .0
        }));
    }
    let mut ok = 0;
    for h in handles {
        if h.await.unwrap() == StatusCode::OK {
            ok += 1;
        }
    }
    assert_eq!(ok, 1, "uma única entrega entre consultas concorrentes");
}

async fn admin_and_member(app: &axum::Router, pool: &db::DbPool) -> (String, String) {
    one_central::bootstrap::bootstrap_admin(pool, ADMIN_EMAIL, ADMIN_PASS)
        .await
        .unwrap();
    let (_, l) = call(
        app,
        "POST",
        "/v1/login",
        None,
        Some(json!({"email": ADMIN_EMAIL, "password": ADMIN_PASS})),
    )
    .await;
    let admin = l["token"].as_str().unwrap().to_string();
    call(
        app,
        "POST",
        "/v1/users",
        Some(&admin),
        Some(json!({"email": MEMBER_EMAIL, "password": MEMBER_PASS})),
    )
    .await;
    let (_, l) = call(
        app,
        "POST",
        "/v1/login",
        None,
        Some(json!({"email": MEMBER_EMAIL, "password": MEMBER_PASS})),
    )
    .await;
    (admin, l["token"].as_str().unwrap().to_string())
}

#[tokio::test]
#[ignore = "banco PostgreSQL de teste na VPS (ver cabeçalho)"]
async fn chat_turns_route_commands_respect_scope_and_never_fake_ai_replies() {
    let (app, pool) = fresh_app().await;
    let (admin, member) = admin_and_member(&app, &pool).await;
    // análise exige OneExecutor online (sem ele, nada é enfileirado)
    let _executor = paired_executor(&app, &admin).await;
    let (_, node) = call(
        &app,
        "POST",
        "/v1/onenodes",
        Some(&admin),
        Some(json!({"name": "Nó chat"})),
    )
    .await;
    let nid = node["id"].as_str().unwrap().to_string();
    let (_, p1) = call(
        &app,
        "POST",
        "/v1/projects",
        Some(&admin),
        Some(json!({"name": "P1"})),
    )
    .await;
    let (_, p2) = call(
        &app,
        "POST",
        "/v1/projects",
        Some(&admin),
        Some(json!({"name": "P2"})),
    )
    .await;
    let (pid1, pid2) = (
        p1["id"].as_str().unwrap().to_string(),
        p2["id"].as_str().unwrap().to_string(),
    );
    call(
        &app,
        "POST",
        &format!("/v1/projects/{pid1}/grants"),
        Some(&admin),
        Some(json!({"node_id": nid, "root_path": "/srv/teste"})),
    )
    .await;

    let (_, c) = call(
        &app,
        "POST",
        "/v1/conversations",
        Some(&member),
        Some(json!({"title": "Nova conversa"})),
    )
    .await;
    let cid = c["id"].as_str().unwrap().to_string();
    let turn = |text: &str, node_id: Option<&str>, project_id: Option<&str>| json!({"text": text, "node_id": node_id, "project_id": project_id});

    // sem contexto: /investigar não cria tarefa
    let (s, r) = call(
        &app,
        "POST",
        &format!("/v1/conversations/{cid}/turns"),
        Some(&member),
        Some(turn("/investigar sub/in.txt", None, None)),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    assert_eq!(r["assistant_message"]["state"], "rejected");
    assert!(r["assistant_message"]["task_id"].is_null());

    // projeto sem OneNode é contexto incompleto (erro explícito); OneNode sem projeto é válido
    let (s, _) = call(
        &app,
        "POST",
        &format!("/v1/conversations/{cid}/turns"),
        Some(&member),
        Some(turn("oi", None, Some(&pid1))),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // projeto sem concessão: recusado, e o contexto não muda
    let (s, _) = call(
        &app,
        "POST",
        &format!("/v1/conversations/{cid}/turns"),
        Some(&member),
        Some(turn("oi", Some(&nid), Some(&pid2))),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    // com contexto válido: /investigar cria tarefa somente leitura, persistida e associada à mensagem
    let (s, r) = call(
        &app,
        "POST",
        &format!("/v1/conversations/{cid}/turns"),
        Some(&member),
        Some(turn("/investigar sub/in.txt", Some(&nid), Some(&pid1))),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    assert_eq!(r["assistant_message"]["state"], "task");
    let task_id = r["assistant_message"]["task_id"]
        .as_str()
        .unwrap()
        .to_string();
    let (_, t) = call(
        &app,
        "GET",
        &format!("/v1/tasks/{task_id}"),
        Some(&member),
        None,
    )
    .await;
    assert_eq!(
        (t["status"].as_str(), t["tool"].as_str()),
        (Some("queued"), Some("files_read"))
    );
    assert_eq!(t["args"]["path"], "sub/in.txt");

    // comandos de escrita e desconhecidos: recusados com mensagem clara, sem tarefa
    for cmd in ["/corrigir bug", "/melhorar texto", "/testar", "/inventado"] {
        let (s, r) = call(
            &app,
            "POST",
            &format!("/v1/conversations/{cid}/turns"),
            Some(&member),
            Some(turn(cmd, Some(&nid), Some(&pid1))),
        )
        .await;
        assert_eq!(s, StatusCode::CREATED);
        assert_eq!(r["assistant_message"]["state"], "rejected", "{cmd}");
        assert!(r["assistant_message"]["task_id"].is_null());
    }

    // texto livre sem provedor de IA: vira pedido de análise ao OneExecutor (job), nunca resposta simulada
    let (_, r) = call(
        &app,
        "POST",
        &format!("/v1/conversations/{cid}/turns"),
        Some(&member),
        Some(turn("me ajude a planejar", Some(&nid), Some(&pid1))),
    )
    .await;
    assert_eq!(r["assistant_message"]["state"], "task");
    assert!(r["assistant_message"]["job_id"].is_string());
    // sem projeto escolhido e com UM projeto concedido ao nó: identificado pela concessão, sem perguntar
    let (_, r) = call(
        &app,
        "POST",
        &format!("/v1/conversations/{cid}/turns"),
        Some(&member),
        Some(json!({"text": "analise", "node_id": nid})),
    )
    .await;
    assert_eq!(r["assistant_message"]["state"], "task");
    // com DOIS projetos concedidos, sem escolha: pede o projeto em vez de adivinhar
    call(
        &app,
        "POST",
        &format!("/v1/projects/{pid2}/grants"),
        Some(&admin),
        Some(json!({"node_id": nid, "root_path": "/srv/teste"})),
    )
    .await;
    // conversa nova, sem projeto escolhido: com dois projetos concedidos, pede o projeto
    let (_, c3) = call(
        &app,
        "POST",
        "/v1/conversations",
        Some(&member),
        Some(json!({"title": "Sem projeto"})),
    )
    .await;
    let c3id = c3["id"].as_str().unwrap().to_string();
    let (_, r) = call(
        &app,
        "POST",
        &format!("/v1/conversations/{c3id}/turns"),
        Some(&member),
        Some(json!({"text": "analise de novo", "node_id": nid})),
    )
    .await;
    assert_eq!(r["assistant_message"]["state"], "rejected");

    // histórico persistido com estado e tarefa; contexto salvo na conversa
    let (_, msgs) = call(
        &app,
        "GET",
        &format!("/v1/conversations/{cid}/messages"),
        Some(&member),
        None,
    )
    .await;
    let states: Vec<&str> = msgs
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["role"] == "assistant")
        .map(|m| m["state"].as_str().unwrap())
        .collect();
    assert_eq!(
        states,
        vec!["rejected", "task", "rejected", "rejected", "rejected", "rejected", "task", "task"]
    );
    assert!(msgs
        .as_array()
        .unwrap()
        .iter()
        .any(|m| m["task_id"] == task_id.as_str()));
    let (_, convs) = call(&app, "GET", "/v1/conversations", Some(&member), None).await;
    let mine = convs
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == cid.as_str())
        .unwrap();
    assert_eq!(
        (mine["node_id"].as_str(), mine["project_id"].as_str()),
        (Some(nid.as_str()), Some(pid1.as_str()))
    );

    // conversa de outro usuário não aceita turno (nem aparece para o admin)
    let (s, _) = call(
        &app,
        "POST",
        &format!("/v1/conversations/{cid}/turns"),
        Some(&admin),
        Some(turn("oi", None, None)),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
#[ignore = "banco PostgreSQL de teste na VPS (ver cabeçalho)"]
async fn login_is_locked_after_repeated_failures_even_with_the_right_password() {
    let (app, pool) = fresh_app().await;
    one_central::bootstrap::bootstrap_admin(&pool, ADMIN_EMAIL, ADMIN_PASS)
        .await
        .unwrap();
    for _ in 0..5 {
        let (s, _) = call(
            &app,
            "POST",
            "/v1/login",
            None,
            Some(json!({"email": ADMIN_EMAIL, "password": "errada-errada-123"})),
        )
        .await;
        assert_eq!(s, StatusCode::UNAUTHORIZED);
    }
    let (s, body) = call(
        &app,
        "POST",
        "/v1/login",
        None,
        Some(json!({"email": ADMIN_EMAIL, "password": "errada-errada-123"})),
    )
    .await;
    assert_eq!(
        (s, body["error"].as_str()),
        (StatusCode::TOO_MANY_REQUESTS, Some("too_many_attempts"))
    );
    // com a senha certa, o bloqueio continua valendo dentro da janela
    let (s, _) = call(
        &app,
        "POST",
        "/v1/login",
        None,
        Some(json!({"email": ADMIN_EMAIL, "password": ADMIN_PASS})),
    )
    .await;
    assert_eq!(s, StatusCode::TOO_MANY_REQUESTS);
    // e-mail inexistente recebe o mesmo bloqueio: não revela quais contas existem
    for _ in 0..5 {
        call(
            &app,
            "POST",
            "/v1/login",
            None,
            Some(json!({"email": "ninguem@teste.local", "password": "errada-errada-123"})),
        )
        .await;
    }
    let (s, _) = call(
        &app,
        "POST",
        "/v1/login",
        None,
        Some(json!({"email": "ninguem@teste.local", "password": "errada-errada-123"})),
    )
    .await;
    assert_eq!(s, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
#[ignore = "banco PostgreSQL de teste na VPS (ver cabeçalho)"]
async fn executor_job_flow_with_leases_grants_and_isolation() {
    let (app, pool) = fresh_app().await;
    let (admin, member) = admin_and_member(&app, &pool).await;
    // OneNode pareado
    let (_, node) = call(
        &app,
        "POST",
        "/v1/onenodes",
        Some(&admin),
        Some(json!({"name": "Nó job"})),
    )
    .await;
    let nid = node["id"].as_str().unwrap().to_string();
    let (_, p) = call(
        &app,
        "POST",
        "/node/v1/pair",
        None,
        Some(json!({"node_id": nid, "code": node["pairing_code"]})),
    )
    .await;
    let node_tok = p["token"].as_str().unwrap().to_string();
    let (_, pr) = call(
        &app,
        "POST",
        "/v1/projects",
        Some(&admin),
        Some(json!({"name": "Projeto job"})),
    )
    .await;
    let pid = pr["id"].as_str().unwrap().to_string();
    call(
        &app,
        "POST",
        &format!("/v1/projects/{pid}/grants"),
        Some(&admin),
        Some(json!({"node_id": nid, "root_path": "/srv/teste"})),
    )
    .await;
    // OneExecutor pareado (identidade própria, sem credenciais de IA)
    let (s, ex) = call(
        &app,
        "POST",
        "/v1/executors",
        Some(&admin),
        Some(json!({"name": "Executor Mac (teste)"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    let eid = ex["id"].as_str().unwrap().to_string();
    let (s, paired) = call(
        &app,
        "POST",
        "/executor/v1/pair",
        None,
        Some(json!({"executor_id": eid, "code": ex["pairing_code"]})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let ex_tok = paired["token"].as_str().unwrap().to_string();
    // executor pronto para análise: motor Codex instalado, autenticado e aprovado
    call(&app, "POST", "/executor/v1/heartbeat", Some(&ex_tok),
        Some(json!([{"name": "codex", "installed": true, "authenticated": true, "approved": true}]))).await;
    let (_, list) = call(&app, "GET", "/v1/executors", Some(&member), None).await;
    assert_eq!(list[0]["status"], "online");

    // o usuário pede análise em texto livre dentro do contexto OneNode × projeto
    let (_, c) = call(
        &app,
        "POST",
        "/v1/conversations",
        Some(&member),
        Some(json!({"title": "Análise"})),
    )
    .await;
    let cid = c["id"].as_str().unwrap().to_string();
    let (_, r) = call(
        &app,
        "POST",
        &format!("/v1/conversations/{cid}/turns"),
        Some(&member),
        Some(json!({"text": "Investigue o projeto", "node_id": nid, "project_id": pid})),
    )
    .await;
    assert_eq!(r["assistant_message"]["state"], "task");
    let job = r["assistant_message"]["job_id"]
        .as_str()
        .unwrap()
        .to_string();
    let (_, j) = call(&app, "GET", &format!("/v1/jobs/{job}"), Some(&member), None).await;
    assert_eq!(j["status"], "queued");

    // executor pega o job com lease
    let (s, lease) = call(&app, "POST", "/executor/v1/jobs/next", Some(&ex_tok), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(lease["id"].as_str(), Some(job.as_str()));
    let lease_tok = lease["lease_token"].as_str().unwrap().to_string();
    let (s, _) = call(&app, "POST", "/executor/v1/jobs/next", Some(&ex_tok), None).await;
    assert_eq!(
        s,
        StatusCode::NO_CONTENT,
        "job em execução não é entregue de novo"
    );

    // ferramenta pedida pelo executor: lease errado é recusado; o certo cria tarefa no mesmo nó e projeto
    let (s, _) = call(&app, "POST", &format!("/executor/v1/jobs/{job}/tools"), Some(&ex_tok),
        Some(json!({"lease_token": "00000000-0000-4000-8000-000000000000", "tool": "files_list", "path": "."}))).await;
    assert_eq!(s, StatusCode::CONFLICT);
    let (s, _) = call(
        &app,
        "POST",
        &format!("/executor/v1/jobs/{job}/tools"),
        Some(&ex_tok),
        Some(json!({"lease_token": lease_tok, "tool": "shell", "path": "."})),
    )
    .await;
    assert_eq!(
        s,
        StatusCode::BAD_REQUEST,
        "ferramenta fora do catálogo é recusada"
    );
    let (s, t) = call(
        &app,
        "POST",
        &format!("/executor/v1/jobs/{job}/tools"),
        Some(&ex_tok),
        Some(json!({"lease_token": lease_tok, "tool": "files_list", "path": "."})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    let task_id = t["task_id"].as_str().unwrap().to_string();
    // o OneNode processa a tarefa (como em produção: pelo próprio token)
    let (_, pulled) = call(&app, "GET", "/node/v1/tasks/next", Some(&node_tok), None).await;
    assert_eq!(pulled["id"].as_str(), Some(task_id.as_str()));
    let nl = pulled["lease_token"].as_str().unwrap().to_string();
    call(&app, "POST", &format!("/node/v1/tasks/{task_id}/finish"), Some(&node_tok),
        Some(json!({"lease_token": nl, "status": "succeeded", "result": {"entries": [{"name": "README.md", "kind": "file"}]}}))).await;
    let (s, st) = call(
        &app,
        "GET",
        &format!("/executor/v1/jobs/{job}/tasks/{task_id}"),
        Some(&ex_tok),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(st["status"], "succeeded");
    assert_eq!(st["result"]["entries"][0]["name"], "README.md");
    // outro executor (ou outro job) não lê a tarefa
    let (_, other) = call(
        &app,
        "POST",
        &format!("/executor/v1/jobs/{job}/events"),
        Some(&ex_tok),
        Some(json!({"lease_token": lease_tok, "kind": "progress", "payload": {"msg": "lendo"}})),
    )
    .await;
    let _ = other;

    // finalização com resposta: a mensagem do assistente recebe o texto e o job vira succeeded
    let (s, _) = call(&app, "POST", &format!("/executor/v1/jobs/{job}/finish"), Some(&ex_tok),
        Some(json!({"lease_token": lease_tok, "status": "succeeded", "answer": "O README descreve o projeto.", "evidence": ["README.md"]}))).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _) = call(
        &app,
        "POST",
        &format!("/executor/v1/jobs/{job}/finish"),
        Some(&ex_tok),
        Some(json!({"lease_token": lease_tok, "status": "succeeded", "answer": "repetido"})),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT, "finalizar de novo é recusado");
    let (_, msgs) = call(
        &app,
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
        (Some("completed"), Some("O README descreve o projeto."))
    );
    let (_, j) = call(&app, "GET", &format!("/v1/jobs/{job}"), Some(&member), None).await;
    assert_eq!(j["status"], "succeeded");
    assert_eq!(j["evidence"][0], "README.md");

    // outro usuário não vê o job
    let (_, other_user) = call(
        &app,
        "POST",
        "/v1/login",
        None,
        Some(json!({"email": ADMIN_EMAIL, "password": ADMIN_PASS})),
    )
    .await;
    let admin_tok = other_user["token"].as_str().unwrap().to_string();
    let (s, _) = call(
        &app,
        "GET",
        &format!("/v1/jobs/{job}"),
        Some(&admin_tok),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // concessão revogada: jobs pendentes são cancelados antes de qualquer execução
    let (_, r2) = call(
        &app,
        "POST",
        &format!("/v1/conversations/{cid}/turns"),
        Some(&member),
        Some(json!({"text": "outra análise", "node_id": nid, "project_id": pid})),
    )
    .await;
    let job2 = r2["assistant_message"]["job_id"]
        .as_str()
        .unwrap()
        .to_string();
    call(
        &app,
        "POST",
        &format!("/v1/projects/{pid}/grants/{nid}/revoke"),
        Some(&admin),
        None,
    )
    .await;
    let (s, _) = call(&app, "POST", "/executor/v1/jobs/next", Some(&ex_tok), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (_, j2) = call(
        &app,
        "GET",
        &format!("/v1/jobs/{job2}"),
        Some(&member),
        None,
    )
    .await;
    assert_eq!(j2["status"], "cancelled");

    // executor revogado perde acesso
    call(
        &app,
        "POST",
        &format!("/v1/executors/{eid}/revoke"),
        Some(&admin),
        None,
    )
    .await;
    let (s, _) = call(&app, "POST", "/executor/v1/heartbeat", Some(&ex_tok), None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
}

// ---------- tempo real, cookie de sessão, cancelamento e escolha de projeto ----------

/// Requisição crua (cabeçalhos livres), para cookies, Origin e stream SSE.
async fn raw(
    app: &axum::Router,
    method: &str,
    path: &str,
    headers: &[(&str, String)],
    body: Option<Value>,
) -> axum::response::Response {
    let mut b = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json");
    for (k, v) in headers {
        b = b.header(*k, v);
    }
    let req = b
        .body(Body::from(body.map(|v| v.to_string()).unwrap_or_default()))
        .unwrap();
    app.clone().oneshot(req).await.unwrap()
}

/// Quadro SSE: (id, evento, dados).
type Frame = (String, String, String);

fn parse_sse(text: &str) -> Vec<Frame> {
    let complete = match text.rfind("\n\n") {
        Some(i) => &text[..i],
        None => return vec![],
    };
    complete
        .split("\n\n")
        .filter_map(|block| {
            let (mut id, mut ev, mut data) = (String::new(), String::new(), String::new());
            for line in block.lines() {
                if let Some(v) = line.strip_prefix("id: ") {
                    id = v.into();
                } else if let Some(v) = line.strip_prefix("event: ") {
                    ev = v.into();
                } else if let Some(v) = line.strip_prefix("data: ") {
                    data = v.into();
                }
            }
            (!ev.is_empty()).then_some((id, ev, data))
        })
        .collect()
}

/// Lê o stream até `done` ser verdadeiro ou o prazo acabar.
async fn read_stream(
    res: axum::response::Response,
    done: impl Fn(&[Frame]) -> bool,
    within: std::time::Duration,
) -> Vec<Frame> {
    let mut body = res.into_body();
    let mut text = String::new();
    let deadline = std::time::Instant::now() + within;
    loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            break;
        }
        match tokio::time::timeout(left, body.frame()).await {
            Ok(Some(Ok(frame))) => {
                if let Ok(data) = frame.into_data() {
                    text.push_str(std::str::from_utf8(&data).unwrap());
                }
                let frames = parse_sse(&text);
                if done(&frames) {
                    return frames;
                }
            }
            _ => break,
        }
    }
    parse_sse(&text)
}

fn session_cookie_value(set_cookie: &str) -> String {
    set_cookie
        .split(';')
        .next()
        .unwrap()
        .trim_start_matches("oh_session=")
        .to_string()
}

/// OneNode pareado, projeto com concessão ativa para ele. Devolve (nó, projeto, token do nó).
async fn node_with_project(app: &axum::Router, admin: &str, project: &str) -> (String, String) {
    let (_, node) = call(
        app,
        "POST",
        "/v1/onenodes",
        Some(admin),
        Some(json!({"name": "Nó tempo real"})),
    )
    .await;
    let nid = node["id"].as_str().unwrap().to_string();
    call(
        app,
        "POST",
        "/node/v1/pair",
        None,
        Some(json!({"node_id": nid, "code": node["pairing_code"]})),
    )
    .await;
    let (_, pr) = call(
        app,
        "POST",
        "/v1/projects",
        Some(admin),
        Some(json!({"name": project})),
    )
    .await;
    let pid = pr["id"].as_str().unwrap().to_string();
    call(
        app,
        "POST",
        &format!("/v1/projects/{pid}/grants"),
        Some(admin),
        Some(json!({"node_id": nid, "root_path": "/srv/teste"})),
    )
    .await;
    (nid, pid)
}

async fn paired_executor(app: &axum::Router, admin: &str) -> String {
    let (_, ex) = call(
        app,
        "POST",
        "/v1/executors",
        Some(admin),
        Some(json!({"name": "Executor tempo real"})),
    )
    .await;
    let eid = ex["id"].as_str().unwrap().to_string();
    let (_, p) = call(
        app,
        "POST",
        "/executor/v1/pair",
        None,
        Some(json!({"executor_id": eid, "code": ex["pairing_code"]})),
    )
    .await;
    let token = p["token"].as_str().unwrap().to_string();
    // o executor reporta o motor Codex pronto: instalado, autenticado e aprovado
    let (s, _) = call(
        app,
        "POST",
        "/executor/v1/heartbeat",
        Some(&token),
        Some(json!([{"name": "codex", "installed": true, "authenticated": true, "approved": true, "version": "0.164.0"}])),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    token
}

async fn new_conversation(app: &axum::Router, token: &str) -> String {
    let (_, c) = call(
        app,
        "POST",
        "/v1/conversations",
        Some(token),
        Some(json!({"title": "Tempo real"})),
    )
    .await;
    c["id"].as_str().unwrap().to_string()
}

#[tokio::test]
#[ignore = "exige ONEAGENT_CENTRAL_TEST_DATABASE_URL (banco _test): rodar com --ignored --test-threads=1"]
async fn concurrent_writes_get_a_gapless_event_sequence() {
    let (app, pool) = fresh_app().await;
    let (admin, member) = admin_and_member(&app, &pool).await;
    let cid = new_conversation(&app, &member).await;
    let mut tasks = Vec::new();
    for i in 0..20 {
        let (app, member, cid) = (app.clone(), member.clone(), cid.clone());
        tasks.push(tokio::spawn(async move {
            call(
                &app,
                "POST",
                &format!("/v1/conversations/{cid}/messages"),
                Some(&member),
                Some(json!({"body": format!("mensagem {i}")})),
            )
            .await
        }));
    }
    for t in tasks {
        assert_eq!(t.await.unwrap().0, StatusCode::CREATED);
    }
    let client = pool.get().await.unwrap();
    let cid_uuid: uuid::Uuid = cid.parse().unwrap();
    let seqs: Vec<i64> = client
        .query(
            "SELECT seq FROM conversation_events WHERE conversation_id = $1 ORDER BY seq",
            &[&cid_uuid],
        )
        .await
        .unwrap()
        .iter()
        .map(|r| r.get(0))
        .collect();
    assert_eq!(
        seqs,
        (1..=20).collect::<Vec<i64>>(),
        "sem lacunas nem repetições"
    );
    let _ = admin;
}

#[tokio::test]
#[ignore = "exige ONEAGENT_CENTRAL_TEST_DATABASE_URL (banco _test): rodar com --ignored --test-threads=1"]
async fn snapshot_then_stream_resumes_without_gap_or_duplicate() {
    let (app, pool) = fresh_app().await;
    let (_admin, member) = admin_and_member(&app, &pool).await;
    let cid = new_conversation(&app, &member).await;
    for i in 0..3 {
        call(
            &app,
            "POST",
            &format!("/v1/conversations/{cid}/messages"),
            Some(&member),
            Some(json!({"body": format!("antes {i}")})),
        )
        .await;
    }
    let (_, state) = call(
        &app,
        "GET",
        &format!("/v1/conversations/{cid}/state"),
        Some(&member),
        None,
    )
    .await;
    let last = state["last_seq"].as_i64().unwrap();
    assert_eq!(last, 3);
    assert_eq!(state["messages"].as_array().unwrap().len(), 3);
    // mudanças depois do snapshot: a retomada precisa entregá-las, uma vez cada
    for i in 0..2 {
        call(
            &app,
            "POST",
            &format!("/v1/conversations/{cid}/messages"),
            Some(&member),
            Some(json!({"body": format!("depois {i}")})),
        )
        .await;
    }
    let res = raw(
        &app,
        "GET",
        &format!("/v1/conversations/{cid}/events"),
        &[
            ("authorization", format!("Bearer {member}")),
            ("last-event-id", last.to_string()),
        ],
        None,
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);
    let frames = read_stream(
        res,
        |f| f.iter().filter(|x| x.1 == "message.created").count() >= 2,
        std::time::Duration::from_secs(5),
    )
    .await;
    let ids: Vec<&str> = frames
        .iter()
        .filter(|f| f.1 == "message.created")
        .map(|f| f.0.as_str())
        .collect();
    assert_eq!(ids, vec!["4", "5"], "retomada sem lacuna nem duplicata");
    let (_, state_after) = call(
        &app,
        "GET",
        &format!("/v1/conversations/{cid}/state"),
        Some(&member),
        None,
    )
    .await;
    assert_eq!(state_after["last_seq"], 5);
}

#[tokio::test]
#[ignore = "exige ONEAGENT_CENTRAL_TEST_DATABASE_URL (banco _test): rodar com --ignored --test-threads=1"]
async fn stream_and_state_are_private_to_the_owner() {
    let (app, pool) = fresh_app().await;
    let (admin, member) = admin_and_member(&app, &pool).await;
    let cid = new_conversation(&app, &member).await;
    let (s1, _) = call(
        &app,
        "GET",
        &format!("/v1/conversations/{cid}/state"),
        Some(&admin),
        None,
    )
    .await;
    let res = raw(
        &app,
        "GET",
        &format!("/v1/conversations/{cid}/events"),
        &[("authorization", format!("Bearer {admin}"))],
        None,
    )
    .await;
    assert_eq!(s1, StatusCode::NOT_FOUND);
    assert_eq!(
        res.status(),
        StatusCode::NOT_FOUND,
        "terceiro não abre o stream"
    );
}

#[tokio::test]
#[ignore = "exige ONEAGENT_CENTRAL_TEST_DATABASE_URL (banco _test): rodar com --ignored --test-threads=1"]
async fn revoked_grant_and_logout_close_the_open_stream() {
    let (app, pool) = fresh_app().await;
    let (admin, member) = admin_and_member(&app, &pool).await;
    let (nid, pid) = node_with_project(&app, &admin, "Projeto revogável").await;
    let cid = new_conversation(&app, &member).await;
    call(
        &app,
        "POST",
        &format!("/v1/conversations/{cid}/turns"),
        Some(&member),
        Some(json!({"text": "Oi", "node_id": nid, "project_id": pid})),
    )
    .await;
    // stream aberto com concessão válida; a revogação precisa encerrá-lo
    let res = raw(
        &app,
        "GET",
        &format!("/v1/conversations/{cid}/events?after=0"),
        &[("authorization", format!("Bearer {member}"))],
        None,
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);
    let (s, _) = call(
        &app,
        "POST",
        &format!("/v1/projects/{pid}/grants/{nid}/revoke"),
        Some(&admin),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let frames = read_stream(
        res,
        |f| f.iter().any(|x| x.1 == "closed"),
        std::time::Duration::from_secs(6),
    )
    .await;
    let closed = frames
        .iter()
        .find(|f| f.1 == "closed")
        .expect("stream encerrado pela revogação");
    assert!(closed.2.contains("grant_revoked"), "motivo: {}", closed.2);

    // sessão encerrada por logout também encerra o stream (conversa sem contexto: a concessão já foi revogada)
    let cid2 = new_conversation(&app, &member).await;
    let res = raw(
        &app,
        "GET",
        &format!("/v1/conversations/{cid2}/events?after=0"),
        &[("authorization", format!("Bearer {member}"))],
        None,
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);
    let out = raw(
        &app,
        "POST",
        "/v1/logout",
        &[("authorization", format!("Bearer {member}"))],
        None,
    )
    .await;
    assert_eq!(out.status(), StatusCode::NO_CONTENT);
    let frames = read_stream(
        res,
        |f| f.iter().any(|x| x.1 == "closed"),
        std::time::Duration::from_secs(6),
    )
    .await;
    let closed = frames
        .iter()
        .find(|f| f.1 == "closed")
        .expect("stream encerrado pelo logout");
    assert!(closed.2.contains("session_ended"));
}

#[tokio::test]
#[ignore = "exige ONEAGENT_CENTRAL_TEST_DATABASE_URL (banco _test): rodar com --ignored --test-threads=1"]
async fn cookie_session_is_http_only_and_writes_need_same_origin() {
    let (app, pool) = fresh_app().await;
    let (_admin, member) = admin_and_member(&app, &pool).await;
    let login = raw(
        &app,
        "POST",
        "/v1/login",
        &[],
        Some(json!({"email": MEMBER_EMAIL, "password": MEMBER_PASS})),
    )
    .await;
    assert_eq!(login.status(), StatusCode::OK);
    let set_cookie = login.headers()["set-cookie"].to_str().unwrap().to_string();
    for attr in [
        "HttpOnly",
        "SameSite=Strict",
        "Secure",
        "Path=/",
        "Max-Age=43200",
    ] {
        assert!(set_cookie.contains(attr), "cookie sem {attr}: {set_cookie}");
    }
    let cookie = format!("oh_session={}", session_cookie_value(&set_cookie));
    let me = raw(&app, "GET", "/v1/me", &[("cookie", cookie.clone())], None).await;
    assert_eq!(
        me.status(),
        StatusCode::OK,
        "sessão por cookie vale para leitura"
    );
    // escrita por cookie: origem precisa ser a mesma (Host) ou autorizada
    let body = Some(json!({"title": "Cookie"}));
    let no_origin = raw(
        &app,
        "POST",
        "/v1/conversations",
        &[
            ("cookie", cookie.clone()),
            ("host", "hub.teste.local".into()),
        ],
        body.clone(),
    )
    .await;
    assert_eq!(no_origin.status(), StatusCode::FORBIDDEN);
    let foreign = raw(
        &app,
        "POST",
        "/v1/conversations",
        &[
            ("cookie", cookie.clone()),
            ("host", "hub.teste.local".into()),
            ("origin", "https://evil.example".into()),
        ],
        body.clone(),
    )
    .await;
    assert_eq!(foreign.status(), StatusCode::FORBIDDEN);
    let same = raw(
        &app,
        "POST",
        "/v1/conversations",
        &[
            ("cookie", cookie.clone()),
            ("host", "hub.teste.local".into()),
            ("origin", "https://hub.teste.local".into()),
        ],
        body.clone(),
    )
    .await;
    assert_eq!(same.status(), StatusCode::CREATED);
    // cliente que não é navegador (bearer) não depende de origem
    let bearer = raw(
        &app,
        "POST",
        "/v1/conversations",
        &[(
            "authorization",
            format!("Bearer {}", session_cookie_value(&set_cookie)),
        )],
        body,
    )
    .await;
    assert_eq!(bearer.status(), StatusCode::CREATED);
    // logout limpa o cookie e revoga a sessão
    let out = raw(
        &app,
        "POST",
        "/v1/logout",
        &[
            ("cookie", cookie.clone()),
            ("host", "hub.teste.local".into()),
            ("origin", "https://hub.teste.local".into()),
        ],
        None,
    )
    .await;
    assert_eq!(out.status(), StatusCode::NO_CONTENT);
    assert!(out.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .contains("Max-Age=0"));
    let after = raw(&app, "GET", "/v1/me", &[("cookie", cookie)], None).await;
    assert_eq!(after.status(), StatusCode::UNAUTHORIZED);
    let _ = member;
}

#[tokio::test]
#[ignore = "exige ONEAGENT_CENTRAL_TEST_DATABASE_URL (banco _test): rodar com --ignored --test-threads=1"]
async fn cancel_is_immediate_when_queued_and_confirmed_by_executor_when_running() {
    let (app, pool) = fresh_app().await;
    let (admin, member) = admin_and_member(&app, &pool).await;
    let (nid, pid) = node_with_project(&app, &admin, "Projeto cancelável").await;
    let ex = paired_executor(&app, &admin).await;
    let cid = new_conversation(&app, &member).await;
    let turn = |text: &str| json!({"text": text, "node_id": nid, "project_id": pid});

    // job na fila: cancelamento imediato, sem executor
    let (_, r) = call(
        &app,
        "POST",
        &format!("/v1/conversations/{cid}/turns"),
        Some(&member),
        Some(turn("Analise A")),
    )
    .await;
    let queued = r["assistant_message"]["job_id"]
        .as_str()
        .unwrap()
        .to_string();
    let (s, _) = call(
        &app,
        "POST",
        &format!("/v1/jobs/{queued}/cancel"),
        Some(&member),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::ACCEPTED);
    let (_, j) = call(
        &app,
        "GET",
        &format!("/v1/jobs/{queued}"),
        Some(&member),
        None,
    )
    .await;
    assert_eq!(j["status"], "cancelled");
    let (_, st) = call(
        &app,
        "GET",
        &format!("/v1/conversations/{cid}/state"),
        Some(&member),
        None,
    )
    .await;
    let msg = st["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["job_id"] == json!(queued))
        .unwrap();
    assert_eq!(msg["state"], "cancelled");

    // job em execução: o pedido não vira cancelado sem a confirmação do executor
    let (_, r) = call(
        &app,
        "POST",
        &format!("/v1/conversations/{cid}/turns"),
        Some(&member),
        Some(turn("Analise B")),
    )
    .await;
    let running = r["assistant_message"]["job_id"]
        .as_str()
        .unwrap()
        .to_string();
    let (_, n) = call(&app, "POST", "/executor/v1/jobs/next", Some(&ex), None).await;
    assert_eq!(n["id"], json!(running));
    let lease = n["lease_token"].as_str().unwrap().to_string();
    // antes do pedido: finalizar como cancelado é recusado
    let (s, _) = call(
        &app,
        "POST",
        &format!("/executor/v1/jobs/{running}/finish"),
        Some(&ex),
        Some(json!({"lease_token": lease, "status": "cancelled"})),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT);
    let (s, _) = call(
        &app,
        "POST",
        &format!("/v1/jobs/{running}/cancel"),
        Some(&member),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::ACCEPTED);
    let (_, j) = call(
        &app,
        "GET",
        &format!("/v1/jobs/{running}"),
        Some(&member),
        None,
    )
    .await;
    assert_eq!(j["status"], "running", "ainda não cancelado");
    assert_eq!(j["cancel_requested"], true);
    // o executor percebe pelo heartbeat, e o progresso informa o cancelamento
    let (_, hb) = call(
        &app,
        "POST",
        &format!("/executor/v1/jobs/{running}/heartbeat"),
        Some(&ex),
        Some(json!({"lease_token": lease})),
    )
    .await;
    assert_eq!(hb["cancel_requested"], true);
    let (_, ev) = call(
        &app,
        "POST",
        &format!("/executor/v1/jobs/{running}/events"),
        Some(&ex),
        Some(
            json!({"lease_token": lease, "kind": "tool", "payload": {"summary": "cat README.md"}}),
        ),
    )
    .await;
    assert_eq!(ev["cancel_requested"], true);
    let (s, _) = call(
        &app,
        "POST",
        &format!("/executor/v1/jobs/{running}/finish"),
        Some(&ex),
        Some(json!({"lease_token": lease, "status": "cancelled"})),
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (_, j) = call(
        &app,
        "GET",
        &format!("/v1/jobs/{running}"),
        Some(&member),
        None,
    )
    .await;
    assert_eq!(j["status"], "cancelled");
    assert_eq!(
        j["events"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["kind"] == "tool")
            .count(),
        1
    );
    // job finalizado não aceita novo cancelamento
    let (s, _) = call(
        &app,
        "POST",
        &format!("/v1/jobs/{running}/cancel"),
        Some(&member),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT);
}

#[tokio::test]
#[ignore = "exige ONEAGENT_CENTRAL_TEST_DATABASE_URL (banco _test): rodar com --ignored --test-threads=1"]
async fn project_is_chosen_by_name_only_among_authorized_projects() {
    let (app, pool) = fresh_app().await;
    let (admin, member) = admin_and_member(&app, &pool).await;
    // análise exige OneExecutor online (sem ele, nada é enfileirado)
    let _executor = paired_executor(&app, &admin).await;
    let (nid, alfa) = node_with_project(&app, &admin, "Alfa Sistema").await;
    let (_, b) = call(
        &app,
        "POST",
        "/v1/projects",
        Some(&admin),
        Some(json!({"name": "Beta"})),
    )
    .await;
    let beta = b["id"].as_str().unwrap().to_string();
    call(
        &app,
        "POST",
        &format!("/v1/projects/{beta}/grants"),
        Some(&admin),
        Some(json!({"node_id": nid, "root_path": "/srv/teste"})),
    )
    .await;
    // nome não autorizado não resolve nada: vira pedido de escolha, sem job
    let cid = new_conversation(&app, &member).await;
    let (_, r) = call(
        &app,
        "POST",
        &format!("/v1/conversations/{cid}/turns"),
        Some(&member),
        Some(json!({"text": "Analise o Gama", "node_id": nid})),
    )
    .await;
    assert_eq!(r["assistant_message"]["state"], "rejected");
    // nome autorizado (sem escolha explícita) vale
    let (_, r) = call(
        &app,
        "POST",
        &format!("/v1/conversations/{cid}/turns"),
        Some(&member),
        Some(json!({"text": "Analise o desempenho do beta", "node_id": nid})),
    )
    .await;
    let job = r["assistant_message"]["job_id"]
        .as_str()
        .unwrap()
        .to_string();
    let (_, j) = call(&app, "GET", &format!("/v1/jobs/{job}"), Some(&member), None).await;
    assert_eq!(j["project_id"], json!(beta));
    // dois projetos citados: pede a escolha, sem job
    let cid2 = new_conversation(&app, &member).await;
    let (_, r) = call(
        &app,
        "POST",
        &format!("/v1/conversations/{cid2}/turns"),
        Some(&member),
        Some(json!({"text": "Compare Alfa Sistema com Beta", "node_id": nid})),
    )
    .await;
    assert_eq!(r["assistant_message"]["state"], "rejected");
    assert!(r["assistant_message"]["job_id"].is_null());
    // escolha explícita vence o nome citado
    let (_, r) = call(
        &app,
        "POST",
        &format!("/v1/conversations/{cid2}/turns"),
        Some(&member),
        Some(json!({"text": "Veja o Beta", "node_id": nid, "project_id": alfa})),
    )
    .await;
    let job = r["assistant_message"]["job_id"]
        .as_str()
        .unwrap()
        .to_string();
    let (_, j) = call(&app, "GET", &format!("/v1/jobs/{job}"), Some(&member), None).await;
    assert_eq!(j["project_id"], json!(alfa));
}

#[tokio::test]
#[ignore = "exige ONEAGENT_CENTRAL_TEST_DATABASE_URL (banco _test): rodar com --ignored --test-threads=1"]
async fn login_is_limited_per_origin_across_accounts_and_keeps_account_lockout() {
    let (app, pool) = fresh_app().await;
    let _ = admin_and_member(&app, &pool).await;
    // varredura: muitos e-mails diferentes a partir da mesma origem (sem ConnectInfo nos testes: origem fixa)
    for i in 0..30 {
        let (s, _) = call(
            &app,
            "POST",
            "/v1/login",
            None,
            Some(json!({"email": format!("x{i}@teste.local"), "password": "senha-errada-123"})),
        )
        .await;
        assert_eq!(s, StatusCode::UNAUTHORIZED);
    }
    // a origem está bloqueada: nem a senha certa de outra conta passa
    let (s, _) = call(
        &app,
        "POST",
        "/v1/login",
        None,
        Some(json!({"email": MEMBER_EMAIL, "password": MEMBER_PASS})),
    )
    .await;
    assert_eq!(s, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
#[ignore = "exige ONEAGENT_CENTRAL_TEST_DATABASE_URL (banco _test): rodar com --ignored --test-threads=1"]
async fn revoking_a_grant_stops_a_running_job_from_requesting_more_tools() {
    let (app, pool) = fresh_app().await;
    let (admin, member) = admin_and_member(&app, &pool).await;
    let (nid, pid) = node_with_project(&app, &admin, "Projeto revogado em execução").await;
    let ex = paired_executor(&app, &admin).await;
    let cid = new_conversation(&app, &member).await;
    let (_, r) = call(
        &app,
        "POST",
        &format!("/v1/conversations/{cid}/turns"),
        Some(&member),
        Some(json!({"text": "Analise", "node_id": nid, "project_id": pid})),
    )
    .await;
    let job = r["assistant_message"]["job_id"]
        .as_str()
        .unwrap()
        .to_string();
    let (_, n) = call(&app, "POST", "/executor/v1/jobs/next", Some(&ex), None).await;
    let lease = n["lease_token"].as_str().unwrap().to_string();
    // antes da revogação, uma ferramenta é aceita
    let (s, _) = call(
        &app,
        "POST",
        &format!("/executor/v1/jobs/{job}/tools"),
        Some(&ex),
        Some(json!({"lease_token": lease, "tool": "files_list", "path": "."})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    let (s, _) = call(
        &app,
        "POST",
        &format!("/v1/projects/{pid}/grants/{nid}/revoke"),
        Some(&admin),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    // o job em execução recebe o pedido de cancelamento e nenhuma ferramenta nova é criada
    let (_, j) = call(&app, "GET", &format!("/v1/jobs/{job}"), Some(&member), None).await;
    assert_eq!(j["cancel_requested"], true);
    let (s, body) = call(
        &app,
        "POST",
        &format!("/executor/v1/jobs/{job}/tools"),
        Some(&ex),
        Some(json!({"lease_token": lease, "tool": "files_list", "path": "."})),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT, "{body}");
    // o executor confirma o cancelamento
    let (s, _) = call(
        &app,
        "POST",
        &format!("/executor/v1/jobs/{job}/finish"),
        Some(&ex),
        Some(json!({"lease_token": lease, "status": "cancelled"})),
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (_, j) = call(&app, "GET", &format!("/v1/jobs/{job}"), Some(&member), None).await;
    assert_eq!(j["status"], "cancelled");
}

#[tokio::test]
#[ignore = "exige ONEAGENT_CENTRAL_TEST_DATABASE_URL (banco _test): rodar com --ignored --test-threads=1"]
async fn browser_login_never_returns_the_session_token_in_the_body() {
    let (app, pool) = fresh_app().await;
    let _ = admin_and_member(&app, &pool).await;
    let creds = json!({"email": MEMBER_EMAIL, "password": MEMBER_PASS});
    // navegador (envia Origin): só o cookie HttpOnly
    let browser = raw(
        &app,
        "POST",
        "/v1/login",
        &[
            ("origin", "https://hub.teste.local".into()),
            ("host", "hub.teste.local".into()),
        ],
        Some(creds.clone()),
    )
    .await;
    assert_eq!(browser.status(), StatusCode::OK);
    assert!(browser.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .contains("HttpOnly"));
    let body = json_of(browser).await;
    assert!(
        body.get("token").is_none(),
        "token não pode ir ao JavaScript do navegador: {body}"
    );
    // cliente sem navegador (sem Origin): o token continua disponível, como antes
    let api = raw(&app, "POST", "/v1/login", &[], Some(creds)).await;
    assert!(json_of(api).await.get("token").is_some());
}

async fn json_of(res: axum::response::Response) -> Value {
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

#[tokio::test]
#[ignore = "banco PostgreSQL de teste na VPS (ver cabeçalho)"]
async fn node_catalog_follows_grants_roots_suspension_and_applied_versions() {
    let (app, pool) = fresh_app().await;
    let (admin, member) = admin_and_member(&app, &pool).await;

    // OneNode sem projetos: pareia e recebe um catálogo vazio, válido
    let (_, node) = call(&app, "POST", "/v1/onenodes", Some(&admin), Some(json!({"name": "Nó catálogo"}))).await;
    let nid = node["id"].as_str().unwrap().to_string();
    let (s, paired) = call(
        &app,
        "POST",
        "/node/v1/pair",
        None,
        Some(json!({"node_id": nid, "code": node["pairing_code"]})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let nt = paired["token"].as_str().unwrap().to_string();
    let (s, cat) = call(&app, "GET", "/node/v1/catalog", Some(&nt), None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(cat["projects"].as_array().unwrap().is_empty(), "zero projetos é válido");
    let v0 = cat["version"].as_i64().unwrap();

    // raízes inválidas são recusadas e não mudam o catálogo
    let (_, pr) = call(&app, "POST", "/v1/projects", Some(&admin), Some(json!({"name": "ERP"}))).await;
    let pid = pr["id"].as_str().unwrap().to_string();
    for bad in ["relativo/caminho", "/a/../etc", "/", ""] {
        let (s, _) = call(
            &app,
            "POST",
            &format!("/v1/projects/{pid}/grants"),
            Some(&admin),
            Some(json!({"node_id": nid, "root_path": bad})),
        )
        .await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "raiz {bad:?} deve ser recusada");
    }
    let (_, cat) = call(&app, "GET", "/node/v1/catalog", Some(&nt), None).await;
    assert_eq!(cat["version"].as_i64().unwrap(), v0, "raiz recusada não altera o catálogo");

    // concessão válida: entra no catálogo e a versão sobe
    let (s, _) = call(
        &app,
        "POST",
        &format!("/v1/projects/{pid}/grants"),
        Some(&admin),
        Some(json!({"node_id": nid, "root_path": "/opt/onenode/workspace/teste"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    let (_, cat) = call(&app, "GET", "/node/v1/catalog", Some(&nt), None).await;
    let v1 = cat["version"].as_i64().unwrap();
    assert!(v1 > v0, "a versão sobe a cada mudança");
    assert_eq!(cat["projects"][0]["project_id"], json!(pid));
    assert_eq!(cat["projects"][0]["root_path"], json!("/opt/onenode/workspace/teste"));

    // relatório com versão antiga é recusado; a versão atual é aceita
    let (s, _) = call(
        &app,
        "POST",
        "/node/v1/catalog/applied",
        Some(&nt),
        Some(json!({"version": v0, "accepted": [], "rejected": []})),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT);
    let (s, _) = call(
        &app,
        "POST",
        "/node/v1/catalog/applied",
        Some(&nt),
        Some(json!({"version": v1, "accepted": [pid], "rejected": []})),
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (_, list) = call(&app, "GET", "/v1/onenodes", Some(&admin), None).await;
    let me = list.as_array().unwrap().iter().find(|n| n["id"] == json!(nid)).unwrap();
    assert_eq!(me["catalog_in_sync"], json!(true));

    // revogar a concessão tira o projeto do catálogo e sobe a versão
    let (s, _) = call(&app, "POST", &format!("/v1/projects/{pid}/grants/{nid}/revoke"), Some(&admin), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (_, cat) = call(&app, "GET", "/node/v1/catalog", Some(&nt), None).await;
    assert!(cat["projects"].as_array().unwrap().is_empty());
    assert!(cat["version"].as_i64().unwrap() > v1);

    // membro não suspende; administrador suspende: o nó recebe 423 sem perder o token
    let (s, _) = call(&app, "POST", &format!("/v1/onenodes/{nid}/suspend"), Some(&member), None).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = call(&app, "POST", &format!("/v1/onenodes/{nid}/suspend"), Some(&admin), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, body) = call(&app, "GET", "/node/v1/catalog", Some(&nt), None).await;
    assert_eq!(s, StatusCode::LOCKED);
    assert_eq!(body["error"], json!("node_suspended"));
    let (s, _) = call(&app, "POST", "/node/v1/heartbeat", Some(&nt), None).await;
    assert_eq!(s, StatusCode::LOCKED);

    // retomar restaura o acesso com o mesmo token, sem novo pareamento
    let (s, _) = call(&app, "POST", &format!("/v1/onenodes/{nid}/resume"), Some(&admin), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _) = call(&app, "POST", "/node/v1/heartbeat", Some(&nt), None).await;
    assert_eq!(s, StatusCode::OK);

    // revogar o nó encerra o acesso de vez
    let (s, _) = call(&app, "POST", &format!("/v1/onenodes/{nid}/revoke"), Some(&admin), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _) = call(&app, "GET", "/node/v1/catalog", Some(&nt), None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
#[ignore = "banco PostgreSQL de teste na VPS (ver cabeçalho)"]
async fn node_detail_is_admin_only_and_shows_grants_roots_and_sync_state() {
    let (app, pool) = fresh_app().await;
    let (admin, member) = admin_and_member(&app, &pool).await;
    let (nid, pid) = node_with_project(&app, &admin, "Projeto Detalhe").await;

    // membro não vê o detalhe administrativo
    let (s, _) = call(&app, "GET", &format!("/v1/onenodes/{nid}"), Some(&member), None).await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    // administrador vê concessão com a raiz pedida e o catálogo ainda não aplicado
    let (s, d) = call(&app, "GET", &format!("/v1/onenodes/{nid}"), Some(&admin), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(d["catalog_in_sync"], json!(false));
    assert_eq!(d["grants"][0]["project_id"], json!(pid));
    assert_eq!(d["grants"][0]["root_path"], json!("/srv/teste"));

    // a listagem de concessões também traz a raiz
    let (_, g) = call(&app, "GET", "/v1/grants", Some(&admin), None).await;
    assert_eq!(g[0]["root_path"], json!("/srv/teste"));

    // inexistente: 404
    let (s, _) = call(&app, "GET", &format!("/v1/onenodes/{}", uuid::Uuid::nil()), Some(&admin), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
#[ignore = "banco PostgreSQL de teste na VPS (ver cabeçalho)"]
async fn suspended_node_refuses_new_tasks_and_recovers_after_resume() {
    let (app, pool) = fresh_app().await;
    let (admin, _member) = admin_and_member(&app, &pool).await;
    let (nid, pid) = node_with_project(&app, &admin, "Projeto suspenso").await;
    let task = json!({
        "node_id": nid, "project_id": pid, "tool": "files_list", "args": {"path": "."},
        "prompt": "listar", "idempotency_key": "suspensao-0001"
    });
    let (s, _) = call(&app, "POST", &format!("/v1/onenodes/{nid}/suspend"), Some(&admin), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, body) = call(&app, "POST", "/v1/tasks", Some(&admin), Some(task.clone())).await;
    assert_eq!(s, StatusCode::LOCKED);
    assert_eq!(body["error"], json!("node_suspended"));
    let (s, _) = call(&app, "POST", &format!("/v1/onenodes/{nid}/resume"), Some(&admin), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _) = call(&app, "POST", "/v1/tasks", Some(&admin), Some(json!({
        "node_id": nid, "project_id": pid, "tool": "files_list", "args": {"path": "."},
        "prompt": "listar", "idempotency_key": "retomada-0001"
    }))).await;
    assert_eq!(s, StatusCode::CREATED);
}

#[tokio::test]
#[ignore = "banco PostgreSQL de teste na VPS (ver cabeçalho)"]
async fn chat_context_is_explicit_per_conversation_and_never_promises_unavailable_execution() {
    let (app, pool) = fresh_app().await;
    let (admin, member) = admin_and_member(&app, &pool).await;
    let (nid, pid) = node_with_project(&app, &admin, "Projeto contexto").await;
    let cid = new_conversation(&app, &member).await;
    let turn = |text: &str, node: Option<&str>, project: Option<&str>| json!({"text": text, "node_id": node, "project_id": project});

    // contexto OneHub Central: persistido, sem OneNode, e a análise é recusada com a mensagem do contexto
    let (s, _) = call(&app, "PUT", &format!("/v1/conversations/{cid}/context"), Some(&member), Some(json!({"node_id": null, "project_id": null}))).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, r) = call(&app, "POST", &format!("/v1/conversations/{cid}/turns"), Some(&member), Some(turn("analise o projeto", None, None))).await;
    assert_eq!(s, StatusCode::CREATED);
    assert_eq!(r["assistant_message"]["state"], "rejected");
    assert!(r["assistant_message"]["body"].as_str().unwrap().contains("OneHub Central"));
    assert!(r["assistant_message"]["job_id"].is_null());

    // sem OneExecutor online: a conversa recusa com motivo claro e NÃO cria job
    let (s, _) = call(&app, "PUT", &format!("/v1/conversations/{cid}/context"), Some(&member), Some(json!({"node_id": nid, "project_id": pid}))).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, r) = call(&app, "POST", &format!("/v1/conversations/{cid}/turns"), Some(&member), Some(turn("analise", None, None))).await;
    assert_eq!(s, StatusCode::CREATED);
    assert_eq!(r["assistant_message"]["state"], "rejected");
    assert!(r["assistant_message"]["body"].as_str().unwrap().contains("OneExecutor"));
    assert!(r["assistant_message"]["job_id"].is_null());
    let jobs: i64 = pool.get().await.unwrap().query_one("SELECT count(*) FROM jobs", &[]).await.unwrap().get(0);
    assert_eq!(jobs, 0, "nenhum job pode ficar órfão");

    // persistência: o contexto volta na listagem de conversas, sem depender do cliente
    let (_, list) = call(&app, "GET", "/v1/conversations", Some(&member), None).await;
    let mine = list.as_array().unwrap().iter().find(|c| c["id"] == json!(cid)).unwrap();
    assert_eq!(mine["node_id"], json!(nid));
    assert_eq!(mine["project_id"], json!(pid));

    // dono da conversa: o administrador não altera a conversa de outro usuário
    let (s, _) = call(&app, "PUT", &format!("/v1/conversations/{cid}/context"), Some(&admin), Some(json!({"node_id": null, "project_id": null}))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // projeto sem concessão não pode ser escolhido no contexto
    let (_, p2) = call(&app, "POST", "/v1/projects", Some(&admin), Some(json!({"name": "Sem concessão"}))).await;
    let pid2 = p2["id"].as_str().unwrap().to_string();
    let (s, _) = call(&app, "PUT", &format!("/v1/conversations/{cid}/context"), Some(&member), Some(json!({"node_id": nid, "project_id": pid2}))).await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    // OneNode sem projetos: contexto válido, e a mensagem indica a página OneNodes
    let (_, n2) = call(&app, "POST", "/v1/onenodes", Some(&admin), Some(json!({"name": "Nó sem projetos"}))).await;
    let n2id = n2["id"].as_str().unwrap().to_string();
    let (s, _) = call(&app, "PUT", &format!("/v1/conversations/{cid}/context"), Some(&member), Some(json!({"node_id": n2id, "project_id": null}))).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (_, r) = call(&app, "POST", &format!("/v1/conversations/{cid}/turns"), Some(&member), Some(turn("analise", None, None))).await;
    assert!(r["assistant_message"]["body"].as_str().unwrap().contains("não tem projetos autorizados"));

    // OneNode suspenso: a análise fica indisponível com motivo, sem job
    let (s, _) = call(&app, "POST", &format!("/v1/onenodes/{nid}/suspend"), Some(&admin), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (_, r) = call(&app, "POST", &format!("/v1/conversations/{cid}/turns"), Some(&member), Some(turn("analise", Some(&nid), Some(&pid)))).await;
    assert!(r["assistant_message"]["body"].as_str().unwrap().contains("suspenso"));
    assert!(r["assistant_message"]["job_id"].is_null());
    let (_, _) = call(&app, "POST", &format!("/v1/onenodes/{nid}/resume"), Some(&admin), None).await;

    // OneNode revogado não pode ser escolhido
    let (s, _) = call(&app, "POST", &format!("/v1/onenodes/{n2id}/revoke"), Some(&admin), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, body) = call(&app, "PUT", &format!("/v1/conversations/{cid}/context"), Some(&member), Some(json!({"node_id": n2id, "project_id": null}))).await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(body["error"], json!("node_revoked"));
}

#[tokio::test]
#[ignore = "banco PostgreSQL de teste na VPS (ver cabeçalho)"]
async fn analysis_is_created_only_when_a_paired_executor_is_online() {
    let (app, pool) = fresh_app().await;
    let (admin, member) = admin_and_member(&app, &pool).await;
    let (nid, pid) = node_with_project(&app, &admin, "Projeto executor").await;
    let _executor = paired_executor(&app, &admin).await;
    let cid = new_conversation(&app, &member).await;
    let (s, _) = call(&app, "PUT", &format!("/v1/conversations/{cid}/context"), Some(&member), Some(json!({"node_id": nid, "project_id": pid}))).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, r) = call(&app, "POST", &format!("/v1/conversations/{cid}/turns"), Some(&member), Some(json!({"text": "analise o projeto", "node_id": null, "project_id": null}))).await;
    assert_eq!(s, StatusCode::CREATED);
    assert_eq!(r["assistant_message"]["state"], "task");
    assert!(!r["assistant_message"]["job_id"].is_null(), "com executor online, o job é criado");
}

#[tokio::test]
#[ignore = "banco PostgreSQL de teste na VPS (ver cabeçalho)"]
async fn cors_preflight_allows_the_methods_the_web_uses_from_an_allowed_origin() {
    use one_central::routes::{router_with_settings, Settings};
    let pool = db::connect(&test_url()).unwrap();
    {
        let client = pool.get().await.unwrap();
        client.batch_execute("DROP SCHEMA public CASCADE; CREATE SCHEMA public;").await.unwrap();
    }
    db::migrate(&pool).await.expect("migrations");
    let origin = "https://app.exemplo.test";
    let settings = Settings {
        allowed_origins: vec![origin.to_string()],
        ..Settings::default()
    };
    let app = router_with_settings(pool, std::sync::Arc::new(one_central::orchestrator::UnavailableProvider), settings);
    // o navegador pede permissão antes de um PUT (contexto do chat); sem ela, a persistência falha em silêncio
    let req = Request::builder()
        .method("OPTIONS")
        .uri("/v1/conversations/00000000-0000-0000-0000-000000000000/context")
        .header("origin", origin)
        .header("access-control-request-method", "PUT")
        .header("access-control-request-headers", "content-type")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let allow_methods = res
        .headers()
        .get("access-control-allow-methods")
        .map(|v| v.to_str().unwrap().to_string())
        .unwrap_or_default();
    assert!(allow_methods.contains("PUT"), "PUT precisa estar liberado no CORS: {allow_methods:?}");
    assert_eq!(res.headers().get("access-control-allow-origin").map(|v| v.to_str().unwrap()), Some(origin));
}

#[tokio::test]
#[ignore = "banco PostgreSQL de teste na VPS (ver cabeçalho)"]
async fn delegation_is_blocked_without_a_ready_engine_and_creates_no_job() {
    let (app, pool) = fresh_app().await;
    let (admin, member) = admin_and_member(&app, &pool).await;
    let cid = new_conversation(&app, &member).await;
    // executor online mas com o motor Codex NÃO autenticado: online não basta
    let (_, ex) = call(&app, "POST", "/v1/executors", Some(&admin), Some(json!({"name": "Sem login"}))).await;
    let (_, p) = call(&app, "POST", "/executor/v1/pair", None, Some(json!({"executor_id": ex["id"], "code": ex["pairing_code"]}))).await;
    let tok = p["token"].as_str().unwrap().to_string();
    call(&app, "POST", "/executor/v1/heartbeat", Some(&tok),
        Some(json!([{"name": "codex", "installed": true, "authenticated": false, "approved": true}]))).await;
    let (s, r) = call(&app, "POST", "/v1/delegations", Some(&member),
        Some(json!({"conversation_id": cid, "intent": "analise este texto: o céu é azul"}))).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(r["status"], json!("blocked"));
    assert!(r["reason"].as_str().unwrap().contains("ferramenta") || r["reason"].as_str().unwrap().contains("motor"));
    let jobs: i64 = pool.get().await.unwrap().query_one("SELECT count(*) FROM jobs", &[]).await.unwrap().get(0);
    assert_eq!(jobs, 0, "bloqueio não cria job");
    // executor não recebe nada
    let (s, _) = call(&app, "POST", "/executor/v1/jobs/next", Some(&tok), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
}

#[tokio::test]
#[ignore = "banco PostgreSQL de teste na VPS (ver cabeçalho)"]
async fn ready_delegation_is_delivered_structured_and_finished_with_the_result() {
    let (app, pool) = fresh_app().await;
    let (admin, member) = admin_and_member(&app, &pool).await;
    let cid = new_conversation(&app, &member).await;
    let ex_tok = paired_executor(&app, &admin).await;
    let (s, r) = call(&app, "POST", "/v1/delegations", Some(&member),
        Some(json!({"conversation_id": cid, "intent": "analise este texto: a sala é azul"}))).await;
    assert_eq!(s, StatusCode::CREATED, "{r}");
    assert_eq!(r["status"], json!("queued"));
    let job = r["job_id"].as_str().unwrap().to_string();
    // o executor recebe o job de delegação com o prompt estruturado (instruções, tarefa e critérios)
    let (s, lease) = call(&app, "POST", "/executor/v1/jobs/next", Some(&ex_tok), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(lease["id"].as_str(), Some(job.as_str()));
    assert_eq!(lease["kind"], json!("delegation"));
    assert!(lease["node_id"].is_null() && lease["project_id"].is_null(), "delegação não tem nó nem projeto");
    let q = lease["question"].as_str().unwrap();
    assert!(q.contains("Análise de texto") || q.contains("text-analyst") || q.contains("somente leitura"));
    assert!(q.contains("## Tarefa") && q.contains("a sala é azul") && q.contains("## Critérios de aceite"));
    let lease_tok = lease["lease_token"].as_str().unwrap().to_string();
    // conclusão com o resultado: o OneHub guarda a resposta e marca o job como concluído
    let (s, _) = call(&app, "POST", &format!("/executor/v1/jobs/{job}/finish"), Some(&ex_tok),
        Some(json!({"lease_token": lease_tok, "status": "succeeded", "answer": "A frase afirma que a sala é azul."}))).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (_, j) = call(&app, "GET", &format!("/v1/jobs/{job}"), Some(&member), None).await;
    assert_eq!(j["status"], json!("succeeded"));
    assert_eq!(j["answer"], json!("A frase afirma que a sala é azul."));
}

#[tokio::test]
#[ignore = "banco PostgreSQL de teste na VPS (ver cabeçalho)"]
async fn delegation_refuses_unknown_intents_and_invalid_engine_reports() {
    let (app, pool) = fresh_app().await;
    let (admin, member) = admin_and_member(&app, &pool).await;
    let cid = new_conversation(&app, &member).await;
    let _ex = paired_executor(&app, &admin).await;
    let (s, r) = call(&app, "POST", "/v1/delegations", Some(&member),
        Some(json!({"conversation_id": cid, "intent": "faça um bolo de cenoura"}))).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(r["status"], json!("no_agent"));
    // relatório com motor desconhecido é recusado: o executor não pode inventar motores
    let (_, ex2) = call(&app, "POST", "/v1/executors", Some(&admin), Some(json!({"name": "Inválido"}))).await;
    let (_, p2) = call(&app, "POST", "/executor/v1/pair", None, Some(json!({"executor_id": ex2["id"], "code": ex2["pairing_code"]}))).await;
    let tok2 = p2["token"].as_str().unwrap().to_string();
    let (s, body) = call(&app, "POST", "/executor/v1/heartbeat", Some(&tok2),
        Some(json!([{"name": "gpt-x", "installed": true, "authenticated": true, "approved": true}]))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], json!("engines_invalid"));
}


fn app_with_provider_key(pool: db::DbPool) -> axum::Router {
    let settings = one_central::routes::Settings {
        provider_key: Some([42u8; 32]),
        ..one_central::routes::Settings::default()
    };
    one_central::routes::router_with_settings(
        pool,
        std::sync::Arc::new(one_central::orchestrator::UnavailableProvider),
        settings,
    )
}

#[tokio::test]
#[ignore = "banco PostgreSQL de teste na VPS (ver cabeçalho)"]
async fn provider_connections_store_only_ciphertext_and_never_return_the_key() {
    const PLAIN_KEY: &str = "sk-or-v1-chave-de-teste-ABCD1234";
    let (_, pool) = fresh_app().await;
    let (admin, member) = admin_and_member(&router(pool.clone()), &pool).await;

    // sem chave-mestra: o serviço continua de pé e responde 503 explícito
    let (s, body) = call(
        &router(pool.clone()),
        "POST",
        "/v1/provider-connections",
        Some(&admin),
        Some(json!({"name": "OpenRouter", "provider": "openrouter", "api_key": PLAIN_KEY})),
    )
    .await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["error"], "provider_key_missing");

    let app = app_with_provider_key(pool.clone());
    let (s, body) = call(
        &app,
        "POST",
        "/v1/provider-connections",
        Some(&admin),
        Some(json!({"name": "OpenRouter", "provider": "openrouter", "api_key": PLAIN_KEY, "default_model": "openai/gpt-4o-mini"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    assert_eq!(body["key_hint"], "••••1234");
    assert!(!body.to_string().contains(PLAIN_KEY), "a resposta nunca devolve a chave");
    let id = body["id"].as_str().unwrap().to_string();

    // o banco guarda só o texto cifrado: a chave em claro não aparece em nenhuma coluna
    let client = pool.get().await.unwrap();
    let row = client
        .query_one(
            "SELECT key_ciphertext, key_nonce FROM provider_connections WHERE id::text = $1",
            &[&id],
        )
        .await
        .unwrap();
    let ct: Vec<u8> = row.get(0);
    assert!(!ct.windows(PLAIN_KEY.len()).any(|w| w == PLAIN_KEY.as_bytes()));
    let nonce: Vec<u8> = row.get(1);
    assert_eq!(
        one_central::provider_keys::open(&[42u8; 32], "openrouter", &ct, &nonce).unwrap(),
        PLAIN_KEY
    );
    drop(client);

    // validação de entrada
    let (s, b) = call(&app, "POST", "/v1/provider-connections", Some(&admin),
        Some(json!({"name": "X", "provider": "groq", "api_key": PLAIN_KEY}))).await;
    assert_eq!((s, b["error"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_provider")));
    let (s, b) = call(&app, "POST", "/v1/provider-connections", Some(&admin),
        Some(json!({"name": "X", "provider": "openai", "api_key": "curta"}))).await;
    assert_eq!((s, b["error"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_api_key")));

    // listagem: só dica, sem segredo; membro comum não acessa
    let (s, list) = call(&app, "GET", "/v1/provider-connections", Some(&admin), None).await;
    assert_eq!(s, StatusCode::OK);
    let text = list.to_string();
    assert!(text.contains("••••1234") && !text.contains(PLAIN_KEY));
    let (s, _) = call(&app, "GET", "/v1/provider-connections", Some(&member), None).await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    // remoção: primeira remove, segunda é 404
    let (s, _) = call(&app, "DELETE", &format!("/v1/provider-connections/{id}"), Some(&admin), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _) = call(&app, "DELETE", &format!("/v1/provider-connections/{id}"), Some(&admin), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

async fn cli_executor(app: &axum::Router, admin: &str) -> (String, String) {
    let (_, ex) = call(app, "POST", "/v1/executors", Some(admin), Some(json!({"name": "Executor CLI"}))).await;
    let eid = ex["id"].as_str().unwrap().to_string();
    let (_, p) = call(app, "POST", "/executor/v1/pair", None, Some(json!({"executor_id": eid, "code": ex["pairing_code"]}))).await;
    let token = p["token"].as_str().unwrap().to_string();
    let (s, _) = call(app, "POST", "/executor/v1/heartbeat", Some(&token), Some(json!([]))).await;
    assert_eq!(s, StatusCode::OK);
    (eid, token)
}

#[tokio::test]
#[ignore = "banco PostgreSQL de teste na VPS (ver cabeçalho)"]
async fn cli_accounts_login_through_the_executor_with_isolated_scope_and_no_secret_leak() {
    let (app, pool) = fresh_app().await;
    let (admin, member) = admin_and_member(&app, &pool).await;
    let (eid, ex_token) = cli_executor(&app, &admin).await;

    // três contas Claude separadas, cada uma com o próprio estado
    for name in ["Claude Empresa 01", "Claude Empresa 02"] {
        let (s, _) = call(&app, "POST", "/v1/cli-connections", Some(&admin),
            Some(json!({"name": name, "engine": "claude", "executor_id": eid, "usage_authorized": true}))).await;
        assert_eq!(s, StatusCode::CREATED);
    }
    // nome repetido na mesma máquina é recusado; uso não declarado também
    let (s, b) = call(&app, "POST", "/v1/cli-connections", Some(&admin),
        Some(json!({"name": "Claude Empresa 01", "engine": "claude", "executor_id": eid, "usage_authorized": true}))).await;
    assert_eq!((s, b["error"].as_str()), (StatusCode::CONFLICT, Some("name_in_use")));
    let (s, b) = call(&app, "POST", "/v1/cli-connections", Some(&admin),
        Some(json!({"name": "Claude Empresa 03", "engine": "claude", "executor_id": eid}))).await;
    assert_eq!((s, b["error"].as_str()), (StatusCode::BAD_REQUEST, Some("usage_not_authorized")));

    let (_, list) = call(&app, "GET", "/v1/cli-connections", Some(&admin), None).await;
    let conns = list["connections"].as_array().unwrap().clone();
    let c1 = conns[0]["id"].as_str().unwrap().to_string();
    let c2 = conns[1]["id"].as_str().unwrap().to_string();

    // membro comum não inicia login
    let (s, _) = call(&app, "POST", &format!("/v1/cli-connections/{c1}/login"), Some(&member), None).await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    // login da conta 1: uma sessão ativa por conta
    let (s, b) = call(&app, "POST", &format!("/v1/cli-connections/{c1}/login"), Some(&admin), None).await;
    assert_eq!(s, StatusCode::ACCEPTED);
    let sid = b["session"]["id"].as_str().unwrap().to_string();
    let (s, b) = call(&app, "POST", &format!("/v1/cli-connections/{c1}/login"), Some(&admin), None).await;
    assert_eq!((s, b["error"].as_str()), (StatusCode::CONFLICT, Some("session_in_progress")));
    // a conta 2 continua livre: login independente
    let (s, _) = call(&app, "POST", &format!("/v1/cli-connections/{c2}/login"), Some(&admin), None).await;
    assert_eq!(s, StatusCode::ACCEPTED);

    // o executor recebe uma sessão por vez e só das próprias contas
    let (_, n1) = call(&app, "POST", "/executor/v1/cli/next", Some(&ex_token), None).await;
    assert_eq!(n1["session"]["id"], json!(sid));
    assert_eq!(n1["session"]["engine"], "claude");
    let (_, n2) = call(&app, "POST", "/executor/v1/cli/next", Some(&ex_token), None).await;
    assert!(n2["session"]["id"].is_string() && n2["session"]["id"] != json!(sid));
    let (_, n3) = call(&app, "POST", "/executor/v1/cli/next", Some(&ex_token), None).await;
    assert!(n3["session"].is_null());

    // link oficial aceito; link de fora da lista recusado
    let (s, b) = call(&app, "POST", &format!("/executor/v1/cli/sessions/{sid}/event"), Some(&ex_token),
        Some(json!({"kind": "url", "url": "https://evil.example/login"}))).await;
    assert_eq!((s, b["error"].as_str()), (StatusCode::BAD_REQUEST, Some("url_not_official")));
    let (s, _) = call(&app, "POST", &format!("/executor/v1/cli/sessions/{sid}/event"), Some(&ex_token),
        Some(json!({"kind": "url", "url": "https://claude.ai/oauth/authorize?x=1"}))).await;
    assert_eq!(s, StatusCode::NO_CONTENT);

    // a CLI pede o código colado: o usuário envia, o executor recebe uma única vez
    call(&app, "POST", &format!("/executor/v1/cli/sessions/{sid}/event"), Some(&ex_token), Some(json!({"kind": "awaiting_code"}))).await;
    let (_, g) = call(&app, "GET", &format!("/v1/cli-sessions/{sid}"), Some(&admin), None).await;
    assert_eq!(g["awaiting_code"], json!(true));
    let (s, _) = call(&app, "POST", &format!("/v1/cli-sessions/{sid}/code"), Some(&admin), Some(json!({"code": "ABC123-XYZ"}))).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (_, ctl) = call(&app, "POST", &format!("/executor/v1/cli/sessions/{sid}/control"), Some(&ex_token), None).await;
    assert_eq!(ctl["pasted_code"], json!("ABC123-XYZ"));
    let (_, ctl2) = call(&app, "POST", &format!("/executor/v1/cli/sessions/{sid}/control"), Some(&ex_token), None).await;
    assert!(ctl2["pasted_code"].is_null(), "o código colado é entregue uma única vez");

    // fim com verificação real: a conta fica autenticada e o estado é independente da conta 2
    let (s, _) = call(&app, "POST", &format!("/executor/v1/cli/sessions/{sid}/event"), Some(&ex_token),
        Some(json!({"kind": "finished", "state": "succeeded", "auth": "authenticated"}))).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (_, list) = call(&app, "GET", "/v1/cli-connections", Some(&admin), None).await;
    let conns = list["connections"].as_array().unwrap();
    assert_eq!(conns[0]["auth_state"], "authenticated");
    assert_eq!(conns[1]["auth_state"], "unknown");
    // nada de código, link ou segredo permanece depois do fim
    let text = list.to_string();
    assert!(!text.contains("ABC123-XYZ"));

    // conta autenticada não pode ser removida sem desconectar antes
    let (s, b) = call(&app, "DELETE", &format!("/v1/cli-connections/{c1}"), Some(&admin), None).await;
    assert_eq!((s, b["error"].as_str()), (StatusCode::CONFLICT, Some("logout_first")));

    // logout individual: só a conta 1 muda
    let (s, b) = call(&app, "POST", &format!("/v1/cli-connections/{c1}/logout"), Some(&admin), None).await;
    assert_eq!(s, StatusCode::ACCEPTED);
    let lout = b["session"]["id"].as_str().unwrap().to_string();
    let (_, n) = call(&app, "POST", "/executor/v1/cli/next", Some(&ex_token), None).await;
    assert_eq!(n["session"]["action"], "logout");
    call(&app, "POST", &format!("/executor/v1/cli/sessions/{lout}/event"), Some(&ex_token),
        Some(json!({"kind": "finished", "state": "succeeded", "auth": "unauthenticated"}))).await;
    let (s, _) = call(&app, "DELETE", &format!("/v1/cli-connections/{c1}"), Some(&admin), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);

    // a sessão de login da conta 2 é cancelada pelo administrador e encerrada pelo executor
    let n2id = n2["session"]["id"].as_str().unwrap().to_string();
    let (s, _) = call(&app, "POST", &format!("/v1/cli-sessions/{n2id}/cancel"), Some(&admin), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (_, ctl) = call(&app, "POST", &format!("/executor/v1/cli/sessions/{n2id}/control"), Some(&ex_token), None).await;
    assert_eq!(ctl["cancel"], json!(true));
    call(&app, "POST", &format!("/executor/v1/cli/sessions/{n2id}/event"), Some(&ex_token),
        Some(json!({"kind": "finished", "state": "cancelled", "auth": "unknown"}))).await;

    // cancelamento de sessão pendente encerra na hora; outro executor não enxerga a sessão
    let (_, st) = call(&app, "POST", &format!("/v1/cli-connections/{c2}/status"), Some(&admin), None).await;
    let stid = st["session"]["id"].as_str().unwrap().to_string();
    let (s, _) = call(&app, "POST", &format!("/v1/cli-sessions/{stid}/cancel"), Some(&admin), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (_, g) = call(&app, "GET", &format!("/v1/cli-sessions/{stid}"), Some(&admin), None).await;
    assert_eq!(g["state"], "cancelled");
}
