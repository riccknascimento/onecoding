import { describe, expect, it } from "vitest";
import { codexReady, directBlockReason } from "../central/ChatContextBar";
import type { CentralExecutor, CentralGrant, CentralNode } from "../transport";

const node: CentralNode = {
  id: "n1",
  name: "VPS",
  status: "online",
  last_seen_at: null,
};
const grant: CentralGrant = {
  node_id: "n1",
  node_name: "VPS",
  project_id: "p1",
  project_name: "SystemFlow",
  root_path: "/opt/proj",
};

describe("prontidão do Codex no contexto do chat", () => {
  it("sem engines no payload antigo não bloqueia só por autenticação", () => {
    const executors: CentralExecutor[] = [{ id: "e1", name: "Prod", engine: "codex", status: "online" }];
    expect(codexReady(executors)).toBe("unknown");
    expect(
      directBlockReason({ node, grants: [grant], executorOnline: true, projectId: "p1", executors }),
    ).toBeNull();
  });

  it("Codex instalado sem autenticação bloqueia com mensagem objetiva", () => {
    const executors: CentralExecutor[] = [
      {
        id: "e1",
        name: "Prod",
        engine: "codex",
        status: "online",
        engines: [{ name: "codex", installed: true, authenticated: false, approved: true, version: "0.164.0" }],
      },
    ];
    expect(codexReady(executors)).toBe(false);
    expect(
      directBlockReason({ node, grants: [grant], executorOnline: true, projectId: "p1", executors }),
    ).toMatch(/não está autenticado/);
  });

  it("Codex autenticado e aprovado libera a análise direta", () => {
    const executors: CentralExecutor[] = [
      {
        id: "e1",
        name: "Prod",
        engine: "codex",
        status: "online",
        engines: [{ name: "codex", installed: true, authenticated: true, approved: true }],
      },
    ];
    expect(codexReady(executors)).toBe(true);
    expect(
      directBlockReason({ node, grants: [grant], executorOnline: true, projectId: "p1", executors }),
    ).toBeNull();
  });
});
