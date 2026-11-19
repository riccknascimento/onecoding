import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { ProviderPage, accountState, errorMessage } from "../central/ProviderPage";
import { OneHubError } from "../transport";
import type { CliConnection, CentralExecutor, OneHubTransport, ProviderConnection } from "../transport";

const CLAUDE_01: CliConnection = {
  id: "c1", name: "Claude Empresa 01", engine: "claude", executor_id: "e1", executor_name: "OneExecutor VPS",
  usage_authorized: true, auth_state: "authenticated", auth_checked_at: "2026-11-19T12:00:00Z", session: null,
};
const CODEX_01: CliConnection = {
  id: "c2", name: "Codex principal", engine: "codex", executor_id: "e1", executor_name: "OneExecutor VPS",
  usage_authorized: true, auth_state: "unauthenticated", auth_checked_at: "2026-11-19T12:00:00Z", session: null,
};
const CURSOR_01: CliConnection = {
  id: "c3", name: "Cursor equipe", engine: "cursor", executor_id: "e1", executor_name: "OneExecutor VPS",
  usage_authorized: true, auth_state: "unknown", auth_checked_at: null, session: null,
};
const EXECUTOR: CentralExecutor = { id: "e1", name: "OneExecutor VPS", engine: "codex", status: "online" };
const API: ProviderConnection = {
  id: "p1", name: "OpenRouter time", provider: "openrouter", key_hint: "••••WXYZ", default_model: null, status: "unverified", created_at: "2026-11-19T12:00:00Z",
};

function fakeTransport(over: Partial<Record<keyof OneHubTransport, unknown>> = {}) {
  const base = {
    cliConnections: vi.fn(async () => [CLAUDE_01, CODEX_01, CURSOR_01]),
    providerConnections: vi.fn(async () => [API]),
    executors: vi.fn(async () => [EXECUTOR]),
    startCliAction: vi.fn(async () => ({ id: "s1", action: "login", state: "pending", url: null, device_code: null, awaiting_code: false, result: null })),
    cancelCliSession: vi.fn(async () => undefined),
    submitCliCode: vi.fn(async () => undefined),
    setCliUsage: vi.fn(async () => undefined),
    deleteCliConnection: vi.fn(async () => undefined),
    createCliConnection: vi.fn(async () => ({})),
    createProviderConnection: vi.fn(async () => ({})),
    deleteProviderConnection: vi.fn(async () => undefined),
    ...over,
  };
  return base as unknown as OneHubTransport & typeof base;
}

describe("One Provider AI: estados reais das contas de CLI", () => {
  it("nomeia cada estado a partir do que o OneHub verificou", () => {
    expect(accountState(CLAUDE_01)).toEqual({ text: "Conectada", tone: "ok" });
    expect(accountState(CODEX_01)).toEqual({ text: "Desconectada", tone: "bad" });
    expect(accountState(CURSOR_01)).toEqual({ text: "Não verificada", tone: "warn" });
    expect(accountState({ ...CODEX_01, session: { id: "s", action: "login", state: "running", url: null, device_code: null, awaiting_code: false, result: null } }))
      .toEqual({ text: "Login em andamento", tone: "info" });
    expect(accountState({ ...CODEX_01, session: { id: "s", action: "login", state: "awaiting_code", url: null, device_code: null, awaiting_code: true, result: null } }))
      .toEqual({ text: "Aguardando o código", tone: "info" });
  });

  it("mostra cada motor com a conta e o estado que vieram do OneHub", async () => {
    render(<ProviderPage transport={fakeTransport()} isAdmin />);
    const claude = await screen.findByRole("list", { name: "Contas Claude Code" });
    expect(within(claude).getByText("Claude Empresa 01")).toBeInTheDocument();
    expect(within(claude).getByText("Conectada")).toBeInTheDocument();
    const codex = screen.getByRole("list", { name: "Contas Codex CLI" });
    expect(within(codex).getByText("Desconectada")).toBeInTheDocument();
    expect(within(screen.getByRole("list", { name: "Contas Cursor CLI" })).getByText("Não verificada")).toBeInTheDocument();
  });

  it("'Conectar conta' pede o login ao OneHub, sem nenhum segredo na página", async () => {
    const t = fakeTransport();
    render(<ProviderPage transport={t} isAdmin />);
    const codex = await screen.findByRole("list", { name: "Contas Codex CLI" });
    await userEvent.click(within(codex).getByRole("button", { name: "Conectar conta" }));
    expect(t.startCliAction).toHaveBeenCalledWith("c2", "login");
    expect(await screen.findByText("Login iniciado. Acompanhe abaixo.")).toBeInTheDocument();
  });

  it("mostra o link oficial e o código de dispositivo durante o login", async () => {
    const running: CliConnection = {
      ...CODEX_01,
      session: { id: "s1", action: "login", state: "running", url: "https://auth.openai.com/codex/device", device_code: "ABCD-1234", awaiting_code: false, result: null },
    };
    render(<ProviderPage transport={fakeTransport({ cliConnections: vi.fn(async () => [running]) })} isAdmin />);
    const link = await screen.findByRole("link", { name: "Abrir página oficial de autorização" });
    expect(link).toHaveAttribute("href", "https://auth.openai.com/codex/device");
    expect(link).toHaveAttribute("target", "_blank");
    expect(link).toHaveAttribute("rel", expect.stringContaining("noopener"));
    expect(screen.getByLabelText("Código de autorização")).toHaveTextContent("ABCD-1234");
  });

  it("quando a CLI pede o código colado, envia pelo OneHub e limpa o campo", async () => {
    const waiting: CliConnection = {
      ...CLAUDE_01, auth_state: "unauthenticated",
      session: { id: "s9", action: "login", state: "awaiting_code", url: "https://claude.ai/oauth/authorize", device_code: null, awaiting_code: true, result: null },
    };
    const t = fakeTransport({ cliConnections: vi.fn(async () => [waiting]) });
    render(<ProviderPage transport={t} isAdmin />);
    const form = await screen.findByRole("form", { name: "Código da autorização" });
    const input = within(form).getByRole("textbox");
    await userEvent.type(input, "  ABC123-XYZ  ");
    await userEvent.click(within(form).getByRole("button", { name: "Enviar código" }));
    expect(t.submitCliCode).toHaveBeenCalledWith("s9", "ABC123-XYZ");
    await waitFor(() => expect(input).toHaveValue(""));
  });

  it("cancelar a operação pede o cancelamento à sessão, não encerra nada na página", async () => {
    const running: CliConnection = {
      ...CODEX_01, session: { id: "s2", action: "login", state: "running", url: null, device_code: null, awaiting_code: false, result: null },
    };
    const t = fakeTransport({ cliConnections: vi.fn(async () => [running]) });
    render(<ProviderPage transport={t} isAdmin />);
    await userEvent.click(await screen.findByRole("button", { name: "Cancelar" }));
    expect(t.cancelCliSession).toHaveBeenCalledWith("s2");
  });

  it("uma conta já em uso não aceita outro login, e a mensagem vem do OneHub em português", async () => {
    const t = fakeTransport({
      startCliAction: vi.fn(async () => { throw new OneHubError(409, "session_in_progress"); }),
    });
    render(<ProviderPage transport={t} isAdmin />);
    const codex = await screen.findByRole("list", { name: "Contas Codex CLI" });
    await userEvent.click(within(codex).getByRole("button", { name: "Conectar conta" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Já existe uma operação em andamento nesta conta.");
  });

  it("desconectar só fica disponível para conta autenticada; remover exige conta desconectada", async () => {
    render(<ProviderPage transport={fakeTransport()} isAdmin />);
    const claude = await screen.findByRole("list", { name: "Contas Claude Code" });
    expect(within(claude).getByRole("button", { name: "Desconectar" })).toBeEnabled();
    expect(within(claude).getByRole("button", { name: "Remover" })).toBeDisabled();
    const codex = screen.getByRole("list", { name: "Contas Codex CLI" });
    expect(within(codex).getByRole("button", { name: "Desconectar" })).toBeDisabled();
    expect(within(codex).getByRole("button", { name: "Remover" })).toBeEnabled();
  });

  it("membro comum vê as contas, mas não tem ações de administração", async () => {
    render(<ProviderPage transport={fakeTransport({ providerConnections: vi.fn(async () => []), executors: vi.fn(async () => []) })} isAdmin={false} />);
    await screen.findByRole("list", { name: "Contas Claude Code" });
    expect(screen.queryByRole("button", { name: "Conectar conta" })).not.toBeInTheDocument();
    expect(screen.queryByRole("form", { name: "Adicionar conta de CLI" })).not.toBeInTheDocument();
  });
});

describe("One Provider AI: cadastro de conta e de API Key", () => {
  it("só cria a conta depois da autorização de uso e com o OneExecutor escolhido", async () => {
    const t = fakeTransport();
    render(<ProviderPage transport={t} isAdmin />);
    const form = await screen.findByRole("form", { name: "Adicionar conta de CLI" });
    const create = within(form).getByRole("button", { name: "Criar conta" });
    await userEvent.type(within(form).getByLabelText("Nome amigável"), "Claude Empresa 02");
    expect(create).toBeDisabled();
    await userEvent.click(within(form).getByRole("checkbox"));
    expect(create).toBeEnabled();
    await userEvent.click(create);
    expect(t.createCliConnection).toHaveBeenCalledWith({ name: "Claude Empresa 02", engine: "claude", executor_id: "e1" });
  });

  it("a chave de API sai do campo ao enviar e nunca reaparece na página", async () => {
    const KEY = "sk-or-v1-chave-de-teste-ABCD1234";
    const t = fakeTransport();
    const { container } = render(<ProviderPage transport={t} isAdmin />);
    const form = await screen.findByRole("form", { name: "Adicionar conexão por API" });
    await userEvent.type(within(form).getByLabelText("Nome da conexão"), "OpenRouter equipe");
    const keyInput = within(form).getByLabelText("Chave de API");
    expect(keyInput).toHaveAttribute("type", "password");
    await userEvent.type(keyInput, KEY);
    await userEvent.click(within(form).getByRole("button", { name: "Guardar chave" }));
    expect(t.createProviderConnection).toHaveBeenCalledWith(expect.objectContaining({ provider: "openrouter", api_key: KEY }));
    expect(keyInput).toHaveValue("");
    expect(container.innerHTML).not.toContain(KEY);
  });

  it("não mostra conexão por API como disponível sem o OneHub confirmar a chave", async () => {
    render(<ProviderPage transport={fakeTransport()} isAdmin />);
    const list = await screen.findByRole("list", { name: "Conexões por API" });
    expect(within(list).getByText("••••WXYZ")).toBeInTheDocument();
    expect(screen.getByText(/Teste de conexão e lista de modelos ainda dependem/)).toBeInTheDocument();
  });
});

describe("One Provider AI: mensagens", () => {
  it("traduz os códigos conhecidos e nunca mostra detalhe técnico desconhecido", () => {
    expect(errorMessage(new OneHubError(409, "executor_offline"))).toContain("offline");
    expect(errorMessage(new OneHubError(500, "internal"))).toBe("Não foi possível concluir a operação. Tente de novo.");
    expect(errorMessage(new OneHubError(401, "unauthorized"))).toBe("Sua sessão terminou. Entre de novo.");
  });
});
