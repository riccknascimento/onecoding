//! Núcleo do OneAgent.app: shell Tauri + OneHub (Fase 1: leitura local, confiança de executáveis e
//! persistência).
//!
//! A UI (webview) é semiconfiável: só chama os comandos abaixo, e cada um precisa de permissão
//! explícita em `capabilities/default.json`. A decisão de aprovar um executável NÃO é da webview:
//! ela só pede, e a confirmação é um diálogo nativo aberto aqui, em Rust.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use one_adapters::guard::Throttle;
use one_adapters::{ApprovalError, SystemHost};
use one_chatgpt::ChatGptProvider;
use one_exec::{ExecLimits, ExecService, StoreTrust, LOCAL_GATEWAY};
use one_orchestrator::{
    ChatItem, DisconnectedProvider, OrchestratorProvider, TurnEvent, TurnRequest,
};
use one_protocol::{
    ChatContext, ChatConversation, ChatMessageState, ChatRole, EngineKind, GatewayInfo,
    PersistedChatMessage, ProviderStatus, RunDetail, RunView, StartRunRequest, StoreStatus,
    UsageInfo, Workspace,
};
use one_store::Store;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};

/// Mesma detecção repetida dentro deste prazo reaproveita o resultado (limita execução das CLIs).
const DETECT_TTL: Duration = Duration::from_secs(3);

struct Inner {
    store: Arc<Mutex<Store>>,
    exec: Arc<ExecService>,
    orchestrator_provider: Arc<dyn OrchestratorProvider>,
    chatgpt_provider: Option<Arc<ChatGptProvider>>,
    detect: Throttle<GatewayInfo>,
    /// Só um diálogo de aprovação por vez (evita enxurrada de janelas por uma webview abusiva).
    approval_open: AtomicBool,
    active_chats: Mutex<HashMap<String, Arc<AtomicBool>>>,
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

#[tauri::command]
fn orchestrator_provider_status(state: State<'_, AppState>) -> ProviderStatus {
    state.0.orchestrator_provider.status()
}

#[tauri::command]
async fn connect_chatgpt_plan(state: State<'_, AppState>) -> Result<ProviderStatus, String> {
    let provider = state
        .0
        .chatgpt_provider
        .clone()
        .ok_or("Integração ChatGPT indisponível neste host.")?;
    tauri::async_runtime::spawn_blocking(move || provider.connect().map_err(|e| e.to_string()))
        .await
        .map_err(|e| e.to_string())??;
    Ok(state.0.orchestrator_provider.status())
}

#[tauri::command]
async fn refresh_chatgpt_models(state: State<'_, AppState>) -> Result<ProviderStatus, String> {
    let provider = state
        .0
        .chatgpt_provider
        .clone()
        .ok_or("Integração ChatGPT indisponível neste host.")?;
    tauri::async_runtime::spawn_blocking(move || {
        provider.refresh_models().map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())??;
    Ok(state.0.orchestrator_provider.status())
}

#[tauri::command]
async fn disconnect_chatgpt_plan(state: State<'_, AppState>) -> Result<(), String> {
    let provider = state
        .0
        .chatgpt_provider
        .clone()
        .ok_or("Integração ChatGPT indisponível neste host.")?;
    tauri::async_runtime::spawn_blocking(move || provider.disconnect().map_err(|e| e.to_string()))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
fn chat_conversations_list(state: State<'_, AppState>) -> Result<Vec<ChatConversation>, String> {
    state
        .0
        .store
        .lock()
        .map_err(|_| "store bloqueado".to_string())?
        .chat_conversations_list()
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn chat_messages_list(
    state: State<'_, AppState>,
    conversation_id: String,
) -> Result<Vec<PersistedChatMessage>, String> {
    let store = state
        .0
        .store
        .lock()
        .map_err(|_| "store bloqueado".to_string())?;
    if store
        .conversation_mode(&conversation_id)
        .map_err(|e| e.to_string())?
        != Some(one_protocol::ChatMode::Orchestrator)
    {
        return Err("conversa do orquestrador inexistente".into());
    }
    store
        .chat_messages_list(&conversation_id)
        .map_err(|e| e.to_string())
}

/// Guarda o contexto (Gateway, projeto, executor, modelos) de uma conversa. Só preferência: não
/// concede permissão de execução e não entra na requisição ao provedor.
#[tauri::command]
fn chat_conversation_set_context(
    state: State<'_, AppState>,
    conversation_id: String,
    context: ChatContext,
) -> Result<(), String> {
    context.validate()?;
    let store = state
        .0
        .store
        .lock()
        .map_err(|_| "store bloqueado".to_string())?;
    if store
        .chat_conversation_context_set(&conversation_id, &context)
        .map_err(|e| e.to_string())?
    {
        Ok(())
    } else {
        Err("conversa do orquestrador inexistente".into())
    }
}

#[tauri::command]
async fn chatgpt_send_message(
    state: State<'_, AppState>,
    conversation_id: Option<String>,
    text: String,
    model: String,
    context: Option<ChatContext>,
) -> Result<String, String> {
    if text.trim().is_empty() || text.len() > 16 * 1024 {
        return Err("A mensagem deve conter até 16 KB.".into());
    }
    if model.is_empty() || model.len() > 128 {
        return Err("Modelo inválido.".into());
    }
    if let Some(c) = &context {
        c.validate()?;
    }
    let inner = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let provider = inner.chatgpt_provider.clone().ok_or("Integração ChatGPT indisponível neste host.")?;
        if !matches!(provider.status(), ProviderStatus::Connected { plan_usage_authorized: true, .. }) { return Err("ChatGPT Plan Usage não está autorizado. Reconecte a conta.".to_string()); }
        let conv = match conversation_id {
            Some(id) => {
                let store=inner.store.lock().map_err(|_| "store bloqueado".to_string())?;
                if store.conversation_mode(&id).map_err(|e| e.to_string())? != Some(one_protocol::ChatMode::Orchestrator) { return Err("conversa do orquestrador inexistente".into()); }
                id
            }
            None => inner.store.lock().map_err(|_| "store bloqueado".to_string())?.chat_conversation_create().map_err(|e| e.to_string())?,
        };
        if let Some(c) = &context {
            inner.store.lock().map_err(|_| "store bloqueado".to_string())?.chat_conversation_context_set(&conv, c).map_err(|e| e.to_string())?;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut active=inner.active_chats.lock().map_err(|_| "gerações bloqueadas".to_string())?;
            if active.contains_key(&conv) { return Err("Já existe uma geração nesta conversa.".into()); }
            active.insert(conv.clone(),cancel.clone());
        }
        let result = (|| {
            let (user_id, assistant_id, items) = {
                let store=inner.store.lock().map_err(|_| "store bloqueado".to_string())?;
                let existing=store.chat_messages_list(&conv).map_err(|e| e.to_string())?;
                let recent: Vec<_>=existing.iter().filter(|m| matches!(m.state,ChatMessageState::Completed)).rev().take(20).collect::<Vec<_>>().into_iter().rev().collect();
                let mut items: Vec<ChatItem>=recent.iter().map(|m| match m.role { ChatRole::User=>ChatItem::User(m.text.clone()), ChatRole::Orchestrator=>ChatItem::Assistant(m.text.clone()) }).collect();
                items.push(ChatItem::User(text.clone()));
                let bytes: usize=items.iter().map(|x| match x { ChatItem::User(s)|ChatItem::Assistant(s)=>s.len(), _=>0 }).sum();
                if bytes > 32 * 1024 { return Err("O histórico desta conversa excede o limite de 32 KB.".into()); }
                let user=store.chat_message_insert(&conv,ChatRole::User,&text,Some(&model),ChatMessageState::Completed).map_err(|e|e.to_string())?;
                let assistant=store.chat_message_insert(&conv,ChatRole::Orchestrator,"",Some(&model),ChatMessageState::Streaming).map_err(|e|e.to_string())?;
                (user,assistant,items)
            };
            let _ = user_id;
            let instructions="Você é o consultor técnico do OneAgent. Ajude o usuário a entender e planejar demandas de software. Nesta etapa, não afirme que executou alterações e não delegue tarefas: a execução requer uma etapa futura com autorização explícita.";
            let tools=[];
            let req=TurnRequest { instructions, items:&items, tools:&tools };
            let output=provider.complete_chat_cancellable(&model,&req,Some(&cancel),&mut |event| {
                if let TurnEvent::TextDelta(delta)=event {
                    if cancel.load(Ordering::Acquire) { return; }
                    if let Ok(store)=inner.store.lock() { let _=store.chat_message_delta(&assistant_id,&delta); }
                }
            });
            let store=inner.store.lock().map_err(|_| "store bloqueado".to_string())?;
            match output {
                Ok(out) if out.tool_calls.is_empty() && !cancel.load(Ordering::Acquire) => {
                    store.chat_message_finish(&conv,&assistant_id,ChatMessageState::Completed,out.model.as_deref(),&out.usage,None).map_err(|e|e.to_string())?;
                    Ok(conv.clone())
                }
                Ok(out) if !out.tool_calls.is_empty() => {
                    let err="O provedor retornou uma chamada de ferramenta; ferramentas estão desativadas nesta fase.";
                    store.chat_message_finish(&conv,&assistant_id,ChatMessageState::Error,out.model.as_deref(),&out.usage,Some(err)).map_err(|e|e.to_string())?;
                    Err(err.into())
                }
                Ok(out) => {
                    store.chat_message_finish(&conv,&assistant_id,ChatMessageState::Interrupted,out.model.as_deref(),&out.usage,None).map_err(|e|e.to_string())?;
                    Err("Geração interrompida.".into())
                }
                Err(error) => {
                    let message=error.message().to_owned();
                    let was_cancelled=cancel.load(Ordering::Acquire);
                    store.chat_message_finish(&conv,&assistant_id,if was_cancelled { ChatMessageState::Interrupted } else { ChatMessageState::Error },None,&UsageInfo::default(),if was_cancelled { None } else { Some(&message) }).map_err(|e|e.to_string())?;
                    if was_cancelled { Err("Geração interrompida.".into()) } else { Err(message) }
                }
            }
        })();
        if let Ok(mut active)=inner.active_chats.lock() { active.remove(&conv); }
        result
    }).await.map_err(|e|e.to_string())?
}

#[tauri::command]
fn chatgpt_cancel_generation(state: State<'_, AppState>, conversation_id: String) -> bool {
    let active = state
        .0
        .active_chats
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if let Some(flag) = active.get(&conversation_id) {
        flag.store(true, Ordering::Release);
        true
    } else {
        false
    }
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
            store
                .chat_generation_reconcile()
                .map_err(|e| e.to_string())?;
            let store = Arc::new(Mutex::new(store));
            let exec = ExecService::new(
                store.clone(),
                Arc::new(SystemHost::new()),
                ExecLimits::default(),
            );
            let (orchestrator_provider, chatgpt_provider): (
                Arc<dyn OrchestratorProvider>,
                Option<Arc<ChatGptProvider>>,
            ) = match ChatGptProvider::new() {
                Ok(provider) => {
                    let provider = Arc::new(provider);
                    (provider.clone(), Some(provider))
                }
                Err(error) => (
                    Arc::new(DisconnectedProvider::new(format!(
                        "Integração ChatGPT indisponível: {error}"
                    ))),
                    None,
                ),
            };
            app.manage(AppState(Arc::new(Inner {
                store,
                exec,
                orchestrator_provider,
                chatgpt_provider,
                detect: Throttle::new(DETECT_TTL),
                approval_open: AtomicBool::new(false),
                active_chats: Mutex::new(HashMap::new()),
            })));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            store_status,
            orchestrator_provider_status,
            connect_chatgpt_plan,
            refresh_chatgpt_models,
            disconnect_chatgpt_plan,
            chat_conversations_list,
            chat_messages_list,
            chat_conversation_set_context,
            chatgpt_send_message,
            chatgpt_cancel_generation,
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
            orchestrator_provider: Arc::new(DisconnectedProvider::new(
                "ChatGPT Plus não conectado nesta fase.",
            )),
            chatgpt_provider: None,
            detect: Throttle::new(DETECT_TTL),
            approval_open: AtomicBool::new(false),
            active_chats: Mutex::new(HashMap::new()),
        }));
        let app = mock_builder()
            .plugin(tauri_plugin_dialog::init())
            .manage(state.clone())
            .invoke_handler(tauri::generate_handler![
                store_status,
                orchestrator_provider_status,
                connect_chatgpt_plan,
                refresh_chatgpt_models,
                disconnect_chatgpt_plan,
                chat_conversations_list,
                chat_messages_list,
                chat_conversation_set_context,
                chatgpt_send_message,
                chatgpt_cancel_generation,
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
        let (app, state) = app_with_state();
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

        let provider = call(&w, "orchestrator_provider_status", serde_json::json!({}))
            .expect("estado do provedor é permitido");
        assert_eq!(provider["state"], "disconnected");
        assert!(provider["reason"]
            .as_str()
            .unwrap()
            .contains("não conectado"));

        let conversations = call(&w, "chat_conversations_list", serde_json::json!({}))
            .expect("histórico é permitido");
        assert_eq!(conversations.as_array().unwrap().len(), 0);

        // contexto por conversa: comando alcançável, restrito ao Modo Orquestrador e validado
        let ctx = serde_json::json!({
            "gatewayId":"local","workspaceId":"ws1","executorEngine":"auto",
            "executorModel":null,"orchestratorModel":"gpt-test"
        });
        let missing = call(
            &w,
            "chat_conversation_set_context",
            serde_json::json!({ "conversationId":"cv_inexistente", "context":ctx }),
        )
        .expect_err("conversa inexistente");
        assert!(missing.contains("inexistente"), "{missing}");
        let conv = state
            .0
            .store
            .lock()
            .unwrap()
            .chat_conversation_create()
            .unwrap();
        let direct = state
            .0
            .store
            .lock()
            .unwrap()
            .conversation_create(one_protocol::ChatMode::Direct, "d")
            .unwrap();
        let wrong_mode = call(
            &w,
            "chat_conversation_set_context",
            serde_json::json!({ "conversationId":direct, "context":ctx }),
        )
        .expect_err("conversa do Modo direto não aceita contexto");
        assert!(wrong_mode.contains("inexistente"), "{wrong_mode}");
        let too_long = serde_json::json!({ "gatewayId":"x".repeat(200) });
        assert!(call(
            &w,
            "chat_conversation_set_context",
            serde_json::json!({ "conversationId":conv, "context":too_long }),
        )
        .is_err());
        call(
            &w,
            "chat_conversation_set_context",
            serde_json::json!({ "conversationId":conv, "context":ctx }),
        )
        .expect("contexto é permitido");
        let listed = call(&w, "chat_conversations_list", serde_json::json!({})).unwrap();
        assert_eq!(listed[0]["context"]["executorEngine"], "auto");
        assert_eq!(listed[0]["context"]["orchestratorModel"], "gpt-test");

        let send = call(
            &w,
            "chatgpt_send_message",
            serde_json::json!({ "text":"oi", "model":"gpt-test", "conversationId":null }),
        )
        .expect_err("sem provedor não envia mensagens");
        assert!(send.contains("Integração ChatGPT indisponível"));

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
        let err = call(
            &w,
            "chatgpt_send_message",
            serde_json::json!({ "text":"oi", "model":"gpt-test", "conversationId":null }),
        )
        .expect_err("o comando de conversa não deve ser concedido a outra janela");
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
