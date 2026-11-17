import { render, screen } from "@testing-library/react";
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
    expect(screen.getByRole("combobox", { name: "Gateway" })).toHaveValue("demo-apliflow");
    const ready: LocalGatewayState = { phase: "ready", gateway };
    rerender(<ChatPage local={ready} />);
    expect(screen.getByRole("combobox", { name: "Gateway" })).toHaveValue("local");
  });
});
