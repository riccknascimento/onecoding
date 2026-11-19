import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { LocalGatewayState } from "../App";
import type { EngineStatus, GatewayInfo, ProviderStatus, RunDetail, RunEvent, RunView, Workspace } from "../contracts";
import { NO_CONTEXT } from "../chat/model";
import { ChatPage } from "../pages/ChatPage";

vi.mock("../api", () => ({
  inTauri: () => true,
  listWorkspaces: vi.fn(),
  authorizeWorkspace: vi.fn(),
  startRun: vi.fn(),
  cancelRun: vi.fn(),
  listRuns: vi.fn(),
  getRun: vi.fn(),
  getOrchestratorStatus: vi.fn(),
  connectChatGptPlan: vi.fn(),
  refreshChatGptModels: vi.fn(),
  disconnectChatGptPlan: vi.fn(),
  listChatConversations: vi.fn(),
  listChatMessages: vi.fn(),
  sendChatMessage: vi.fn(),
  setChatConversationContext: vi.fn(),
  cancelChatGeneration: vi.fn(),
}));
import * as api from "../api";

const ws: Workspace = { id: "ws1", gateway_id: "local", label: "projeto", path: "/tmp/projeto" };

const codex = (trust: EngineStatus["trust"]): EngineStatus => ({
  engine: "codex", installed: true, path: "/x/codex", version: "0.1", auth: "logged_in", trust,
  fingerprint: "ab", models: [], note: null, execution_block: null,
});

const local = (trust: EngineStatus["trust"]): LocalGatewayState => {
  const gateway: GatewayInfo = {
    id: "local", name: "Este Mac", kind: "local", status: "online",
    os: "macos-aarch64", protocol_version: 1, engines: [codex(trust)], demo: false,
  };
  return { phase: "ready", gateway };
};

const runView = (over: Partial<RunView> = {}): RunView => ({
  run_id: "rn1", task_id: "tk1", gateway_id: "local", workspace_id: "ws1", workspace_label: "projeto",
  engine: "codex", model_requested: null, model_reported: null, mode: "read_only", prompt: "Resuma o projeto",
  status: "running", created_at: "2026-11-17T12:00:00Z", started_at: null, finished_at: null, duration_ms: null,
  exit_code: null, engine_session_id: null,
  usage: { input_tokens: null, output_tokens: null, cached_tokens: null, reasoning_tokens: null },
  tool_calls: 0, attempts: 1, initiator: "user", error_kind: null, error_message: null, final_text: null, ...over,
});

const ev = (seq: number, payload: RunEvent["payload"]): RunEvent => ({ run_id: "rn1", seq, ts: "t", payload });

beforeEach(() => {
  vi.mocked(api.listWorkspaces).mockResolvedValue([ws]);
  vi.mocked(api.listRuns).mockResolvedValue([]);
  vi.mocked(api.startRun).mockReset();
  vi.mocked(api.getRun).mockReset();
  vi.mocked(api.cancelRun).mockReset();
  vi.mocked(api.getOrchestratorStatus).mockResolvedValue({
    state: "disconnected",
    reason: "ChatGPT Plus não conectado nesta fase.",
  });
  vi.mocked(api.connectChatGptPlan).mockReset();
  vi.mocked(api.refreshChatGptModels).mockReset();
  vi.mocked(api.disconnectChatGptPlan).mockReset();
  vi.mocked(api.listChatConversations).mockResolvedValue([]);
  vi.mocked(api.listChatMessages).mockResolvedValue([]);
  vi.mocked(api.sendChatMessage).mockReset();
  vi.mocked(api.setChatConversationContext).mockReset();
  vi.mocked(api.setChatConversationContext).mockResolvedValue(undefined);
  vi.mocked(api.cancelChatGeneration).mockReset();
});

describe("Chat com execução real", () => {
  it("Modo Direto sem Codex aprovado: execução desabilitada, sem conversa fictícia", async () => {
    render(<ChatPage local={local("unknown")} />);
    expect(screen.getByRole("radio", { name: "VibeAgent" })).toBeChecked();
    const user = userEvent.setup();
    await user.click(screen.getByRole("radio", { name: "Coding" }));
    // Modo Direto exige OneNode escolhido explicitamente (sem escolha automática)
    await user.selectOptions(screen.getByRole("combobox", { name: "OneNode" }), "local");
    expect(screen.getByText("Coding · ChatGPT não participa desta execução.")).toBeInTheDocument();
    expect(screen.getByRole("textbox", { name: "Mensagem" })).toBeDisabled();
    expect(screen.getByText(/Aprove o Codex CLI em Agent Gateways/)).toBeInTheDocument();
    expect(screen.queryByText("Orquestrador", { selector: ".msg-role" })).not.toBeInTheDocument();
    expect(api.startRun).not.toHaveBeenCalled();
  });

  it("Modo Orquestrador desconectado não encaminha mensagens ao Codex", async () => {
    const user = userEvent.setup();
    render(<ChatPage local={local("trusted")} />);
    await user.click(screen.getByRole("radio", { name: "VibeAgent" }));
    expect(screen.getByText("Provedor configurado: nenhum")).toBeInTheDocument();
    expect(await screen.findByText(/Estado: desconectado/)).toBeInTheDocument();
    expect(screen.getByText("ChatGPT não conectado. Conecte uma conta para usar o VibeAgent.")).toBeInTheDocument();
    expect(screen.getByRole("textbox", { name: "Mensagem" })).toBeDisabled();
    expect(api.startRun).not.toHaveBeenCalled();
  });

  it("envia uma mensagem explícita ao ChatGPT e mostra resposta e uso persistidos", async () => {
    const user = userEvent.setup();
    vi.mocked(api.getOrchestratorStatus).mockResolvedValue({
      state: "connected", provider: "chatgpt_plan", label: "ChatGPT Plus",
      models: ["GPT test (gpt-test)"], plan_usage_authorized: true,
    });
    vi.mocked(api.sendChatMessage).mockResolvedValue("cv1");
    vi.mocked(api.listChatMessages).mockResolvedValue([
      { id: "u1", conversationId: "cv1", role: "user", text: "Me ajude a planejar", provider: "chatgpt_plan", model: "gpt-test", state: "completed", errorMessage: null, usage: { input_tokens: null, output_tokens: null, cached_tokens: null, reasoning_tokens: null }, ts: "2026-11-17T12:00:00Z" },
      { id: "a1", conversationId: "cv1", role: "orchestrator", text: "Vamos decompor a demanda.", provider: "chatgpt_plan", model: "gpt-test", state: "completed", errorMessage: null, usage: { input_tokens: 12, output_tokens: 7, cached_tokens: null, reasoning_tokens: null }, ts: "2026-11-17T12:00:01Z" },
    ]);
    render(<ChatPage local={local("trusted")} />);
    const box = await screen.findByRole("textbox", { name: "Mensagem" });
    await user.type(box, "Me ajude a planejar");
    await user.click(screen.getByRole("button", { name: "Enviar" }));
    expect(await screen.findByText("Vamos decompor a demanda.")).toBeInTheDocument();
    expect(api.sendChatMessage).toHaveBeenCalledWith(null, "Me ajude a planejar", "gpt-test", expect.objectContaining({ orchestratorModel: "gpt-test", executorEngine: "codex" }));
    expect(api.startRun).not.toHaveBeenCalled();
    expect(screen.getByText(/12 entrada · 7 saída/)).toBeInTheDocument();
  });

  it("recupera histórico persistido após remontagem sem iniciar uma nova chamada", async () => {
    vi.mocked(api.getOrchestratorStatus).mockResolvedValue({
      state: "connected", provider: "chatgpt_plan", label: "ChatGPT Plus",
      models: ["GPT test (gpt-test)"], plan_usage_authorized: true,
    });
    vi.mocked(api.listChatConversations).mockResolvedValue([{ id: "cv-saved", title: "Plano salvo", provider: "chatgpt_plan", model: "gpt-test", state: "idle", updatedAt: "2026-11-17T12:00:00Z", context: NO_CONTEXT }]);
    vi.mocked(api.listChatMessages).mockResolvedValue([{ id: "m-saved", conversationId: "cv-saved", role: "orchestrator", text: "Histórico recuperado do SQLite.", provider: "chatgpt_plan", model: "gpt-test", state: "completed", errorMessage: null, usage: { input_tokens: 8, output_tokens: 5, cached_tokens: null, reasoning_tokens: null }, ts: "2026-11-17T12:00:00Z" }]);
    const first = render(<ChatPage local={local("trusted")} />);
    expect(await screen.findByText("Histórico recuperado do SQLite.")).toBeInTheDocument();
    first.unmount();
    render(<ChatPage local={local("trusted")} />);
    expect(await screen.findByText("Histórico recuperado do SQLite.")).toBeInTheDocument();
    expect(api.sendChatMessage).not.toHaveBeenCalled();
  });

  it("provedor simulado é identificado como teste e nunca como ChatGPT conectado", async () => {
    const simulated: ProviderStatus = { state: "simulated", provider: "scripted", label: "somente testes" };
    vi.mocked(api.getOrchestratorStatus).mockResolvedValue(simulated);
    const user = userEvent.setup();
    render(<ChatPage local={local("trusted")} />);
    await user.click(screen.getByRole("radio", { name: "VibeAgent" }));
    expect(await screen.findByText("Estado: simulado (somente testes)")).toBeInTheDocument();
    expect(screen.queryByText("Estado: conectado")).not.toBeInTheDocument();
    expect(screen.getByRole("textbox", { name: "Mensagem" })).toBeDisabled();
    expect(api.startRun).not.toHaveBeenCalled();
  });

  it("falha ao consultar estado do provedor aparece como indisponível", async () => {
    vi.mocked(api.getOrchestratorStatus).mockRejectedValue(new Error("status offline"));
    const user = userEvent.setup();
    render(<ChatPage local={local("trusted")} />);
    await user.click(screen.getByRole("radio", { name: "VibeAgent" }));
    expect(await screen.findByText("Estado: indisponível · status offline")).toBeInTheDocument();
    expect(screen.getByRole("textbox", { name: "Mensagem" })).toBeDisabled();
  });

  it("mostra conexão real e habilita o chat sem executar ferramentas", async () => {
    vi.mocked(api.getOrchestratorStatus).mockResolvedValue({
      state: "connected",
      provider: "chatgpt_plan",
      label: "ChatGPT Plus",
      models: ["GPT test (gpt-test)"],
      plan_usage_authorized: true,
    });
    const user = userEvent.setup();
    render(<ChatPage local={local("trusted")} />);
    await user.click(screen.getByRole("radio", { name: "VibeAgent" }));
    expect(await screen.findByText("Estado: conectado · ChatGPT Plus")).toBeInTheDocument();
    expect(screen.getByText("ChatGPT Plan Usage conectado. O chat não executa ferramentas nem tarefas nesta fase.")).toBeInTheDocument();
    expect(screen.getByText("Provedor configurado: ChatGPT Plan Usage · ChatGPT Plus")).toBeInTheDocument();
    expect(screen.getByRole("textbox", { name: "Mensagem" })).toBeEnabled();
    expect(screen.getByRole("combobox", { name: "Modelo do ChatGPT" })).toHaveValue("gpt-test");
    expect(api.startRun).not.toHaveBeenCalled();
  });

  it("login atualiza estado e modelos sem iniciar uma chamada de resposta", async () => {
    const user = userEvent.setup();
    vi.mocked(api.connectChatGptPlan).mockResolvedValue({
      state: "connected", provider: "chatgpt_plan", label: "conta Plus",
      models: ["GPT X (gpt-x)"], plan_usage_authorized: true,
    });
    render(<ChatPage local={local("trusted")} />);
    await user.click(screen.getByRole("radio", { name: "VibeAgent" }));
    await user.click(await screen.findByRole("button", { name: "Conectar com ChatGPT" }));
    expect(await screen.findByText("ChatGPT Plan Usage: autorizado")).toBeInTheDocument();
    expect(screen.getByRole("combobox", { name: "Modelo do ChatGPT" })).toHaveValue("gpt-x");
    expect(api.connectChatGptPlan).toHaveBeenCalledOnce();
    expect(api.startRun).not.toHaveBeenCalled();
  });

  it("envia só identificadores e texto, acompanha eventos e mostra métricas reais", async () => {
    const user = userEvent.setup();
    vi.mocked(api.startRun).mockResolvedValue("rn1");
    const running: RunDetail = {
      run: runView(),
      events: [ev(1, { type: "status", status: "running" }), ev(2, { type: "message", text: "Vou listar os arquivos." })],
    };
    const done: RunDetail = {
      run: runView({
        status: "succeeded", duration_ms: 9890, tool_calls: 2, exit_code: 0,
        usage: { input_tokens: 45017, output_tokens: 169, cached_tokens: 34048, reasoning_tokens: 0 },
      }),
      events: [
        ev(3, { type: "tool_started", name: "shell", summary: "ls -la" }),
        ev(4, { type: "tool_finished", name: "shell", ok: true, exit_code: 0 }),
        ev(5, { type: "message", text: "Arquivos: README.md e hello.txt." }),
        ev(6, { type: "status", status: "succeeded" }),
      ],
    };
    vi.mocked(api.getRun).mockResolvedValueOnce(running).mockResolvedValue(done);
    vi.mocked(api.listRuns).mockResolvedValue([done.run]);

    render(<ChatPage local={local("trusted")} />);
    await user.click(screen.getByRole("radio", { name: "Coding" }));
    // Modo Direto exige OneNode escolhido explicitamente (sem escolha automática)
    await user.selectOptions(screen.getByRole("combobox", { name: "OneNode" }), "local");
    const box = screen.getByRole("textbox", { name: "Mensagem" });
    await waitFor(() => expect(box).toBeEnabled());
    expect(screen.queryByText("DEMO")).not.toBeInTheDocument();

    await user.type(box, "Resuma o projeto");
    await user.click(screen.getByRole("button", { name: "Enviar" }));
    expect(api.startRun).toHaveBeenCalledWith({ workspace_id: "ws1", engine: "codex", model: null, prompt: "Resuma o projeto" });

    expect(await screen.findByText("Vou listar os arquivos.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Cancelar" })).toBeInTheDocument();
    expect(await screen.findByText("Arquivos: README.md e hello.txt.")).toBeInTheDocument();
    expect(screen.getAllByText("Concluída", { selector: ".badge" }).length).toBeGreaterThan(0);
    const metrics = screen.getByLabelText("Métricas da execução");
    expect(metrics).toHaveTextContent("tokens entrada 45.017 / saída 169 / cache 34.048");
    expect(metrics).toHaveTextContent("não informado pelo motor");
    expect(metrics).toHaveTextContent("9.9 s");
    expect(screen.queryByRole("button", { name: "Cancelar" })).not.toBeInTheDocument();
    expect(screen.getByLabelText("Histórico de conversas e execuções")).toHaveTextContent("Resuma o projeto");
  });

  it("cancelar chama o backend para a execução ativa", async () => {
    const user = userEvent.setup();
    vi.mocked(api.startRun).mockResolvedValue("rn1");
    vi.mocked(api.getRun).mockResolvedValue({ run: runView(), events: [] });
    vi.mocked(api.cancelRun).mockResolvedValue(true);
    render(<ChatPage local={local("trusted")} />);
    await user.click(screen.getByRole("radio", { name: "Coding" }));
    // Modo Direto exige OneNode escolhido explicitamente (sem escolha automática)
    await user.selectOptions(screen.getByRole("combobox", { name: "OneNode" }), "local");
    await waitFor(() => expect(screen.getByRole("textbox", { name: "Mensagem" })).toBeEnabled());
    await user.type(screen.getByRole("textbox", { name: "Mensagem" }), "x");
    await user.click(screen.getByRole("button", { name: "Enviar" }));
    await user.click(await screen.findByRole("button", { name: "Cancelar" }));
    expect(api.cancelRun).toHaveBeenCalledWith("rn1");
  });

  it("erro do backend é exibido e não finge sucesso", async () => {
    const user = userEvent.setup();
    vi.mocked(api.startRun).mockRejectedValue("o executável mudou desde a aprovação; valide de novo em Agent Gateways");
    render(<ChatPage local={local("trusted")} />);
    await user.click(screen.getByRole("radio", { name: "Coding" }));
    // Modo Direto exige OneNode escolhido explicitamente (sem escolha automática)
    await user.selectOptions(screen.getByRole("combobox", { name: "OneNode" }), "local");
    await waitFor(() => expect(screen.getByRole("textbox", { name: "Mensagem" })).toBeEnabled());
    await user.type(screen.getByRole("textbox", { name: "Mensagem" }), "x");
    await user.click(screen.getByRole("button", { name: "Enviar" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("o executável mudou");
    expect(screen.queryByLabelText("Métricas da execução")).not.toBeInTheDocument();
  });

  it("execução falha mostra o erro do motor", async () => {
    const user = userEvent.setup();
    vi.mocked(api.startRun).mockResolvedValue("rn1");
    vi.mocked(api.getRun).mockResolvedValue({
      run: runView({ status: "failed", error_kind: "executor_failed", error_message: "model not supported", exit_code: 1 }),
      events: [ev(1, { type: "error", kind: "executor_failed", message: "model not supported" })],
    });
    render(<ChatPage local={local("trusted")} />);
    await user.click(screen.getByRole("radio", { name: "Coding" }));
    // Modo Direto exige OneNode escolhido explicitamente (sem escolha automática)
    await user.selectOptions(screen.getByRole("combobox", { name: "OneNode" }), "local");
    await waitFor(() => expect(screen.getByRole("textbox", { name: "Mensagem" })).toBeEnabled());
    await user.type(screen.getByRole("textbox", { name: "Mensagem" }), "x");
    await user.click(screen.getByRole("button", { name: "Enviar" }));
    expect((await screen.findAllByRole("alert")).length).toBeGreaterThan(0);
    expect(screen.getAllByText("Falhou", { selector: ".badge" }).length).toBeGreaterThan(0);
  });

  it("mostra o mesmo contexto operacional nos dois modos, e o modelo do ChatGPT só no Orquestrador", async () => {
    const user = userEvent.setup();
    vi.mocked(api.getOrchestratorStatus).mockResolvedValue({
      state: "connected", provider: "chatgpt_plan", label: "ChatGPT Plus",
      models: ["GPT test (gpt-test)"], plan_usage_authorized: true,
    });
    render(<ChatPage local={local("trusted")} />);
    for (const name of ["OneNode", "Projeto / Workspace", "Motor executor", "Modelo executor"]) {
      expect(await screen.findByRole("combobox", { name })).toBeInTheDocument();
    }
    expect(screen.getByRole("combobox", { name: "Modelo do ChatGPT" })).toBeInTheDocument();
    expect(screen.getByRole("option", { name: "Automático · futuro" })).toBeInTheDocument();
    await user.click(screen.getByRole("radio", { name: "Coding" }));
    // Modo Direto exige OneNode escolhido explicitamente (sem escolha automática)
    await user.selectOptions(screen.getByRole("combobox", { name: "OneNode" }), "local");
    for (const name of ["OneNode", "Projeto / Workspace", "Motor executor", "Modelo executor"]) {
      expect(screen.getByRole("combobox", { name })).toBeInTheDocument();
    }
    expect(screen.queryByRole("combobox", { name: "Modelo do ChatGPT" })).not.toBeInTheDocument();
    expect(screen.queryByRole("option", { name: "Automático · futuro" })).not.toBeInTheDocument();
  });

  it("não apresenta Gateway remoto, Claude Code nem Cursor como disponíveis", async () => {
    render(<ChatPage local={local("trusted")} />);
    expect(await screen.findByRole("option", { name: "Apliflow (demo) — desconectado" })).toBeDisabled();
    expect(screen.getByRole("option", { name: "Claude Code — indisponível" })).toBeDisabled();
    expect(screen.getByRole("option", { name: "Cursor CLI — indisponível" })).toBeDisabled();
    expect(screen.getByRole("option", { name: "Codex CLI" })).toBeEnabled();
  });

  it("escolher Gateway, projeto e executor só guarda preferência: não envia nem executa nada", async () => {
    const user = userEvent.setup();
    vi.mocked(api.getOrchestratorStatus).mockResolvedValue({
      state: "connected", provider: "chatgpt_plan", label: "ChatGPT Plus",
      models: ["GPT test (gpt-test)"], plan_usage_authorized: true,
    });
    vi.mocked(api.listChatConversations).mockResolvedValue([{ id: "cv1", title: "Plano", provider: "chatgpt_plan", model: "gpt-test", state: "idle", updatedAt: "2026-11-17T12:00:00Z", context: NO_CONTEXT }]);
    render(<ChatPage local={local("trusted")} />);
    await screen.findByText("Plano", { selector: ".chat-title" });
    await user.selectOptions(screen.getByRole("combobox", { name: "Motor executor" }), "auto");
    await waitFor(() => expect(api.setChatConversationContext).toHaveBeenCalledWith("cv1", {
      gatewayId: "local", workspaceId: "ws1", executorEngine: "auto", executorModel: null, orchestratorModel: "gpt-test",
    }));
    expect(api.startRun).not.toHaveBeenCalled();
    expect(api.sendChatMessage).not.toHaveBeenCalled();
  });

  it("reabrir uma conversa recupera Gateway, projeto, executor e modelos salvos no SQLite", async () => {
    const user = userEvent.setup();
    vi.mocked(api.getOrchestratorStatus).mockResolvedValue({
      state: "connected", provider: "chatgpt_plan", label: "ChatGPT Plus",
      models: ["GPT A (gpt-a)", "GPT B (gpt-b)"], plan_usage_authorized: true,
    });
    vi.mocked(api.listWorkspaces).mockResolvedValue([ws, { ...ws, id: "ws2", label: "outro", path: "/tmp/outro" }]);
    vi.mocked(api.listChatConversations).mockResolvedValue([
      { id: "novo", title: "Sem contexto", provider: "chatgpt_plan", model: "gpt-a", state: "idle", updatedAt: "2026-11-17T13:00:00Z", context: NO_CONTEXT },
      { id: "velho", title: "Com contexto", provider: "chatgpt_plan", model: "gpt-a", state: "idle", updatedAt: "2026-11-17T12:00:00Z",
        context: { gatewayId: "local", workspaceId: "ws2", executorEngine: "auto", executorModel: null, orchestratorModel: "gpt-b" } },
    ]);
    render(<ChatPage local={local("trusted")} />);
    await user.click(await screen.findByRole("button", { name: /Com contexto/ }));
    await waitFor(() => expect(screen.getByRole("combobox", { name: "Projeto / Workspace" })).toHaveValue("ws2"));
    expect(screen.getByRole("combobox", { name: "Motor executor" })).toHaveValue("auto");
    expect(screen.getByRole("combobox", { name: "Modelo do ChatGPT" })).toHaveValue("gpt-b");
    expect(screen.getByRole("combobox", { name: "OneNode" })).toHaveValue("local");
  });

  it("o histórico único identifica o modo de cada item e abrir uma execução leva ao Modo Direto", async () => {
    const user = userEvent.setup();
    vi.mocked(api.listChatConversations).mockResolvedValue([{ id: "cv1", title: "Plano salvo", provider: "chatgpt_plan", model: "gpt-test", state: "idle", updatedAt: "2026-11-17T12:00:00Z", context: NO_CONTEXT }]);
    const done = runView({ status: "succeeded", created_at: "2026-11-17T13:00:00Z" });
    vi.mocked(api.listRuns).mockResolvedValue([done]);
    vi.mocked(api.getRun).mockResolvedValue({ run: done, events: [ev(1, { type: "message", text: "Resposta da execução." })] });
    render(<ChatPage local={local("trusted")} />);
    const rail = await screen.findByLabelText("Histórico de conversas e execuções");
    expect(await within(rail).findByText("Plano salvo")).toBeInTheDocument();
    expect(within(rail).getByText("Resuma o projeto")).toBeInTheDocument();
    expect(within(rail).getByText("VibeAgent")).toBeInTheDocument();
    expect(within(rail).getByText("Coding")).toBeInTheDocument();
    await user.click(within(rail).getByRole("button", { name: /Resuma o projeto/ }));
    expect(screen.getByRole("radio", { name: "Coding" })).toBeChecked();
    expect(await screen.findByText("Resposta da execução.")).toBeInTheDocument();
    expect(screen.getByText("Coding · ChatGPT não participa desta execução.")).toBeInTheDocument();
  });
});
