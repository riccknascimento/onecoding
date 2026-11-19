//! Delegação real (ChatGPT Plus → OneHub → Gateway local → Codex). IGNORADOS por padrão: só rodam com
//! `--ignored`, sob autorização explícita do usuário. Usam uma CÓPIA do banco do OneAgent e um workspace
//! temporário; o banco real nunca é alterado.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use one_adapters::{candidate, SystemHost};
use one_chatgpt::ChatGptProvider;
use one_exec::{ExecLimits, ExecService, StoreTrust, LOCAL_GATEWAY};
use one_orchestrator::{Orchestrator, OrchestratorLimits, ToolPolicy, ToolRuntime};
use one_protocol::{EngineKind, TrustState};
use one_store::Store;

fn copy_of_real_db(tag: &str) -> (PathBuf, PathBuf) {
    let home = PathBuf::from(std::env::var_os("HOME").expect("HOME"));
    let real = home.join("Library/Application Support/br.com.apliflow.oneagent/oneagent.db");
    let dir = std::env::temp_dir().join(format!("oa-live-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let copy = dir.join("oneagent.db");
    std::fs::copy(&real, &copy).expect("copiar o banco real (somente leitura)");
    (dir, copy)
}

/// Confere, SEM executar nada, se o Codex está aprovado para a identidade atual. Não aprova nada.
#[test]
#[ignore = "verificação local: não chama o Codex nem o ChatGPT"]
fn live_precheck_codex_trust_is_current() {
    let (_dir, copy) = copy_of_real_db("precheck");
    let store = Arc::new(Mutex::new(Store::open(&copy).unwrap()));
    let trust = StoreTrust(store.clone());
    let host = SystemHost::new();
    let c = candidate(EngineKind::Codex, &host, &trust).expect("codex encontrado");
    assert_eq!(
        c.state,
        TrustState::Trusted,
        "Codex não está aprovado para esta identidade ({}). Aprove pelo app em Agent Gateways (diálogo nativo) e rode de novo.",
        c.path
    );
}

/// UMA tarefa de delegação real, somente leitura, em workspace temporário.
#[test]
#[ignore = "chamada REAL ao ChatGPT Plus e ao Codex: somente com autorização explícita"]
fn live_single_read_only_delegation() {
    let (_dir, copy) = copy_of_real_db("live");
    let store = Arc::new(Mutex::new(Store::open(&copy).unwrap()));
    // confiança: a mesma verificação do app, sobre a cópia; sem aprovação programática
    {
        let trust = StoreTrust(store.clone());
        let c = candidate(EngineKind::Codex, &SystemHost::new(), &trust).expect("codex");
        assert_eq!(
            c.state,
            TrustState::Trusted,
            "aprovação nativa do Codex necessária"
        );
    }
    let ws_dir = std::fs::canonicalize(std::env::temp_dir())
        .unwrap()
        .join(format!("oa-live-ws-{}", std::process::id()));
    std::fs::create_dir_all(&ws_dir).unwrap();
    std::fs::write(
        ws_dir.join("in.txt"),
        "Projeto de teste. Palavra-chave: ABACAXI-42.\n",
    )
    .unwrap();
    let ws = store
        .lock()
        .unwrap()
        .workspace_add(LOCAL_GATEWAY, ws_dir.to_str().unwrap(), "teste temporário")
        .unwrap();

    let host: Arc<dyn one_adapters::Host + Send> = Arc::new(SystemHost::new());
    let exec = ExecService::new(
        store.clone(),
        host.clone(),
        ExecLimits {
            timeout: std::time::Duration::from_secs(300),
            max_output: 8 * 1024 * 1024,
        },
    );
    let tools = Arc::new(ToolRuntime::new(
        exec,
        store.clone(),
        host,
        ToolPolicy::default(),
    ));
    let provider = Arc::new(ChatGptProvider::new().expect("Keychain do ChatGPT"));
    let orch = Orchestrator::new(
        provider,
        tools,
        store.clone(),
        OrchestratorLimits {
            max_steps: 5,
            max_tool_calls: 5,
        },
    );
    let prompt = format!(
        "Delegue ao motor codex, com modo somente leitura, no workspace id {} (use workspaces_list para confirmar), a tarefa: \
         'Leia o arquivo in.txt e responda somente com a palavra-chave.' Depois consulte o estado com task_status e apresente a conclusão.",
        ws.id
    );
    let reply = orch.send(None, &prompt).expect("turno do orquestrador");
    println!(
        "STEPS={} TOOL_CALLS={} STOPPED={:?}",
        reply.steps, reply.tool_calls, reply.stopped
    );
    println!("RESPOSTA={:?}", reply.text);
    for r in store.lock().unwrap().runs_list(5).unwrap() {
        println!("RUN {:?}", r);
    }
    let _ = ws_dir;
}
