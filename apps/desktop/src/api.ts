import { invoke } from "@tauri-apps/api/core";
import type { ChatContext, ChatConversation, EngineKind, GatewayInfo, PersistedChatMessage, ProviderStatus, RunDetail, RunView, StartRunRequest, StoreStatus, Workspace } from "./contracts";

/** `true` dentro do app Tauri; `false` no navegador (dev) e nos testes. */
export function inTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/** Detecção somente leitura das CLIs deste Mac. `null` fora do app. */
export async function detectLocalGateway(withModels = true): Promise<GatewayInfo | null> {
  if (!inTauri()) return null;
  return invoke<GatewayInfo>("detect_local_gateway", { withModels });
}

export async function storeStatus(): Promise<StoreStatus | null> {
  if (!inTauri()) return null;
  return invoke<StoreStatus>("store_status");
}

export async function getOrchestratorStatus(): Promise<ProviderStatus> {
  if (!inTauri()) return { state: "disconnected", reason: "Provedor disponível somente no app OneAgent." };
  return invoke<ProviderStatus>("orchestrator_provider_status");
}

export async function connectChatGptPlan(): Promise<ProviderStatus> {
  return invoke<ProviderStatus>("connect_chatgpt_plan");
}

export async function refreshChatGptModels(): Promise<ProviderStatus> {
  return invoke<ProviderStatus>("refresh_chatgpt_models");
}

export async function disconnectChatGptPlan(): Promise<void> {
  return invoke<void>("disconnect_chatgpt_plan");
}

export async function listChatConversations(): Promise<ChatConversation[]> {
  if (!inTauri()) return [];
  return invoke<ChatConversation[]>("chat_conversations_list");
}

export async function listChatMessages(conversationId: string): Promise<PersistedChatMessage[]> {
  if (!inTauri()) return [];
  return invoke<PersistedChatMessage[]>("chat_messages_list", { conversationId });
}

export async function sendChatMessage(conversationId: string | null, text: string, model: string, context: ChatContext | null = null): Promise<string> {
  return invoke<string>("chatgpt_send_message", { conversationId, text, model, context });
}

/** Guarda Gateway, projeto, executor e modelos da conversa. Só preferência: não concede execução. */
export async function setChatConversationContext(conversationId: string, context: ChatContext): Promise<void> {
  return invoke<void>("chat_conversation_set_context", { conversationId, context });
}

export async function cancelChatGeneration(conversationId: string): Promise<boolean> {
  return invoke<boolean>("chatgpt_cancel_generation", { conversationId });
}

/** Pede a aprovação de uma CLI. A confirmação real é um diálogo nativo aberto pelo app (Rust). */
export async function approveExecutable(engine: EngineKind, fingerprint: string): Promise<void> {
  if (!inTauri()) throw new Error("disponível apenas no app");
  await invoke("approve_executable", { engine, fingerprint });
}

export async function listWorkspaces(): Promise<Workspace[]> {
  if (!inTauri()) return [];
  return invoke<Workspace[]>("list_workspaces");
}

/** Abre o seletor nativo de pasta (a webview nunca informa caminhos). `null` = cancelado. */
export async function authorizeWorkspace(): Promise<Workspace | null> {
  if (!inTauri()) throw new Error("disponível apenas no app");
  return invoke<Workspace | null>("authorize_workspace");
}

export async function startRun(request: StartRunRequest): Promise<string> {
  if (!inTauri()) throw new Error("disponível apenas no app");
  return invoke<string>("start_run", { request });
}

export async function cancelRun(runId: string): Promise<boolean> {
  if (!inTauri()) return false;
  return invoke<boolean>("cancel_run", { runId });
}

export async function listRuns(limit = 50): Promise<RunView[]> {
  if (!inTauri()) return [];
  return invoke<RunView[]>("list_runs", { limit });
}

export async function getRun(runId: string, afterSeq: number): Promise<RunDetail> {
  if (!inTauri()) throw new Error("disponível apenas no app");
  return invoke<RunDetail>("get_run", { runId, afterSeq });
}
