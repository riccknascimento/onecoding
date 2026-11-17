import { invoke } from "@tauri-apps/api/core";
import type { EngineKind, GatewayInfo, RunDetail, RunView, StartRunRequest, StoreStatus, Workspace } from "./contracts";

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
