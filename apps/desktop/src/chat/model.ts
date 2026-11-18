import type { ChatContext, ChatConversation, ExecutorPreference, ProviderStatus, RunView } from "../contracts";

/** Escolha do contexto operacional mostrada na barra. Vale para os dois modos. */
export interface Selection {
  gatewayId: string;
  workspaceId: string;
  /** `auto` só existe no Orquestrador e é apenas uma preferência futura: nada é delegado. */
  engine: ExecutorPreference;
  /** Vazio = padrão da CLI. */
  executorModel: string;
  /** Modelo do ChatGPT usado pelo OneOrchestrator (separado do modelo do executor). */
  chatModel: string;
}

export const EMPTY_SELECTION: Selection = { gatewayId: "", workspaceId: "", engine: "codex", executorModel: "", chatModel: "" };

/** Contexto salvo na conversa; conversas antigas (sem contexto) herdam os valores atuais da barra. */
export function fromConversation(c: ChatConversation, base: Selection): Selection {
  const x = c.context;
  const saved = x.gatewayId !== null || x.workspaceId !== null || x.executorEngine !== null || x.executorModel !== null;
  return {
    gatewayId: saved ? (x.gatewayId ?? "") : base.gatewayId,
    workspaceId: saved ? (x.workspaceId ?? "") : base.workspaceId,
    engine: x.executorEngine ?? base.engine,
    executorModel: saved ? (x.executorModel ?? "") : base.executorModel,
    chatModel: x.orchestratorModel ?? c.model ?? base.chatModel,
  };
}

/** Uma execução direta já guarda Gateway, workspace, motor e modelo solicitado. */
export function fromRun(r: RunView, base: Selection): Selection {
  return { ...base, gatewayId: r.gateway_id, workspaceId: r.workspace_id, engine: r.engine, executorModel: r.model_requested ?? "" };
}

export function toContext(s: Selection): ChatContext {
  return {
    gatewayId: s.gatewayId || null,
    workspaceId: s.workspaceId || null,
    executorEngine: s.engine,
    executorModel: s.executorModel || null,
    orchestratorModel: s.chatModel || null,
  };
}

/** Contexto a gravar: um valor ainda desconhecido (Gateway detectando, provedor offline) nunca apaga o que a
 *  conversa já tinha salvo. O modelo do executor vazio é legítimo ("padrão da CLI") e é gravado como está. */
export function mergeContext(next: Selection, stored: ChatContext): ChatContext {
  const c = toContext(next);
  return {
    gatewayId: c.gatewayId ?? stored.gatewayId,
    workspaceId: c.workspaceId ?? stored.workspaceId,
    executorEngine: c.executorEngine,
    executorModel: c.executorModel,
    orchestratorModel: c.orchestratorModel ?? stored.orchestratorModel,
  };
}

export const NO_CONTEXT: ChatContext = { gatewayId: null, workspaceId: null, executorEngine: null, executorModel: null, orchestratorModel: null };

export interface ChatModelOption {
  label: string;
  slug: string;
}

/** O provedor devolve rótulos no formato "Nome (slug)". */
export function chatModelsOf(status: ProviderStatus | null): ChatModelOption[] {
  if (status?.state !== "connected") return [];
  return status.models.map((label) => ({ label, slug: label.match(/\(([^()]+)\)\s*$/)?.[1] ?? label }));
}

export function errText(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
}

export function formatWhen(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "";
  const now = new Date();
  return d.toDateString() === now.toDateString()
    ? d.toLocaleTimeString("pt-BR", { hour: "2-digit", minute: "2-digit" })
    : d.toLocaleDateString("pt-BR", { day: "2-digit", month: "2-digit" });
}
