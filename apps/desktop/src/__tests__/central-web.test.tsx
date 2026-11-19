import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { CentralApp } from "../central/CentralApp";
import { WorkspaceTabs } from "../central/WorkspaceTabs";
import { createHttpTransport, OneHubError } from "../transport";
import type { CentralMessage, ConversationSnapshot, OneHubTransport, StreamEvent, SubscribeOptions } from "../transport";

const USER = { id: "u1", email: "admin@teste.local", role: "admin" as const };
const msg = (over: Partial<CentralMessage>): CentralMessage => ({
  id: "m", role: "assistant", body: "", created_at: "2026-11-18T00:00:00Z", state: "completed", task_id: null, job_id: null, ...over,
});
const NODE_DETAIL = {
  id: "n1", name: "Servidor A", status: "online" as const, suspended: false, paired_at: null, last_seen_at: null,
  access_type: "direct" as const, catalog_version: 1, applied_version: 1, applied_at: null, catalog_in_sync: true,
  applied_report: null, grants: [],
};
const EMPTY: ConversationSnapshot = { last_seq: 0, messages: [], events: [], jobs: [], tasks: [] };

describe("abas de Workspace", () => {
  const workspaces = [
    { id: "w1", name: "SystemFlow", node_id: "n1", project_id: "p1", updated_at: "x" },
    { id: "w2", name: "Atendimento", node_id: null, project_id: null, updated_at: "x" },
  ];

  it("mantém Settings fixa, não duplica Workspace aberto e permite fechar só a aba", async () => {
    const user = userEvent.setup();
    const onOpen = vi.fn();
    const onClose = vi.fn();
    const onCreate = vi.fn(async () => undefined);
    render(<WorkspaceTabs workspaces={workspaces} openTabs={["w1"]} activeTab="w1" onActivate={vi.fn()} onOpen={onOpen} onClose={onClose} onCreate={onCreate} onRename={vi.fn(async () => undefined)} />);
    expect(screen.getByRole("button", { name: /Settings/ })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Fechar SystemFlow" }));
    expect(onClose).toHaveBeenCalledWith("w1");
    await user.click(screen.getByRole("button", { name: "Abrir ou criar Workspace" }));
    expect(screen.getByRole("button", { name: "SystemFlow · aberto" })).toBeDisabled();
    await user.click(screen.getByRole("button", { name: "Atendimento" }));
    expect(onOpen).toHaveBeenCalledWith("w2");
    await user.click(screen.getByRole("button", { name: "Abrir ou criar Workspace" }));
    await user.type(screen.getByRole("textbox", { name: "Nome do novo Workspace" }), "Novo Projeto");
    await user.click(screen.getByRole("button", { name: "Criar" }));
    expect(onCreate).toHaveBeenCalledWith("Novo Projeto");
  });
});

/** Transporte de teste. `me` falha até o login, como o OneHub real (sem cookie = 401). */
function fakeTransport(over: Partial<OneHubTransport> = {}): OneHubTransport {
  let session = false;
  const base: OneHubTransport = {
    login: vi.fn(async () => {
      session = true;
    }),
    logout: vi.fn(async () => {
      session = false;
    }),
    me: vi.fn(async () => {
      if (!session) throw new OneHubError(401, "unauthorized");
      return USER;
    }),
    nodes: vi.fn(async () => [{ id: "n1", name: "Servidor A", status: "online" as const, last_seen_at: null }]),
    projects: vi.fn(async () => [{ id: "p1", name: "SystemFlow" }]),
    cliConnections: vi.fn(async () => []),
    createCliConnection: vi.fn(async () => ({})),
    startCliAction: vi.fn(async () => ({ id: "s0", action: "login" as const, state: "pending", url: null, device_code: null, awaiting_code: false, result: null })),
    cancelCliSession: vi.fn(async () => undefined),
    submitCliCode: vi.fn(async () => undefined),
    setCliUsage: vi.fn(async () => undefined),
    deleteCliConnection: vi.fn(async () => undefined),
    providerConnections: vi.fn(async () => []),
    createProviderConnection: vi.fn(async () => ({})),
    deleteProviderConnection: vi.fn(async () => undefined),
    grants: vi.fn(async () => [{ node_id: "n1", node_name: "Servidor A", project_id: "p1", project_name: "SystemFlow", root_path: "/opt/onenode/workspace/systemflow" }]),
    workspaces: vi.fn(async () => [{ id: "w1", name: "Meu Workspace", node_id: null, project_id: null, updated_at: "x" }]),
    createWorkspace: vi.fn(async (name: string) => ({ id: "w2", name, node_id: null, project_id: null, updated_at: "x" })),
    renameWorkspace: vi.fn(async (id: string, name: string) => ({ id, name, node_id: null, project_id: null, updated_at: "x" })),
    setWorkspaceContext: vi.fn(async () => undefined),
    conversations: vi.fn(async () => [{ id: "c1", title: "Plano", updated_at: "x", node_id: null, project_id: null, workspace_id: "w1" }]),
    createConversation: vi.fn(async () => ({ id: "c2", title: "Nova conversa" })),
    messages: vi.fn(async () => []),
    turn: vi.fn(async () => ({ user_message: msg({ role: "user", body: "oi" }), assistant_message: msg({ body: "" }) })),
    state: vi.fn(async () => EMPTY),
    subscribe: vi.fn(() => ({ close: vi.fn() })),
    cancelJob: vi.fn(async () => undefined),
    task: vi.fn(async () => ({ id: "t", status: "succeeded" as const, tool: "files_read", args: {}, result: {}, events: [] })),
    job: vi.fn(async () => ({ id: "j", status: "succeeded" as const, question: "q", answer: "r", evidence: [], error: null, cancel_requested: false })),
    executors: vi.fn(async () => [{ id: "e1", name: "Executor Mac", engine: "codex", status: "online" as const }]),
    health: vi.fn(async () => ({ status: "ok", database: "ok" })),
    createNode: vi.fn(async () => ({ id: "n9", pairing_code: "CODIGO-TESTE-1", pairing_expires_at: new Date(Date.now() + 900000).toISOString() })),
    revokeNode: vi.fn(async () => undefined),
    createExecutor: vi.fn(async () => ({ id: "e9", pairing_code: "CODIGO-EXEC-1", pairing_expires_at: new Date(Date.now() + 900000).toISOString() })),
    revokeExecutor: vi.fn(async () => undefined),
    createProject: vi.fn(async () => ({ id: "p9", name: "Novo" })),
    grantProject: vi.fn(async () => undefined),
    revokeGrant: vi.fn(async () => undefined),
    nodeDetail: vi.fn(async () => NODE_DETAIL),
    setConversationContext: vi.fn(async () => undefined),
    tasks: vi.fn(async () => []),
    suspendNode: vi.fn(async () => undefined),
    resumeNode: vi.fn(async () => undefined),
  };
  return { ...base, ...over };
}

/** Transporte que guarda o canal de stream aberto, para o teste empurrar eventos reais. */
function streaming(snapshot: Partial<ConversationSnapshot> = {}) {
  const channel: { current?: SubscribeOptions } = {};
  const t = fakeTransport({
    me: vi.fn(async () => USER),
    state: vi.fn(async () => ({ ...EMPTY, ...snapshot })),
    subscribe: vi.fn((_id: string, opts: SubscribeOptions) => {
      channel.current = opts;
      return { close: vi.fn() };
    }),
  });
  const emit = (ev: StreamEvent) => act(() => channel.current!.onEvent(ev));
  const status = (s: "live" | "reconnecting" | "closed", reason?: string) => act(() => channel.current!.onStatus(s, reason));
  return { t, emit, status };
}

/** Sessão já ativa: abre a conversa e espera o canal de stream estar assinado. */
async function open(t: OneHubTransport) {
  render(<CentralApp transport={t} />);
  await waitFor(() => expect(t.subscribe).toHaveBeenCalled());
  return userEvent.setup();
}

/** Conversa que se comporta como o OneHub: o contexto salvo volta na listagem de conversas. */
function contextConversation() {
  const conv = { id: "c1", title: "Plano", updated_at: "x", node_id: null as string | null, project_id: null as string | null, workspace_id: "w1" };
  return {
    conv,
    conversations: vi.fn(async () => [{ ...conv }]),
    setConversationContext: vi.fn(async (_id: string, ctx: { node_id: string | null; project_id: string | null }) => {
      conv.node_id = ctx.node_id;
      conv.project_id = ctx.project_id;
    }),
  };
}

async function login(t: OneHubTransport) {
  render(<CentralApp transport={t} />);
  const user = userEvent.setup();
  await user.type(await screen.findByLabelText("E-mail"), "admin@teste.local");
  await user.type(screen.getByLabelText("Senha"), "senha-admin-teste-123");
  await user.click(screen.getByRole("button", { name: "Entrar" }));
  return user;
}

afterEach(() => {
  vi.restoreAllMocks();
  window.history.replaceState({}, "", "/");
  window.localStorage.clear();
  window.sessionStorage.clear();
});

describe("OneAgent Web (OneHub Central): tela", () => {
  it("login recusado mostra mensagem genérica", async () => {
    const t = fakeTransport({ login: vi.fn(async () => { throw new OneHubError(401, "unauthorized"); }) });
    await login(t);
    expect(await screen.findByRole("alert")).toHaveTextContent("E-mail ou senha inválidos.");
  });

  it("após login mostra conversas, contexto OneHub Central por padrão, motor e Orquestrador indisponível; sem dados DEMO", async () => {
    await login(fakeTransport());
    expect(await screen.findByRole("button", { name: /Plano/ })).toBeInTheDocument();
    expect(screen.getByRole("combobox", { name: "OneNode" })).toHaveDisplayValue("OneHub Central (administração)");
    expect(screen.getByText(/Codex CLI \(OneExecutor\): online/)).toBeInTheDocument();
    expect(screen.getByText(/Orquestrador · indisponível/)).toBeInTheDocument();
    expect(document.body.textContent?.toLowerCase()).not.toContain("demo");
  });

  it("a sessão é restaurada pelo cookie ao recarregar: sem pedir login de novo", async () => {
    render(<CentralApp transport={fakeTransport({ me: vi.fn(async () => USER) })} />);
    expect(await screen.findByRole("button", { name: /Plano/ })).toBeInTheDocument();
    expect(screen.queryByLabelText("Senha")).not.toBeInTheDocument();
  });

  it("abre no Chat por padrão e alterna grupos da sidebar", async () => {
    window.history.replaceState({}, "", "/");
    render(<CentralApp transport={fakeTransport({ me: vi.fn(async () => USER) })} />);
    expect(await screen.findByRole("button", { name: /Plano/ })).toBeInTheDocument();
    expect(screen.getByRole("combobox", { name: "OneNode" })).toBeInTheDocument();
    const resources = screen.getByRole("button", { name: "Recursos" });
    expect(resources).toHaveAttribute("aria-expanded", "true");
    await userEvent.click(resources);
    expect(resources).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByRole("button", { name: "Projetos" })).not.toBeInTheDocument();
    await userEvent.click(resources);
    expect(screen.getByRole("button", { name: "Projetos" })).toBeInTheDocument();
  });

  it("abre uma nova conversa mesmo quando está em uma configuração", async () => {
    window.history.replaceState({}, "", "/");
    render(<CentralApp transport={fakeTransport({ me: vi.fn(async () => USER) })} />);
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: "OneNode" }));
    expect(window.location.pathname).toBe("/configuracoes/OneNodes");
    await user.click(screen.getAllByRole("button", { name: "Meu Workspace" })[0]!);
    await user.click(screen.getByRole("button", { name: "Nova conversa" }));
    expect(screen.getByText("Nova conversa", { selector: ".chat-title" })).toBeInTheDocument();
    expect(window.location.search).toBe("?workspace=w1");
  });

  it("voltar e avançar do navegador acompanham a seção de configuração", async () => {
    window.history.replaceState({}, "", "/");
    render(<CentralApp transport={fakeTransport({ me: vi.fn(async () => USER) })} />);
    await userEvent.click(await screen.findByRole("button", { name: "Executors" }));
    expect(await screen.findByRole("heading", { name: "OneExecutors" })).toBeInTheDocument();
    expect(window.location.pathname).toBe("/configuracoes/OneExecutors");
    act(() => {
      window.history.pushState({}, "", "/");
      window.dispatchEvent(new PopStateEvent("popstate"));
    });
    expect(await screen.findByRole("combobox", { name: "OneNode" })).toBeInTheDocument();
    act(() => {
      window.history.pushState({}, "", "/configuracoes/OneExecutors");
      window.dispatchEvent(new PopStateEvent("popstate"));
    });
    expect(await screen.findByRole("heading", { name: "OneExecutors" })).toBeInTheDocument();
  });

  it("sem concessão, o OneNode continua selecionável e a tela mostra o caminho para conceder acesso", async () => {
    const ctx = contextConversation();
    const t = fakeTransport({ grants: vi.fn(async () => []), ...ctx });
    await login(t);
    const combo = await screen.findByRole("combobox", { name: "OneNode" });
    await userEvent.selectOptions(combo, "n1");
    expect(await screen.findByText("Nenhum projeto autorizado.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Conceder acesso em OneNodes" })).toBeInTheDocument();
    expect(screen.queryByRole("combobox", { name: "Projeto" })).not.toBeInTheDocument();
    await waitFor(() => expect(t.setConversationContext).toHaveBeenCalledWith("c1", { node_id: "n1", project_id: null }));
  });

  it("escolher OneNode e projeto persiste o contexto na conversa pelo OneHub", async () => {
    const t = fakeTransport(contextConversation());
    await login(t);
    await userEvent.selectOptions(await screen.findByRole("combobox", { name: "OneNode" }), "n1");
    await waitFor(() => expect(t.setConversationContext).toHaveBeenCalledWith("c1", { node_id: "n1", project_id: null }));
    await userEvent.selectOptions(await screen.findByRole("combobox", { name: "Projeto" }), "p1");
    await waitFor(() => expect(t.setConversationContext).toHaveBeenLastCalledWith("c1", { node_id: "n1", project_id: "p1" }));
  });

  it("voltar ao OneHub Central persiste contexto sem OneNode, e a análise não envia nó nem projeto", async () => {
    const ctx = contextConversation();
    ctx.conv.node_id = "n1";
    ctx.conv.project_id = "p1";
    const t = fakeTransport(ctx);
    await login(t);
    await userEvent.selectOptions(await screen.findByRole("combobox", { name: "OneNode" }), "");
    await waitFor(() => expect(t.setConversationContext).toHaveBeenCalledWith("c1", { node_id: null, project_id: null }));
  });

  it("nó suspenso aparece como suspenso e o cabeçalho diz por que a análise está indisponível", async () => {
    const t = fakeTransport({
      conversations: vi.fn(async () => [{ id: "c1", title: "Plano", updated_at: "x", node_id: "n1", project_id: "p1", workspace_id: "w1" }]),
      nodes: vi.fn(async () => [{ id: "n1", name: "Servidor A", status: "online" as const, last_seen_at: null, suspended: true }]),
    });
    await login(t);
    expect(await screen.findByRole("option", { name: /Servidor A · suspenso/ })).toBeInTheDocument();
    expect(screen.getByText("Este OneNode está suspenso até ser retomado.")).toBeInTheDocument();
  });

  it("comandos de escrita aparecem como 'em breve' e não podem ser enviados", async () => {
    await login(fakeTransport());
    expect(await screen.findByRole("button", { name: "/corrigir · em breve" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "/investigar" })).toBeEnabled();
  });
});

describe("OneAgent Web: eventos em tempo real", () => {
  const withJob = (events: ConversationSnapshot["events"] = []) => ({
    messages: [
      msg({ id: "u1", role: "user", body: "o que faz o projeto?", created_at: "2026-11-18T00:00:01Z" }),
      msg({ id: "a1", body: "Pedido enviado ao OneExecutor (Codex) para análise do projeto.", state: "task", job_id: "j9", created_at: "2026-11-18T00:00:02Z" }),
    ],
    jobs: [{ id: "j9", status: "queued" as const, question: "o que faz o projeto?", error: null, cancel_requested: false }],
    events,
    last_seq: 1,
  });

  it("progresso chega sem recarregar a página, e a resposta final aparece com o histórico", async () => {
    const { t, emit } = streaming(withJob());
    await open(t);
    emit({ seq: 2, kind: "job.status", payload: { job_id: "j9", status: "running", cancel_requested: false, error: null } });
    expect(await screen.findByText("OneExecutor pegou o pedido")).toBeInTheDocument();
    emit({ seq: 3, kind: "job.progress", payload: { job_id: "j9", seq: 0, kind: "tool", payload: { summary: "cat README.md" } } });
    expect(await screen.findByText("Codex executou: cat README.md")).toBeInTheDocument();
    emit({ seq: 4, kind: "message.updated", payload: { id: "a1", body: "O projeto mede desempenho.", state: "completed" } });
    emit({ seq: 5, kind: "job.status", payload: { job_id: "j9", status: "succeeded", cancel_requested: false, error: null } });
    expect(await screen.findByText("O projeto mede desempenho.")).toBeInTheDocument();
    expect(screen.getByText("Resposta recebida")).toBeInTheDocument();
  });

  it("evento repetido não duplica mensagem nem linha do tempo", async () => {
    const { t, emit } = streaming(withJob());
    await open(t);
    const queued = { seq: 2, kind: "job.status" as const, payload: { job_id: "j9", status: "running", cancel_requested: false, error: null } };
    emit(queued);
    emit(queued);
    emit({ seq: 2, kind: "job.status", payload: { job_id: "j9", status: "running", cancel_requested: false, error: null } });
    expect(await screen.findByText("OneExecutor pegou o pedido")).toBeInTheDocument();
    expect(screen.getAllByText("OneExecutor pegou o pedido")).toHaveLength(1);
    expect(screen.getAllByText("Pedido enviado ao OneExecutor (Codex) para análise do projeto.")).toHaveLength(1);
  });

  it("cancelar pede ao OneHub e só mostra 'cancelado' depois do evento do executor", async () => {
    const { t, emit } = streaming(withJob([{ seq: 1, kind: "job.status", payload: { job_id: "j9", status: "running", cancel_requested: false, error: null } }]));
    const user = await open(t);
    await user.click(await screen.findByRole("button", { name: /Cancelar/ }));
    expect(t.cancelJob).toHaveBeenCalledWith("j9");
    emit({ seq: 2, kind: "job.status", payload: { job_id: "j9", status: "running", cancel_requested: true, error: null } });
    expect(await screen.findByText("Cancelamento solicitado")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Cancelar/ })).not.toBeInTheDocument();
    emit({ seq: 3, kind: "job.status", payload: { job_id: "j9", status: "cancelled", cancel_requested: true, error: null } });
    expect(await screen.findByText("Cancelado")).toBeInTheDocument();
  });

  it("acesso revogado encerra o acompanhamento com aviso claro", async () => {
    const { t, status } = streaming(withJob());
    await open(t);
    status("closed", "grant_revoked");
    expect(await screen.findByRole("status")).toHaveTextContent("O acesso deste OneNode a este projeto foi revogado");
    expect(screen.getByText("Sem atualizações")).toBeInTheDocument();
  });

  it("reconectando aparece como estado, não como erro", async () => {
    const { t, status } = streaming(withJob());
    await open(t);
    status("reconnecting");
    expect(screen.getByText("Reconectando…")).toBeInTheDocument();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });
});

describe("OneAgent Web: Markdown seguro", () => {
  it("código tem botão de copiar e HTML da resposta nunca vira elemento", async () => {
    const body = 'Veja:\n\n```ts\nconst x = 1;\n```\n\n<img src="x" onerror="alert(1)"> fim';
    const { t } = streaming({ messages: [msg({ id: "a7", body })] });
    render(<CentralApp transport={t} />);
    const container = document.body;
    expect(await screen.findByRole("button", { name: "Copiar código" })).toBeInTheDocument();
    expect(container.querySelector("code")?.textContent).toContain("const x = 1;");
    expect(container.querySelector('img[src="x"]')).toBeNull();
  });
});

describe("transporte HTTP do OneHub", () => {
  it("rejeita o fallback HTML da SPA como resposta inválida da API", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValue(new Response("<!doctype html><html></html>", {
      status: 200,
      headers: { "content-type": "text/html" },
    }));
    const t = createHttpTransport("/central-api");
    await expect(t.conversations()).rejects.toMatchObject({ status: 200, code: "invalid_response" });
  });

  it("sem token no navegador: cookie enviado pelo navegador e nada guardado em armazenamento", async () => {
    const calls: { url: string; init?: RequestInit }[] = [];
    vi.spyOn(globalThis, "fetch").mockImplementation(async (url, init) => {
      calls.push({ url: String(url), init });
      return new Response(JSON.stringify({ token: "nunca-guardado" }), { status: 200 });
    });
    const t = createHttpTransport("/central-api");
    await t.login("a@b.co", "senha-forte-123");
    await t.conversations();
    expect(calls.every((c) => c.init?.credentials === "include")).toBe(true);
    expect(calls.some((c) => (c.init?.headers as Record<string, string> | undefined)?.authorization)).toBe(false);
    expect(localStorage.length + sessionStorage.length).toBe(0);
  });

  it("stream: reconecta com a sequência, entrega cada evento uma vez e encerra com o motivo do OneHub", async () => {
    const sources: FakeSource[] = [];
    class FakeSource {
      static CLOSED = 2;
      url: string;
      readyState = 0;
      withCredentials: boolean;
      listeners = new Map<string, ((ev: { lastEventId: string; data: string; type: string }) => void)[]>();
      onopen: (() => void) | null = null;
      onerror: (() => void) | null = null;
      constructor(url: string, init?: { withCredentials?: boolean }) {
        this.url = url;
        this.withCredentials = Boolean(init?.withCredentials);
        sources.push(this);
      }
      addEventListener(type: string, fn: (ev: { lastEventId: string; data: string; type: string }) => void) {
        this.listeners.set(type, [...(this.listeners.get(type) ?? []), fn]);
      }
      close() {
        this.readyState = 2;
      }
      fire(type: string, id: string, data: unknown) {
        for (const fn of this.listeners.get(type) ?? []) fn({ lastEventId: id, data: JSON.stringify(data), type });
      }
    }
    vi.stubGlobal("EventSource", FakeSource);
    const seen: number[] = [];
    const statuses: string[] = [];
    const t = createHttpTransport("/central-api");
    const sub = t.subscribe("c1", {
      afterSeq: 3,
      onEvent: (ev) => seen.push(ev.seq),
      onStatus: (s, reason) => statuses.push(reason ? `${s}:${reason}` : s),
    });
    expect(sources[0]!.url).toBe("/central-api/v1/conversations/c1/events?after=3");
    expect(sources[0]!.withCredentials).toBe(true);
    sources[0]!.fire("message.created", "4", { message: {} });
    sources[0]!.fire("message.created", "4", { message: {} });
    sources[0]!.fire("job.status", "5", { job_id: "j", status: "running" });
    expect(seen).toEqual([4, 5]);
    sources[0]!.fire("closed", "", { reason: "grant_revoked" });
    expect(statuses.at(-1)).toBe("closed:grant_revoked");
    sub.close();
    vi.unstubAllGlobals();
  });
});

describe("OneAgent Web: falhas de rede e silêncio do stream", () => {
  it("falha de rede ao abrir não leva à tela de login: mostra indisponibilidade e permite tentar de novo", async () => {
    const me = vi.fn().mockRejectedValueOnce(new OneHubError(502, "erro")).mockResolvedValueOnce(USER);
    render(<CentralApp transport={fakeTransport({ me })} />);
    expect(await screen.findByRole("alert")).toHaveTextContent("OneHub Central indisponível");
    expect(screen.queryByLabelText("E-mail")).not.toBeInTheDocument();
    await userEvent.setup().click(screen.getByRole("button", { name: "Tentar de novo" }));
    expect(await screen.findByRole("button", { name: /Plano/ })).toBeInTheDocument();
  });

  it("sem sinal do servidor, o estado vira 'reconectando' e volta a 'live' quando o ping chega", async () => {
    vi.useFakeTimers({ toFake: ["setInterval", "clearInterval", "Date"] });
    try {
      const statuses: string[] = [];
      const sources: { listeners: Map<string, () => void>; readyState: number; onopen: (() => void) | null }[] = [];
      class Src {
        readyState = 0;
        onopen: (() => void) | null = null;
        onerror: (() => void) | null = null;
        listeners = new Map<string, () => void>();
        constructor() {
          sources.push(this);
        }
        addEventListener(type: string, fn: () => void) {
          this.listeners.set(type, fn);
        }
        close() {
          this.readyState = 2;
        }
      }
      vi.stubGlobal("EventSource", Src);
      const t = createHttpTransport("/central-api");
      const sub = t.subscribe("c1", { afterSeq: 0, onEvent: () => undefined, onStatus: (s) => statuses.push(s) });
      sources[0]!.onopen?.();
      expect(statuses.at(-1)).toBe("live");
      // a verificação roda a cada 2 s: depois de 15 s sem sinal, a tela já mostra "reconectando"
      vi.advanceTimersByTime(15_000);
      expect(statuses.at(-1)).toBe("reconnecting");
      sources[0]!.listeners.get("ping")?.();
      vi.advanceTimersByTime(2_000);
      expect(statuses.at(-1)).toBe("live");
      sub.close();
    } finally {
      vi.useRealTimers();
      vi.unstubAllGlobals();
    }
  });
});

describe("OneCoding: configurações e estado do Realtime", () => {
  it("sem conversa ativa, o indicador mostra 'Aguardando conversa' e não 'Conectando'", async () => {
    render(<CentralApp transport={fakeTransport({ me: vi.fn(async () => USER), conversations: vi.fn(async () => []) })} />);
    expect(await screen.findByText("Aguardando conversa")).toBeInTheDocument();
    expect(screen.queryByText("Conectando…")).not.toBeInTheDocument();
  });

  it("administrador vê Configurações e gera código de pareamento de OneNode mostrado uma única vez", async () => {
    const t = fakeTransport({ me: vi.fn(async () => USER) });
    const user = userEvent.setup();
    render(<CentralApp transport={t} />);
    await user.click(await screen.findByRole("button", { name: "OneNode" }));
    expect(await screen.findByRole("heading", { name: "OneNodes" })).toBeInTheDocument();
    expect(window.location.pathname).toBe("/configuracoes/OneNodes");
    await user.click(screen.getByRole("button", { name: "Adicionar OneNode" }));
    await user.type(screen.getByLabelText("Nome do OneNode"), "Servidor novo");
    await user.click(screen.getByRole("button", { name: "Gerar código do OneNode" }));
    expect(t.createNode).toHaveBeenCalledWith("Servidor novo");
    expect(await screen.findByText("CODIGO-TESTE-1")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Entendi, ocultar" }));
    expect(screen.queryByText("CODIGO-TESTE-1")).not.toBeInTheDocument();
  });

  it("administrador abre a seção OneNode das configurações", async () => {
    const t = fakeTransport({ me: vi.fn(async () => USER) });
    const user = userEvent.setup();
    render(<CentralApp transport={t} />);
    await user.click(await screen.findByRole("button", { name: "OneNode" }));
    expect(await screen.findByRole("heading", { level: 2, name: "OneNodes" })).toBeInTheDocument();
    expect(await screen.findByRole("button", { name: "Adicionar OneNode" })).toBeInTheDocument();
    expect(t.nodes).toHaveBeenCalled();
  });

  it("membro não vê o menu OneNodes", async () => {
    const member = { id: "u2", email: "membro@teste.local", role: "member" as const };
    render(<CentralApp transport={fakeTransport({ me: vi.fn(async () => member) })} />);
    await screen.findByText("Aguardando conversa");
    expect(screen.queryByRole("button", { name: "OneNodes" })).not.toBeInTheDocument();
  });

  it("membro não vê Configurações e o painel recusa o acesso", async () => {
    const member = { id: "u2", email: "membro@teste.local", role: "member" as const };
    render(<CentralApp transport={fakeTransport({ me: vi.fn(async () => member) })} />);
    await screen.findByRole("button", { name: /Plano/ });
    expect(screen.queryByRole("button", { name: "Configurações" })).not.toBeInTheDocument();
  });

  it("revogar pede confirmação e não chama a API se o administrador cancelar", async () => {
    const t = fakeTransport();
    const user = userEvent.setup();
    vi.spyOn(window, "confirm").mockReturnValue(false);
    render(<CentralApp transport={fakeTransport({ me: vi.fn(async () => USER), ...{} })} />);
    await user.click(await screen.findByRole("button", { name: "OneNode" }));
    await user.click((await screen.findAllByRole("button", { name: "Revogar" }))[0]!);
    expect(window.confirm).toHaveBeenCalled();
    expect(t.revokeNode).not.toHaveBeenCalled();
    expect(t.revokeExecutor).not.toHaveBeenCalled();
  });
});
