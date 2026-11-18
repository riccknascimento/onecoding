import type { ChatConversation, RunView } from "../contracts";
import { EMPTY_SELECTION, fromConversation, fromRun, mergeContext, NO_CONTEXT, toContext } from "../chat/model";

const conv = (context: ChatConversation["context"], model: string | null = null): ChatConversation => ({
  id: "cv1", title: "t", provider: "chatgpt_plan", model, state: "idle", updatedAt: "2026-11-17T12:00:00Z", context,
});

describe("contexto por conversa", () => {
  it("conversa antiga, sem contexto salvo, herda a barra atual e só o modelo vem da conversa", () => {
    const base = { ...EMPTY_SELECTION, gatewayId: "local", workspaceId: "ws1", engine: "codex" as const, chatModel: "gpt-b" };
    expect(fromConversation(conv(NO_CONTEXT, "gpt-a"), base)).toEqual({ ...base, chatModel: "gpt-a" });
  });

  it("conversa com contexto restaura os cinco campos, e modelo do executor vazio significa padrão da CLI", () => {
    const c = conv({ gatewayId: "local", workspaceId: "ws9", executorEngine: "auto", executorModel: null, orchestratorModel: "gpt-x" }, "gpt-reportado");
    expect(fromConversation(c, { ...EMPTY_SELECTION, executorModel: "lixo" })).toEqual({
      gatewayId: "local", workspaceId: "ws9", engine: "auto", executorModel: "", chatModel: "gpt-x",
    });
  });

  it("execução do Modo Direto restaura Gateway, workspace, motor e modelo solicitado", () => {
    const r = { gateway_id: "local", workspace_id: "ws2", engine: "codex", model_requested: "m1" } as RunView;
    expect(fromRun(r, { ...EMPTY_SELECTION, chatModel: "gpt-a" })).toEqual({
      gatewayId: "local", workspaceId: "ws2", engine: "codex", executorModel: "m1", chatModel: "gpt-a",
    });
  });

  it("valor ainda desconhecido nunca apaga o que a conversa já tinha salvo", () => {
    const stored = { gatewayId: "local", workspaceId: "ws1", executorEngine: "codex" as const, executorModel: null, orchestratorModel: "gpt-a" };
    const unknown = { ...EMPTY_SELECTION, engine: "auto" as const };
    expect(mergeContext(unknown, stored)).toEqual({ ...stored, executorEngine: "auto" });
    expect(toContext({ ...EMPTY_SELECTION, executorModel: "m" }).executorModel).toBe("m");
  });
});
