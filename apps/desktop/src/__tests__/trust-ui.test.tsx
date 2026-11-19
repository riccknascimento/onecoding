import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { LocalGatewayState } from "../App";
import type { EngineStatus, GatewayInfo } from "../contracts";
import { GatewaysPage } from "../pages/GatewaysPage";

const engine = (over: Partial<EngineStatus>): EngineStatus => ({
  engine: "codex",
  installed: true,
  path: "/home/u/.nvm/bin/codex",
  version: null,
  auth: "unknown",
  trust: "unknown",
  fingerprint: "abc123",
  models: [],
  note: "executável ainda não aprovado",
  execution_block: null,
  ...over,
});

const ready = (e: EngineStatus): LocalGatewayState => {
  const gateway: GatewayInfo = {
    id: "local", name: "Este Mac", kind: "local", status: "online",
    os: "macos-aarch64", protocol_version: 1, engines: [e], demo: false,
  };
  return { phase: "ready", gateway };
};

describe("aprovação de executável na UI", () => {
  it("executável desconhecido mostra o aviso e pede aprovação com a impressão digital vista", async () => {
    const onApprove = vi.fn().mockResolvedValue(undefined);
    render(<GatewaysPage local={ready(engine({}))} onApprove={onApprove} />);
    expect(screen.getByText("Não aprovado")).toBeInTheDocument();
    expect(screen.queryByText(/Logado|Sem login/)).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: /Revisar e aprovar/ }));
    expect(onApprove).toHaveBeenCalledWith("codex", "abc123");
  });

  it("executável alterado exige validar de novo", () => {
    render(<GatewaysPage local={ready(engine({ trust: "changed" }))} onApprove={vi.fn()} />);
    expect(screen.getByText("Executável alterado")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Validar de novo/ })).toBeInTheDocument();
  });

  it("reprovado não oferece aprovação", () => {
    render(<GatewaysPage local={ready(engine({ trust: "rejected", fingerprint: null }))} onApprove={vi.fn()} />);
    expect(screen.getByText("Reprovado")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /aprovar|Validar/i })).not.toBeInTheDocument();
  });

  it("mostra o erro quando o usuário cancela o diálogo nativo", async () => {
    const onApprove = vi.fn().mockRejectedValue(new Error("aprovação cancelada"));
    render(<GatewaysPage local={ready(engine({}))} onApprove={onApprove} />);
    await userEvent.click(screen.getByRole("button", { name: /Revisar e aprovar/ }));
    expect(await screen.findByText("aprovação cancelada")).toBeInTheDocument();
  });

  it("confiável não mostra botão e exibe o login", () => {
    render(<GatewaysPage local={ready(engine({ trust: "trusted", auth: "logged_in", version: "1.0.0" }))} onApprove={vi.fn()} />);
    expect(screen.getByText("Logado")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /aprovar/i })).not.toBeInTheDocument();
  });
});
