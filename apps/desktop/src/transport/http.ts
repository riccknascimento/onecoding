// Implementação HTTP do transporte (Web e, no futuro, Mac conectado ao Central).
// Sem token no navegador: a sessão é o cookie HttpOnly do OneHub, enviado pelo próprio navegador.
import { OneHubError, STREAM_KINDS } from "./types";
import type {
  CentralExecutor,
  CentralGrant,
  CentralJob,
  CliConnection,
  CliSessionView,
  ProviderConnection,
  CentralConversation,
  CentralMessage,
  CentralNode,
  CentralTaskDetail,
  CentralTaskSummary,
  CentralUser,
  ConversationSnapshot,
  OneHubTransport,
  StreamStatus,
  SubscribeOptions,
  Subscription,
  TurnInput,
  TurnResult,
} from "./types";

/** Sem sinal (evento ou ping) por este tempo, a tela mostra "reconectando". O servidor envia `ping` a cada 10 s. */
export const STALE_AFTER_MS = 12_000;
/** Sem sinal por este tempo, a conexão é refeita (um proxy pode manter uma conexão morta aberta). */
export const SILENCE_LIMIT_MS = 25_000;
const WATCH_EVERY_MS = 2_000;
/** Valor de `EventSource.CLOSED`: a resposta foi recusada e o navegador desistiu da conexão. */
const EVENT_SOURCE_CLOSED = 2;

/** Base da API: `/central-api` no desenvolvimento (proxy do Vite); em publicação, `VITE_ONEHUB_BASE` aponta para o OneHub. */
export const DEFAULT_API_BASE: string = (import.meta.env.VITE_ONEHUB_BASE as string | undefined) || "/central-api";

export function createHttpTransport(base = DEFAULT_API_BASE): OneHubTransport {
  async function request<T>(method: string, path: string, body?: unknown): Promise<T> {
    const headers: Record<string, string> = {};
    if (body !== undefined) headers["content-type"] = "application/json";
    const res = await fetch(base + path, {
      method,
      headers,
      credentials: "include",
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    if (res.status === 204) return undefined as T;
    const data = (await res.json().catch(() => ({}))) as Record<string, unknown>;
    if (!res.ok) throw new OneHubError(res.status, typeof data.error === "string" ? data.error : "erro");
    return data as T;
  }

  /** Stream da conversa. Reconecta sozinho (com `Last-Event-ID`) e para quando o OneHub encerra o acesso. */
  function subscribe(conversationId: string, options: SubscribeOptions): Subscription {
    let source: EventSource | null = null;
    let lastSeq = options.afterSeq;
    let lastSignal = Date.now();
    let stale = false;
    let closed = false;

    /** Qualquer sinal do servidor (evento ou ping) renova o prazo. */
    const signal = () => {
      lastSignal = Date.now();
      if (stale) {
        stale = false;
        options.onStatus("live");
      }
    };
    const detach = () => {
      source?.close();
      source = null;
    };
    const finish = (reason: string) => {
      closed = true;
      detach();
      clearInterval(watchdog);
      options.onStatus("closed", reason);
    };
    const connect = (status: StreamStatus) => {
      detach();
      lastSignal = Date.now();
      stale = false;
      options.onStatus(status);
      const es = new EventSource(`${base}/v1/conversations/${conversationId}/events?after=${lastSeq}`, { withCredentials: true });
      source = es;
      es.onopen = () => {
        lastSignal = Date.now();
        stale = false;
        options.onStatus("live");
      };
      for (const kind of STREAM_KINDS) {
        es.addEventListener(kind, (ev) => {
          signal();
          const msg = ev as MessageEvent<string>;
          const seq = Number(msg.lastEventId);
          // sequência já vista (reconexão repetida, por exemplo) não é entregue de novo
          if (!(seq > lastSeq)) return;
          let payload: Record<string, unknown>;
          try {
            payload = JSON.parse(msg.data) as Record<string, unknown>;
          } catch {
            return;
          }
          lastSeq = seq;
          options.onEvent({ seq, kind, payload });
        });
      }
      es.addEventListener("ping", signal);
      es.addEventListener("closed", (ev) => {
        let reason = "closed";
        try {
          reason = String((JSON.parse((ev as MessageEvent<string>).data) as { reason?: string }).reason ?? reason);
        } catch {
          /* motivo ilegível: mantém o genérico */
        }
        finish(reason);
      });
      es.onerror = () => {
        if (closed) return;
        // resposta recusada (sessão, concessão ou conversa): o navegador não tenta de novo sozinho
        if (es.readyState === EVENT_SOURCE_CLOSED) {
          finish("http");
          return;
        }
        options.onStatus("reconnecting");
      };
    };

    const watchdog = setInterval(() => {
      if (closed) return;
      const silent = Date.now() - lastSignal;
      if (silent > SILENCE_LIMIT_MS) connect("reconnecting");
      else if (silent > STALE_AFTER_MS && !stale) {
        stale = true;
        options.onStatus("reconnecting");
      }
    }, WATCH_EVERY_MS);
    connect("connecting");

    return {
      close() {
        closed = true;
        clearInterval(watchdog);
        detach();
      },
    };
  }

  return {
    async login(email, password) {
      await request("POST", "/v1/login", { email, password });
    },
    async logout() {
      await request("POST", "/v1/logout");
    },
    me: () => request<CentralUser>("GET", "/v1/me"),
    nodes: () => request<CentralNode[]>("GET", "/v1/onenodes"),
    projects: () => request<{ id: string; name: string }[]>("GET", "/v1/projects"),
    grants: () => request<CentralGrant[]>("GET", "/v1/grants"),
    conversations: () => request<CentralConversation[]>("GET", "/v1/conversations"),
    createConversation: (title = "Nova conversa") => request<{ id: string; title: string }>("POST", "/v1/conversations", { title }),
    setConversationContext: (conversationId, ctx) =>
      request<unknown>("PUT", `/v1/conversations/${conversationId}/context`, ctx).then(() => undefined),
    tasks: () => request<CentralTaskSummary[]>("GET", "/v1/tasks"),
    health: () => request<{ status: string; database: string }>("GET", "/health"),
    createNode: (name) => request("POST", "/v1/onenodes", { name }),
    nodeDetail: (id) => request("GET", `/v1/onenodes/${id}`),
    suspendNode: (id) => request<unknown>("POST", `/v1/onenodes/${id}/suspend`).then(() => undefined),
    resumeNode: (id) => request<unknown>("POST", `/v1/onenodes/${id}/resume`).then(() => undefined),
    revokeNode: (id) => request<unknown>("POST", `/v1/onenodes/${id}/revoke`).then(() => undefined),
    createExecutor: (name) => request("POST", "/v1/executors", { name }),
    revokeExecutor: (id) => request<unknown>("POST", `/v1/executors/${id}/revoke`).then(() => undefined),
    createProject: (name) => request("POST", "/v1/projects", { name }),
    grantProject: (projectId, nodeId, rootPath) =>
      request<unknown>("POST", `/v1/projects/${projectId}/grants`, { node_id: nodeId, root_path: rootPath }).then(() => undefined),
    revokeGrant: (projectId, nodeId) => request<unknown>("POST", `/v1/projects/${projectId}/grants/${nodeId}/revoke`).then(() => undefined),
    messages: (id) => request<CentralMessage[]>("GET", `/v1/conversations/${id}/messages`),
    turn: (id, input: TurnInput) => request<TurnResult>("POST", `/v1/conversations/${id}/turns`, input),
    state: (id) => request<ConversationSnapshot>("GET", `/v1/conversations/${id}/state`),
    subscribe,
    cancelJob: (id) => request<unknown>("POST", `/v1/jobs/${id}/cancel`).then(() => undefined),
    task: (id) => request<CentralTaskDetail>("GET", `/v1/tasks/${id}`),
    job: (id) => request<CentralJob>("GET", `/v1/jobs/${id}`),
    executors: () => request<CentralExecutor[]>("GET", "/v1/executors"),
    cliConnections: () => request<{ connections: CliConnection[] }>("GET", "/v1/cli-connections").then((d) => d.connections),
    createCliConnection: (input) =>
      request("POST", "/v1/cli-connections", { ...input, usage_authorized: true }),
    startCliAction: (id, action) =>
      request<{ session: CliSessionView }>("POST", `/v1/cli-connections/${id}/${action}`).then((d) => d.session),
    cancelCliSession: (sessionId) => request<unknown>("POST", `/v1/cli-sessions/${sessionId}/cancel`).then(() => undefined),
    submitCliCode: (sessionId, code) => request<unknown>("POST", `/v1/cli-sessions/${sessionId}/code`, { code }).then(() => undefined),
    setCliUsage: (id, allowed) => request<unknown>("POST", `/v1/cli-connections/${id}/usage`, { usage_authorized: allowed }).then(() => undefined),
    deleteCliConnection: (id) => request<unknown>("DELETE", `/v1/cli-connections/${id}`).then(() => undefined),
    providerConnections: () => request<{ connections: ProviderConnection[] }>("GET", "/v1/provider-connections").then((d) => d.connections),
    createProviderConnection: (input) => request("POST", "/v1/provider-connections", input),
    deleteProviderConnection: (id) => request<unknown>("DELETE", `/v1/provider-connections/${id}`).then(() => undefined),
  };
}
