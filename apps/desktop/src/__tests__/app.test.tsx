import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { App } from "../App";

const SECTIONS = [
  "Chat / Orquestrador",
  "Agent Gateways",
  "Projetos",
  "OneContext",
  "Skills & Agents",
  "Execuções",
  "Configurações",
];

describe("OneAgent shell", () => {
  it("inicializa no Chat com as 7 seções de navegação", () => {
    render(<App />);
    const nav = screen.getByRole("navigation", { name: "Principal" });
    for (const label of SECTIONS) {
      expect(nav).toHaveTextContent(label);
    }
    expect(screen.getByRole("heading", { level: 1, name: "Chat / Orquestrador" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Chat \/ Orquestrador/ })).toHaveAttribute("aria-current", "page");
  });

  it("navega por todas as seções", async () => {
    const user = userEvent.setup();
    render(<App />);
    for (const label of SECTIONS) {
      await user.click(screen.getByRole("button", { name: new RegExp(label.replace(/[/&]/g, "\\$&")) }));
      expect(screen.getByRole("heading", { level: 1, name: label })).toBeInTheDocument();
    }
  });

  it("marca como DEMO as telas com dados de demonstração", async () => {
    const user = userEvent.setup();
    render(<App />);
    for (const label of ["Projetos", "Skills & Agents", "Execuções"]) {
      await user.click(screen.getByRole("button", { name: new RegExp(label.replace("&", "\\&")) }));
      expect(screen.getAllByText("DEMO").length).toBeGreaterThan(0);
    }
  });

  it("não permite enviar mensagem enquanto o orquestrador está desacoplado", () => {
    render(<App />);
    expect(screen.getByRole("textbox", { name: "Mensagem" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Enviar" })).toBeDisabled();
  });

  it("fora do app, informa que a detecção local não está disponível", async () => {
    const user = userEvent.setup();
    render(<App />);
    await user.click(screen.getByRole("button", { name: /Agent Gateways/ }));
    expect(screen.getByText(/Detecção local disponível apenas no app/)).toBeInTheDocument();
    expect(screen.getByText("Apliflow (demo)")).toBeInTheDocument();
  });
});
