fn main() {
    // Menor privilégio: cada comando exposto precisa de permissão explícita na capability.
    let app = tauri_build::AppManifest::new().commands(&[
        "store_status",
        "orchestrator_provider_status",
        "connect_chatgpt_plan",
        "refresh_chatgpt_models",
        "disconnect_chatgpt_plan",
        "chat_conversations_list",
        "chat_messages_list",
        "chat_conversation_set_context",
        "chatgpt_send_message",
        "chatgpt_cancel_generation",
        "detect_local_gateway",
        "approve_executable",
        "list_workspaces",
        "authorize_workspace",
        "start_run",
        "cancel_run",
        "list_runs",
        "get_run",
    ]);
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(app))
        .expect("falha no tauri-build");
}
