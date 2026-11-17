//! Núcleo do OneAgent.app: shell Tauri + OneHub (Fase 1: leitura local, confiança de executáveis e
//! persistência).
//!
//! A UI (webview) é semiconfiável: só chama os comandos abaixo, e cada um precisa de permissão
//! explícita em `capabilities/default.json`. A decisão de aprovar um executável NÃO é da webview:
//! ela só pede, e a confirmação é um diálogo nativo aberto aqui, em Rust.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use one_adapters::guard::Throttle;
use one_adapters::{ApprovalError, SystemHost};
use one_exec::{ExecLimits, ExecService, StoreTrust, LOCAL_GATEWAY};
use one_protocol::{
    EngineKind, GatewayInfo, RunDetail, RunView, StartRunRequest, StoreStatus, Workspace,
};
use one_store::Store;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};

/// Mesma detecção repetida dentro deste prazo reaproveita o resultado (limita execução das CLIs).
const DETECT_TTL: Duration = Duration::from_secs(3);

struct Inner {
    store: Arc<Mutex<Store>>,
    exec: Arc<ExecService>,
    detect: Throttle<GatewayInfo>,
    /// Só um diálogo de aprovação por vez (evita enxurrada de janelas por uma webview abusiva).
    approval_open: AtomicBool,
}

#[derive(Clone)]
struct AppState(Arc<Inner>);

#[tauri::command]
fn store_status(state: State<'_, AppState>) -> Result<StoreStatus, String> {
    let store = state
        .0
        .store
        .lock()
        .map_err(|_| "store bloqueado".to_string())?;
    store.status().map_err(|e| e.to_string())
}

/// Detecta as CLIs deste Mac (somente leitura; só executa as aprovadas). Serializada e limitada.
#[tauri::command]
async fn detect_local_gateway(
    state: State<'_, AppState>,
    with_models: bool,
) -> Result<GatewayInfo, String> {
    let inner = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        inner.detect.get_or_compute(with_models, || {
            one_adapters::local_gateway(
                &SystemHost::new(),
                &StoreTrust(inner.store.clone()),
                with_models,
            )
        })
    })
    .await
    .map_err(|e| e.to_string())
}

/// Pede a aprovação de uma CLI: confere o que o usuário viu, abre diálogo nativo e só então grava.
#[tauri::command]
async fn approve_executable<R: tauri::Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    engine: EngineKind,
    fingerprint: String,
) -> Result<(), String> {
    let inner = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let busy = AtomicBool::new(false);
        let r = one_adapters::request_approval(
            engine,
            &SystemHost::new(),
            &StoreTrust(inner.store.clone()),
            &fingerprint,
            |msg| {
                // Só um diálogo por vez (evita enxurrada de janelas por uma webview abusiva).
                if inner
                    .approval_open
                    .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                    .is_err()
                {
                    busy.store(true, Ordering::SeqCst);
                    return false;
                }
                let ok = app
                    .dialog()
                    .message(msg)
                    .title("Aprovar executável")
                    .buttons(MessageDialogButtons::OkCancelCustom(
                        "Aprovar".into(),
                        "Cancelar".into(),
                    ))
                    .blocking_show();
                inner.approval_open.store(false, Ordering::SeqCst);
                ok
            },
        );
        inner.detect.invalidate();
        r.map_err(|e| match e {
            ApprovalError::Declined if busy.load(Ordering::SeqCst) => {
                "já existe uma aprovação em andamento".to_string()
            }
            ApprovalError::Declined => "aprovação cancelada".to_string(),
            ApprovalError::Changed => {
                "o executável mudou; atualize a tela e tente de novo".to_string()
            }
            ApprovalError::Unavailable(m) | ApprovalError::Store(m) => m,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Autorizações de workspace: lista o que o usuário já autorizou (a webview nunca informa caminhos).
#[tauri::command]
fn list_workspaces(state: State<'_, AppState>) -> Result<Vec<Workspace>, String> {
    let store = state.0.store.lock().unwrap_or_else(|e| e.into_inner());
    store
        .workspace_list(LOCAL_GATEWAY)
        .map_err(|e| e.to_string())
}

/// Abre o seletor NATIVO de pasta e autoriza o que o usuário escolher (após validar a pasta).
/// `None` = o usuário cancelou.
#[tauri::command]
async fn authorize_workspace<R: tauri::Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
) -> Result<Option<Workspace>, String> {
    let inner = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        if inner
            .approval_open
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err("já existe um diálogo aberto".to_string());
        }
        let picked = app
            .dialog()
            .file()
            .set_title("Escolha a pasta do projeto (somente leitura)")
            .blocking_pick_folder();
        inner.approval_open.store(false, Ordering::SeqCst);
        let Some(picked) = picked else {
            return Ok(None);
        };
        let path = picked.into_path().map_err(|e| e.to_string())?;
        let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
        let canonical = one_exec::workspace::validate(&path, home.as_deref())?;
        let label = canonical
            .file_name()
            .map_or_else(|| "workspace".into(), |n| n.to_string_lossy().into_owned());
        let store = inner.store.lock().unwrap_or_else(|e| e.into_inner());
        let ws = store
            .workspace_add(LOCAL_GATEWAY, &canonical.display().to_string(), &label)
            .map_err(|e| e.to_string())?;
        let _ = store.audit_append("user", "workspace_authorized", &ws.id, "authorized");
        Ok(Some(ws))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Inicia uma execução (somente leitura). Só identificadores e texto vindos da UI; tudo é revalidado.
#[tauri::command]
async fn start_run(state: State<'_, AppState>, request: StartRunRequest) -> Result<String, String> {
    let inner = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || inner.exec.start(&request).map_err(|e| e.message))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
fn cancel_run(state: State<'_, AppState>, run_id: String) -> bool {
    state.0.exec.cancel(&run_id)
}

#[tauri::command]
fn list_runs(state: State<'_, AppState>, limit: u32) -> Result<Vec<RunView>, String> {
    let store = state.0.store.lock().unwrap_or_else(|e| e.into_inner());
    store.runs_list(limit).map_err(|e| e.to_string())
}

/// Estado da execução + eventos novos desde `after_seq` (a UI consulta em intervalos curtos).
#[tauri::command]
fn get_run(
    state: State<'_, AppState>,
    run_id: String,
    after_seq: u32,
) -> Result<RunDetail, String> {
    let store = state.0.store.lock().unwrap_or_else(|e| e.into_inner());
    let run = store
        .run_get(&run_id)
        .map_err(|e| e.to_string())?
        .ok_or("execução inexistente")?;
    let events = store
        .events_after(&run_id, after_seq)
        .map_err(|e| e.to_string())?;
    Ok(RunDetail { run, events })
}

/// Contexto do app (capabilities/ACL e assets resolvidos em build). Expandido uma única vez:
/// o macro não pode aparecer duas vezes no mesmo crate. Os testes usam este mesmo contexto.
fn app_context<R: tauri::Runtime>() -> tauri::Context<R> {
    tauri::generate_context!()
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            let path = one_store::db_path(std::env::var_os("ONEAGENT_DATA_DIR").as_deref(), &dir);
            let store = Store::open(&path)
                .map_err(|e| format!("falha ao abrir {}: {e}", path.display()))?;
            store.ensure_local_gateway().map_err(|e| e.to_string())?;
            // execuções que ficaram ativas quando o app fechou (ou caiu) viram "interrompidas"
            store.reconcile_interrupted().map_err(|e| e.to_string())?;
            let store = Arc::new(Mutex::new(store));
            let exec = ExecService::new(
                store.clone(),
                Arc::new(SystemHost::new()),
                ExecLimits::default(),
            );
            app.manage(AppState(Arc::new(Inner {
                store,
                exec,
                detect: Throttle::new(DETECT_TTL),
                approval_open: AtomicBool::new(false),
            })));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            store_status,
            detect_local_gateway,
            approve_executable,
            list_workspaces,
            authorize_workspace,
            start_run,
            cancel_run,
            list_runs,
            get_run
        ])
        .run(app_context())
        .expect("erro ao iniciar o OneAgent");
}

/// Testes de integração com o contexto REAL do app (capabilities e ACL resolvidos em build) e um
/// webview simulado. Provam que a UI só alcança os comandos previstos. Não abrem diálogo nativo.
#[cfg(test)]
mod tests {
    use one_adapters::TrustStore;
    use tauri::ipc::{CallbackFn, InvokeBody};
    use tauri::test::{get_ipc_response, mock_builder, INVOKE_KEY};
    use tauri::webview::InvokeRequest;
    use tauri::{WebviewWindow, WebviewWindowBuilder};

    use super::*;

    fn app_with_state() -> (tauri::App<tauri::test::MockRuntime>, AppState) {
        let store = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
        store.lock().unwrap().ensure_local_gateway().unwrap();
        let exec = ExecService::new(
            store.clone(),
            Arc::new(SystemHost::new()),
            ExecLimits::default(),
        );
        let state = AppState(Arc::new(Inner {
            store,
            exec,
            detect: Throttle::new(DETECT_TTL),
            approval_open: AtomicBool::new(false),
        }));
        let app = mock_builder()
            .plugin(tauri_plugin_dialog::init())
            .manage(state.clone())
            .invoke_handler(tauri::generate_handler![
                store_status,
                detect_local_gateway,
                approve_executable,
                list_workspaces,
                authorize_workspace,
                start_run,
                cancel_run,
                list_runs,
                get_run
            ])
            .build(app_context())
            .expect("app de teste");
        (app, state)
    }

    fn call(
        w: &WebviewWindow<tauri::test::MockRuntime>,
        cmd: &str,
        body: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let req = InvokeRequest {
            cmd: cmd.into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url: "tauri://localhost".parse().unwrap(),
            body: InvokeBody::Json(body),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.to_string(),
        };
        get_ipc_response(w, req)
            .map(|b| b.deserialize::<serde_json::Value>().unwrap())
            .map_err(|e| e.to_string())
    }

    #[test]
    fn webview_reaches_only_the_granted_commands() {
        let (app, _state) = app_with_state();
        let w = WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .unwrap();

        // permitidos pela capability
        let st = call(&w, "store_status", serde_json::json!({}))
            .expect("store_status deve ser permitido");
        assert_eq!(st["schema_version"], one_store::LATEST_VERSION);
        assert!(
            st.get("path").is_none(),
            "o caminho do banco não vai para a webview"
        );

        let gw = call(
            &w,
            "detect_local_gateway",
            serde_json::json!({ "withModels": false }),
        )
        .expect("detect permitido");
        assert_eq!(gw["engines"].as_array().unwrap().len(), 3);
        for e in gw["engines"].as_array().unwrap() {
            if e["installed"] == true {
                // nada foi aprovado ainda: achar não executa
                assert_eq!(e["trust"], "unknown", "{e}");
                assert!(
                    e["version"].is_null(),
                    "CLI desconhecida não pode ter rodado: {e}"
                );
            }
        }

        // não concedidos: o diálogo nativo e comandos do núcleo (arquivos, janela, eventos)
        for cmd in [
            "plugin:dialog|message",
            "plugin:dialog|open",
            "plugin:window|close",
            "plugin:event|emit",
            "plugin:app|version",
        ] {
            let err = call(&w, cmd, serde_json::json!({})).expect_err(cmd);
            assert!(
                err.contains("not allowed") || err.contains("not found"),
                "{cmd}: {err}"
            );
        }
    }

    #[test]
    fn other_window_labels_get_no_access() {
        let (app, _state) = app_with_state();
        let w = WebviewWindowBuilder::new(&app, "outra", Default::default())
            .build()
            .unwrap();
        let err = call(&w, "store_status", serde_json::json!({}))
            .expect_err("janela não prevista na capability");
        assert!(err.contains("not allowed"), "{err}");
    }

    #[test]
    fn approvals_persist_in_sqlite_and_are_audited() {
        let (_app, state) = app_with_state();
        let trust = StoreTrust(state.0.store.clone());
        assert_eq!(trust.approved_fingerprint(EngineKind::Codex), None);
        trust.approve(EngineKind::Codex, "abc", "/x/codex").unwrap();
        assert_eq!(
            trust.approved_fingerprint(EngineKind::Codex).as_deref(),
            Some("abc")
        );
        let n: i64 = state
            .0
            .store
            .lock()
            .unwrap()
            .conn()
            .query_row(
                "SELECT count(*) FROM audit_log WHERE action='approve_executable'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn run_commands_are_reachable_and_enforce_policy_through_the_webview() {
        let (app, _state) = app_with_state();
        let w = WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .unwrap();
        // listas vazias, sem erro
        assert_eq!(
            call(&w, "list_workspaces", serde_json::json!({})).unwrap(),
            serde_json::json!([])
        );
        assert_eq!(
            call(&w, "list_runs", serde_json::json!({ "limit": 10 })).unwrap(),
            serde_json::json!([])
        );
        // a webview só consegue pedir por id: workspace desconhecido é recusado e nada executa
        let req = serde_json::json!({ "request": { "workspace_id": "ws_forjado", "engine": "codex", "model": null, "prompt": "oi" } });
        let e = call(&w, "start_run", req).expect_err("workspace não autorizado");
        assert!(e.contains("workspace não autorizado"), "{e}");
        // caminhos ou comandos soltos não existem no contrato: campos extras de caminho não viram execução
        let bad = serde_json::json!({ "request": { "workspace_id": "/etc", "engine": "codex", "model": null, "prompt": "oi", "path": "/etc", "command": "rm -rf /" } });
        assert!(call(&w, "start_run", bad).is_err());
        assert_eq!(
            call(&w, "cancel_run", serde_json::json!({ "runId": "rn_x" })).unwrap(),
            serde_json::json!(false)
        );
        assert!(call(
            &w,
            "get_run",
            serde_json::json!({ "runId": "rn_x", "afterSeq": 0 })
        )
        .is_err());
        // as demais ações do núcleo continuam negadas
        for cmd in [
            "plugin:dialog|open",
            "plugin:fs|read_file",
            "plugin:shell|execute",
            "plugin:event|emit",
        ] {
            assert!(call(&w, cmd, serde_json::json!({})).is_err(), "{cmd}");
        }
    }
}
