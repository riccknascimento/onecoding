import { describe, expect, it } from "vitest";
import { addMessages, applyEvent, fromSnapshot } from "../central/chatState";
import type { ChatState } from "../central/chatState";
import type { CentralMessage, ConversationSnapshot, StreamEvent } from "../transport";

const msg = (over: Partial<CentralMessage>): CentralMessage => ({
  id: "m", role: "assistant", body: "", created_at: "2026-11-18T00:00:00Z", state: "completed", task_id: null, job_id: null, ...over,
});
const SNAP: ConversationSnapshot = {
  last_seq: 4,
  messages: [msg({ id: "a1", body: "...", state: "task", job_id: "j1", created_at: "2026-11-18T00:00:02Z" }), msg({ id: "u1", role: "user", body: "oi", created_at: "2026-11-18T00:00:01Z" })],
  events: [
    { seq: 2, kind: "job.status", payload: { job_id: "j1", status: "queued", cancel_requested: false, error: null } },
    { seq: 3, kind: "job.progress", payload: { job_id: "j1", seq: 0, kind: "started", payload: {} } },
    { seq: 4, kind: "task.status", payload: { task_id: "t1", status: "running", tool: "files_read", job_id: "j1" } },
  ],
  jobs: [{ id: "j1", status: "running", question: "oi", error: null, cancel_requested: false }],
  tasks: [{ id: "t1", status: "running", tool: "files_read", job_id: "j1" }],
};

const ev = (seq: number, kind: StreamEvent["kind"], payload: Record<string, unknown>): StreamEvent => ({ seq, kind, payload });

describe("estado do chat", () => {
  it("apresenta a pergunta antes da resposta gravada no mesmo instante", () => {
    const messages = [msg({ id: "a" }), msg({ id: "z", role: "user" })];
    expect(fromSnapshot({ ...SNAP, messages }).messages.map((m) => m.id)).toEqual(["z", "a"]);
  });

  it("snapshot ordena mensagens e monta a linha do tempo a partir da janela de eventos", () => {
    const s = fromSnapshot(SNAP);
    expect(s.messages.map((m) => m.id)).toEqual(["u1", "a1"]);
    expect(s.lastSeq).toBe(4);
    expect(s.jobs.j1!.timeline.map((t) => t.label)).toEqual([
      "Aguardando o OneExecutor",
      "OneExecutor iniciou a tarefa",
      "Ferramenta files_read: em execução no OneNode",
    ]);
    expect(s.jobs.j1!.status).toBe("running");
  });

  it("eventos já cobertos pelo snapshot são ignorados (reconexão sem duplicar)", () => {
    const s = fromSnapshot(SNAP);
    const again = applyEvent(s, ev(3, "job.progress", { job_id: "j1", seq: 0, kind: "started", payload: {} }));
    expect(again).toBe(s);
  });

  it("aplicar a mesma sequência duas vezes não muda o estado", () => {
    const next = applyEvent(fromSnapshot(SNAP), ev(5, "job.status", { job_id: "j1", status: "succeeded", cancel_requested: false, error: null }));
    expect(applyEvent(next, ev(5, "job.status", { job_id: "j1", status: "succeeded", cancel_requested: false, error: null }))).toBe(next);
    expect(next.jobs.j1!.timeline.filter((t) => t.label === "Resposta recebida")).toHaveLength(1);
  });

  it("mensagem repetida é atualizada, não duplicada; message.updated troca corpo e estado", () => {
    let s: ChatState = fromSnapshot(SNAP);
    s = applyEvent(s, ev(5, "message.created", { message: msg({ id: "a1", body: "...", state: "task", job_id: "j1", created_at: "2026-11-18T00:00:02Z" }) }));
    expect(s.messages.filter((m) => m.id === "a1")).toHaveLength(1);
    s = applyEvent(s, ev(6, "message.updated", { id: "a1", body: "Resposta final.", state: "completed" }));
    const a1 = s.messages.find((m) => m.id === "a1")!;
    expect(a1.body).toBe("Resposta final.");
    expect(a1.state).toBe("completed");
  });

  it("cancelamento só aparece como pedido até o executor confirmar", () => {
    let s = applyEvent(fromSnapshot(SNAP), ev(5, "job.status", { job_id: "j1", status: "running", cancel_requested: true, error: null }));
    expect(s.jobs.j1!.cancelRequested).toBe(true);
    expect(s.jobs.j1!.timeline.at(-1)!.label).toBe("Cancelamento pedido; aguardando confirmação do OneExecutor");
    s = applyEvent(s, ev(6, "job.status", { job_id: "j1", status: "cancelled", cancel_requested: true, error: "cancelado pelo usuário" }));
    expect(s.jobs.j1!.status).toBe("cancelled");
    expect(s.jobs.j1!.timeline.at(-1)!.label).toBe("Cancelado");
  });

  it("progresso do Codex: ferramenta e aviso entram; eventos desconhecidos não geram linha", () => {
    let s = fromSnapshot(SNAP);
    s = applyEvent(s, ev(5, "job.progress", { job_id: "j1", seq: 1, kind: "tool", payload: { summary: "cat README.md" } }));
    s = applyEvent(s, ev(6, "job.progress", { job_id: "j1", seq: 2, kind: "warning", payload: { message: "limite atingido" } }));
    s = applyEvent(s, ev(7, "job.progress", { job_id: "j1", seq: 3, kind: "desconhecido", payload: {} }));
    const labels = s.jobs.j1!.timeline.map((t) => t.label);
    expect(labels).toContain("Codex executou: cat README.md");
    expect(labels).toContain("Aviso do Codex: limite atingido");
    expect(labels.some((l) => l.includes("desconhecido"))).toBe(false);
  });

  it("ferramenta pedida pelo executor aparece na linha do job com o estado do OneNode", () => {
    let s = fromSnapshot(SNAP);
    s = applyEvent(s, ev(5, "task.status", { task_id: "t1", status: "succeeded", tool: "files_read", job_id: "j1" }));
    expect(s.tasks.t1!.status).toBe("succeeded");
    expect(s.jobs.j1!.timeline.at(-1)!.label).toBe("Ferramenta files_read: concluída pelo OneNode");
  });

  it("addMessages usa o mesmo critério de upsert", () => {
    const s = fromSnapshot(SNAP);
    const next = addMessages(s, [msg({ id: "u1", role: "user", body: "oi", created_at: "2026-11-18T00:00:01Z" }), msg({ id: "z9", body: "novo", created_at: "2026-11-18T00:00:09Z" })]);
    expect(next.messages.map((m) => m.id)).toEqual(["u1", "a1", "z9"]);
  });
});
