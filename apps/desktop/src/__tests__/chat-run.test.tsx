import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { LocalGatewayState } from "../App";
import type { EngineStatus, GatewayInfo, ProviderStatus, RunDetail, RunEvent, RunView, Workspace } from "../contracts";
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
}));
import * as api from "../api";

const ws: Workspace = { id: "ws1", gateway_id: "local", label: "projeto", path: "/tmp/projeto" };

const codex = (trust: EngineStatus["trust"]): EngineStatus => ({
  engine: "codex", installed: true, path: "/x/codex", version: "0.1", auth: "logged_in", trust,
  fingerprint: "ab", models: [], note: null,
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
});

describe("Chat com execução real", () => {
  it("Modo Direto sem Codex aprovado: execução desabilitada, sem conversa fictícia", async () => {
    render(<ChatPage local={local("unknown")} />);
    expect(screen.getByRole("combobox", { name: "Modo" })).toHaveValue("direct");
    expect(screen.getByText("Modo Direto · ChatGPT não participa desta execução.")).toBeInTheDocument();
    expect(screen.getByRole("textbox", { name: "Mensagem" })).toBeDisabled();
    expect(screen.getByText(/Aprove o Codex CLI em Agent Gateways/)).toBeInTheDocument();
    expect(screen.queryByText("Orquestrador", { selector: ".msg-role" })).not.toBeInTheDocument();
    expect(api.startRun).not.toHaveBeenCalled();
  });

  it("Modo Orquestrador desconectado não encaminha mensagens ao Codex", async () => {
    const user = userEvent.setup();
    render(<ChatPage local={local("trusted")} />);
    await user.selectOptions(screen.getByRole("combobox", { name: "Modo" }), "orchestrator");
    expect(screen.getByText("Provedor configurado: nenhum · ChatGPT Plan Usage previsto na Fase 3B")).toBeInTheDocument();
    expect(await screen.findByText(/Estado: desconectado/)).toBeInTheDocument();
    expect(screen.getByText("ChatGPT não conectado. Conecte uma conta na Fase 3B para usar o Modo Orquestrador.")).toBeInTheDocument();
    expect(screen.getByRole("textbox", { name: "Mensagem" })).toBeDisabled();
    expect(api.startRun).not.toHaveBeenCalled();
  });

  it("provedor simulado é identificado como teste e nunca como ChatGPT conectado", async () => {
    const simulated: ProviderStatus = { state: "simulated", provider: "scripted", label: "somente testes" };
    vi.mocked(api.getOrchestratorStatus).mockResolvedValue(simulated);
    const user = userEvent.setup();
    render(<ChatPage local={local("trusted")} />);
    await user.selectOptions(screen.getByRole("combobox", { name: "Modo" }), "orchestrator");
    expect(await screen.findByText("Estado: simulado (somente testes)")).toBeInTheDocument();
    expect(screen.queryByText("Estado: conectado")).not.toBeInTheDocument();
    expect(screen.getByRole("textbox", { name: "Mensagem" })).toBeDisabled();
    expect(api.startRun).not.toHaveBeenCalled();
  });

  it("falha ao consultar estado do provedor aparece como indisponível", async () => {
    vi.mocked(api.getOrchestratorStatus).mockRejectedValue(new Error("status offline"));
    const user = userEvent.setup();
    render(<ChatPage local={local("trusted")} />);
    await user.selectOptions(screen.getByRole("combobox", { name: "Modo" }), "orchestrator");
    expect(await screen.findByText("Estado: indisponível · status offline")).toBeInTheDocument();
    expect(screen.getByRole("textbox", { name: "Mensagem" })).toBeDisabled();
  });

  it("mostra conexão real sem habilitar envio ainda não implementado", async () => {
    vi.mocked(api.getOrchestratorStatus).mockResolvedValue({
      state: "connected",
      provider: "chatgpt_plan",
      label: "ChatGPT Plus",
      models: [],
    });
    const user = userEvent.setup();
    render(<ChatPage local={local("trusted")} />);
    await user.selectOptions(screen.getByRole("combobox", { name: "Modo" }), "orchestrator");
    expect(await screen.findByText("Estado: conectado · ChatGPT Plus")).toBeInTheDocument();
    expect(screen.getByText("Provedor conectado; o envio do Modo Orquestrador ainda não está disponível nesta fase.")).toBeInTheDocument();
    expect(screen.getByText("Provedor configurado: ChatGPT Plus")).toBeInTheDocument();
    expect(screen.getByRole("textbox", { name: "Mensagem" })).toBeDisabled();
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
    expect(screen.getByLabelText("Histórico de execuções")).toHaveTextContent("Resuma o projeto");
  });

  it("cancelar chama o backend para a execução ativa", async () => {
    const user = userEvent.setup();
    vi.mocked(api.startRun).mockResolvedValue("rn1");
    vi.mocked(api.getRun).mockResolvedValue({ run: runView(), events: [] });
    vi.mocked(api.cancelRun).mockResolvedValue(true);
    render(<ChatPage local={local("trusted")} />);
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
    await waitFor(() => expect(screen.getByRole("textbox", { name: "Mensagem" })).toBeEnabled());
    await user.type(screen.getByRole("textbox", { name: "Mensagem" }), "x");
    await user.click(screen.getByRole("button", { name: "Enviar" }));
    expect((await screen.findAllByRole("alert")).length).toBeGreaterThan(0);
    expect(screen.getAllByText("Falhou", { selector: ".badge" }).length).toBeGreaterThan(0);
  });
});
