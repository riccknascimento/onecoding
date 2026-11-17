import { invoke } from "@tauri-apps/api/core";
import type { EngineKind, GatewayInfo, StoreStatus } from "./contracts";

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
