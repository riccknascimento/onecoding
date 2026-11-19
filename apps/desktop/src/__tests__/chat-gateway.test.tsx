import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { LocalGatewayState } from "../App";
import type { GatewayInfo } from "../contracts";
import { ChatPage } from "../pages/ChatPage";

const gateway: GatewayInfo = {
  id: "local", name: "Este Mac", kind: "local", status: "online",
  os: "macos-aarch64", protocol_version: 1, engines: [], demo: false,
};

describe("seletor de Gateway do Chat", () => {
  it("passa a usar o Gateway local quando a detecção termina depois da tela montar", () => {
    const { rerender } = render(<ChatPage local={{ phase: "loading" }} />);
    // Gateway de demonstração nunca é selecionável: enquanto o local não é detectado, nada fica escolhido.
    expect(screen.getByRole("combobox", { name: "OneNode" })).toHaveValue("");
    expect(screen.getByRole("option", { name: "Apliflow (demo) — desconectado" })).toBeDisabled();
    const ready: LocalGatewayState = { phase: "ready", gateway };
    rerender(<ChatPage local={ready} />);
    expect(screen.getByRole("combobox", { name: "OneNode" })).toHaveValue("local");
  });
});

describe("OneNode no Chat", () => {
  it("Modo Direto não escolhe OneNode sozinho: exige seleção explícita", async () => {
    const ready: LocalGatewayState = { phase: "ready", gateway };
    render(<ChatPage local={ready} />);
    const user = userEvent.setup();
    await user.click(screen.getByRole("radio", { name: "Coding" }));
    expect(screen.getByRole("combobox", { name: "OneNode" })).toHaveValue("");
    await user.selectOptions(screen.getByRole("combobox", { name: "OneNode" }), "local");
    expect(screen.getByRole("combobox", { name: "OneNode" })).toHaveValue("local");
  });

  it("Modo Orquestrador mostra 'Automático' desabilitado até existir roteamento real", () => {
    render(<ChatPage local={{ phase: "ready", gateway }} />);
    const auto = screen.getByRole("option", { name: "Automático · em breve" }) as HTMLOptionElement;
    expect(auto.disabled).toBe(true);
  });
});
