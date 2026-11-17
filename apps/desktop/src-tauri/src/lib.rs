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
use one_adapters::{ApprovalError, SystemHost, TrustStore};
use one_protocol::{EngineKind, GatewayInfo, StoreStatus};
use one_store::Store;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};

/// Mesma detecção repetida dentro deste prazo reaproveita o resultado (limita execução das CLIs).
const DETECT_TTL: Duration = Duration::from_secs(3);

struct Inner {
    store: Mutex<Store>,
    detect: Throttle<GatewayInfo>,
    /// Só um diálogo de aprovação por vez (evita enxurrada de janelas por uma webview abusiva).
    approval_open: AtomicBool,
}

#[derive(Clone)]
struct AppState(Arc<Inner>);

fn engine_key(e: EngineKind) -> &'static str {
    match e {
        EngineKind::Codex => "codex",
        EngineKind::Claude => "claude",
        EngineKind::Cursor => "cursor",
    }
}

/// Aprovações persistidas no SQLite.
struct StoreTrust<'a>(&'a Mutex<Store>);

impl TrustStore for StoreTrust<'_> {
    fn approved_fingerprint(&self, engine: EngineKind) -> Option<String> {
        let s = self.0.lock().unwrap_or_else(|e| e.into_inner());
        s.trusted_get(engine_key(engine))
            .ok()
            .flatten()
            .map(|(fp, _)| fp)
    }

    fn approve(
        &self,
        engine: EngineKind,
        fingerprint: &str,
        canonical_path: &str,
    ) -> Result<(), String> {
        let s = self.0.lock().unwrap_or_else(|e| e.into_inner());
        s.trusted_put(engine_key(engine), fingerprint, canonical_path)
            .map_err(|e| e.to_string())
    }
}

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
            one_adapters::local_gateway(&SystemHost::new(), &StoreTrust(&inner.store), with_models)
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
            &StoreTrust(&inner.store),
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
            app.manage(AppState(Arc::new(Inner {
                store: Mutex::new(store),
                detect: Throttle::new(DETECT_TTL),
                approval_open: AtomicBool::new(false),
            })));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            store_status,
            detect_local_gateway,
            approve_executable
        ])
        .run(app_context())
        .expect("erro ao iniciar o OneAgent");
}

/// Testes de integração com o contexto REAL do app (capabilities e ACL resolvidos em build) e um
/// webview simulado. Provam que a UI só alcança os comandos previstos. Não abrem diálogo nativo.
#[cfg(test)]
mod tests {
    use tauri::ipc::{CallbackFn, InvokeBody};
    use tauri::test::{get_ipc_response, mock_builder, INVOKE_KEY};
    use tauri::webview::InvokeRequest;
    use tauri::{WebviewWindow, WebviewWindowBuilder};

    use super::*;

    fn app_with_state() -> (tauri::App<tauri::test::MockRuntime>, AppState) {
        let state = AppState(Arc::new(Inner {
            store: Mutex::new(Store::open_in_memory().unwrap()),
            detect: Throttle::new(DETECT_TTL),
            approval_open: AtomicBool::new(false),
        }));
        let app = mock_builder()
            .plugin(tauri_plugin_dialog::init())
            .manage(state.clone())
            .invoke_handler(tauri::generate_handler![
                store_status,
                detect_local_gateway,
                approve_executable
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
        assert_eq!(st["schema_version"], 2);
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
        let trust = StoreTrust(&state.0.store);
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
}
