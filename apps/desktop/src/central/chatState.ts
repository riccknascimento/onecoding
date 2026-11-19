// Estado do chat no navegador. Função pura: o snapshot e os eventos do OneHub entram, a tela sai.
// Regras:
// - Mensagens e jobs são upserts por id: um evento repetido nunca duplica nada.
// - Eventos só são aplicados se a sequência for maior que a última aplicada (idempotente na reconexão).
// - Cada rótulo de progresso vem de um evento real; nada é inferido nem simulado.
import type { CentralJobStatus, CentralMessage, CentralTaskStatus, ConversationSnapshot, MessageState, StreamEvent } from "../transport";

export type Tone = "info" | "ok" | "warn" | "bad";

export interface TimelineItem {
  seq: number;
  tone: Tone;
  label: string;
}

export interface JobView {
  id: string;
  status: CentralJobStatus;
  question: string;
  error: string | null;
  cancelRequested: boolean;
  timeline: TimelineItem[];
}

export interface TaskView {
  id: string;
  status: CentralTaskStatus;
  tool: string | null;
  jobId: string | null;
}

export interface ChatState {
  /** Última sequência aplicada. */
  lastSeq: number;
  messages: CentralMessage[];
  jobs: Record<string, JobView>;
  tasks: Record<string, TaskView>;
}

const TASK_LABEL: Record<CentralTaskStatus, { text: string; tone: Tone }> = {
  queued: { text: "na fila do OneNode", tone: "info" },
  running: { text: "em execução no OneNode", tone: "info" },
  succeeded: { text: "concluída pelo OneNode", tone: "ok" },
  failed: { text: "falhou no OneNode", tone: "bad" },
  cancelled: { text: "cancelada", tone: "warn" },
};

function sortMessages(messages: CentralMessage[]): CentralMessage[] {
  return [...messages].sort((a, b) => {
    if (a.created_at !== b.created_at) return a.created_at < b.created_at ? -1 : 1;
    // Um turno pode gravar a pergunta e a resposta no mesmo instante.
    if (a.role === "user" && b.role !== "user") return -1;
    if (b.role === "user" && a.role !== "user") return 1;
    return a.id.localeCompare(b.id);
  });
}

function upsertMessage(messages: CentralMessage[], msg: CentralMessage): CentralMessage[] {
  const exists = messages.some((m) => m.id === msg.id);
  return sortMessages(exists ? messages.map((m) => (m.id === msg.id ? msg : m)) : [...messages, msg]);
}

function blankJob(id: string): JobView {
  return { id, status: "queued", question: "", error: null, cancelRequested: false, timeline: [] };
}

function jobStatusItem(status: CentralJobStatus, cancelRequested: boolean, error: string | null, seq: number): TimelineItem {
  switch (status) {
    case "queued":
      return { seq, tone: "info", label: "Aguardando o OneExecutor" };
    case "running":
      return cancelRequested
        ? { seq, tone: "warn", label: "Cancelamento pedido; aguardando confirmação do OneExecutor" }
        : { seq, tone: "info", label: "OneExecutor pegou o pedido" };
    case "succeeded":
      return { seq, tone: "ok", label: "Resposta recebida" };
    case "failed":
      return { seq, tone: "bad", label: error ? `Falhou: ${error}` : "Falhou" };
    case "cancelled":
      return { seq, tone: "warn", label: "Cancelado" };
  }
}

function progressItem(kind: string, payload: Record<string, unknown>, seq: number): TimelineItem | null {
  switch (kind) {
    case "started":
      return { seq, tone: "info", label: "OneExecutor iniciou a tarefa" };
    case "snapshot":
      return { seq, tone: "info", label: `Arquivos do projeto preparados: ${String(payload.arquivos ?? 0)}` };
    case "codex_started":
      return { seq, tone: "info", label: "Codex iniciado em modo somente leitura" };
    case "tool":
      return { seq, tone: "info", label: `Codex executou: ${String(payload.summary ?? "")}` };
    case "warning":
      return { seq, tone: "warn", label: `Aviso do Codex: ${String(payload.message ?? "")}` };
    default:
      return null;
  }
}

export function fromSnapshot(snap: ConversationSnapshot): ChatState {
  let state: ChatState = {
    lastSeq: 0,
    messages: sortMessages(snap.messages),
    jobs: {},
    tasks: {},
  };
  // A janela de eventos (sufixo do registro) monta a linha do tempo e o status dos jobs antigos.
  const window = [...snap.events].sort((a, b) => a.seq - b.seq);
  for (const e of window) {
    state = applyEvent(state, { seq: e.seq, kind: e.kind as StreamEvent["kind"], payload: e.payload });
  }
  // A linha atual do job é a fonte de verdade do status: a janela pode trazer um estado já superado.
  // Eventos depois da sequência do snapshot chegam pelo stream e corrigem qualquer diferença de instante.
  for (const j of snap.jobs) {
    const prev = state.jobs[j.id] ?? blankJob(j.id);
    state.jobs[j.id] = { ...prev, status: j.status, question: j.question, error: j.error ?? prev.error, cancelRequested: j.cancel_requested };
  }
  for (const t of snap.tasks) {
    state.tasks[t.id] = { id: t.id, status: t.status, tool: t.tool, jobId: t.job_id };
  }
  // a partir daqui o stream entrega só o que vier depois da sequência do snapshot
  return { ...state, lastSeq: snap.last_seq };
}

export function applyEvent(state: ChatState, ev: StreamEvent): ChatState {
  if (ev.seq <= state.lastSeq) return state;
  const next: ChatState = { ...state, lastSeq: ev.seq };
  const p = ev.payload;
  switch (ev.kind) {
    case "message.created":
      return { ...next, messages: upsertMessage(state.messages, p.message as CentralMessage) };
    case "message.updated": {
      const id = String(p.id);
      return {
        ...next,
        messages: state.messages.map((m) => (m.id === id ? { ...m, body: String(p.body ?? ""), state: p.state as MessageState } : m)),
      };
    }
    case "job.status": {
      const id = String(p.job_id);
      const prev = state.jobs[id] ?? blankJob(id);
      const status = p.status as CentralJobStatus;
      const cancelRequested = Boolean(p.cancel_requested);
      const error = typeof p.error === "string" ? p.error : prev.error;
      const item = jobStatusItem(status, cancelRequested, error, ev.seq);
      return { ...next, jobs: { ...state.jobs, [id]: { ...prev, status, cancelRequested, error, timeline: [...prev.timeline, item] } } };
    }
    case "job.progress": {
      const id = String(p.job_id);
      const item = progressItem(String(p.kind), (p.payload as Record<string, unknown>) ?? {}, ev.seq);
      if (!item) return next;
      const prev = state.jobs[id] ?? blankJob(id);
      return { ...next, jobs: { ...state.jobs, [id]: { ...prev, timeline: [...prev.timeline, item] } } };
    }
    case "task.status": {
      const id = String(p.task_id);
      const status = p.status as CentralTaskStatus;
      const tool = typeof p.tool === "string" ? p.tool : null;
      const jobId = typeof p.job_id === "string" ? p.job_id : null;
      const tasks = { ...state.tasks, [id]: { id, status, tool, jobId } };
      if (!jobId) return { ...next, tasks };
      const label = TASK_LABEL[status];
      const prev = state.jobs[jobId] ?? blankJob(jobId);
      const item: TimelineItem = { seq: ev.seq, tone: label.tone, label: `Ferramenta ${tool ?? ""}: ${label.text}` };
      return { ...next, tasks, jobs: { ...state.jobs, [jobId]: { ...prev, timeline: [...prev.timeline, item] } } };
    }
  }
  return next;
}

/** Acrescenta mensagens recebidas diretamente da resposta do turno (o stream as traria de qualquer forma). */
export function addMessages(state: ChatState, messages: CentralMessage[]): ChatState {
  return { ...state, messages: messages.reduce(upsertMessage, state.messages) };
}
