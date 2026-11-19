// Conversa: mensagens por papel e estado, execução real do OneExecutor e ferramentas do OneNode.
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import type { ReactNode } from "react";
import type { CentralJob, CentralMessage, CentralTaskDetail, CentralTaskStatus, OneHubTransport } from "../transport";
import { Icon } from "../icons";
import { Markdown } from "./Markdown";
import type { JobView, TaskView, Tone } from "./chatState";

const JOB_FINAL = ["succeeded", "failed", "cancelled"];
const JOB_LABEL: Record<string, string> = {
  queued: "Na fila",
  running: "Em execução",
  succeeded: "Concluída",
  failed: "Falhou",
  cancelled: "Cancelada",
};
const JOB_TONE: Record<string, Tone> = { queued: "info", running: "info", succeeded: "ok", failed: "bad", cancelled: "warn" };
const MESSAGE_STATE_LABEL: Record<string, { text: string; tone: Tone }> = {
  unavailable: { text: "Indisponível", tone: "warn" },
  rejected: { text: "Recusado", tone: "warn" },
  failed: { text: "Falhou", tone: "bad" },
  cancelled: { text: "Cancelado", tone: "warn" },
};
const TASK_FINAL: CentralTaskStatus[] = ["succeeded", "failed", "cancelled"];

/** Lista de mensagens com rolagem inteligente: só acompanha o fim se o usuário já estiver nele. */
export function ThreadView({ children, tick }: { children: ReactNode; tick: unknown }) {
  const ref = useRef<HTMLDivElement>(null);
  const [stuck, setStuck] = useState(true);
  const [unseen, setUnseen] = useState(false);
  const onScroll = () => {
    const el = ref.current;
    if (!el) return;
    const near = el.scrollHeight - el.scrollTop - el.clientHeight < 96;
    setStuck(near);
    if (near) setUnseen(false);
  };
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    if (stuck) el.scrollTop = el.scrollHeight;
    else setUnseen(true);
    // só reage a mudanças de conteúdo; a posição do leitor é preservada
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tick]);
  const jumpToEnd = () => {
    const el = ref.current;
    if (!el) return;
    el.scrollTop = el.scrollHeight;
    setStuck(true);
    setUnseen(false);
  };
  return (
    <div className="thread-scroll" ref={ref} onScroll={onScroll} aria-label="Conversa">
      <div className="thread">{children}</div>
      {unseen && (
        <button type="button" className="jump-end btn primary" onClick={jumpToEnd}>
          Novas atualizações ↓
        </button>
      )}
    </div>
  );
}

function ExecutionPanel({
  job,
  transport,
  cancelling,
  onCancel,
}: {
  job: JobView;
  transport: OneHubTransport;
  cancelling: boolean;
  onCancel: (id: string) => void;
}) {
  const final = JOB_FINAL.includes(job.status);
  const [evidence, setEvidence] = useState<string[] | null>(null);
  useEffect(() => {
    if (!final || job.status !== "succeeded") return;
    let alive = true;
    transport
      .job(job.id)
      .then((d: CentralJob) => alive && setEvidence(d.evidence))
      .catch(() => alive && setEvidence([]));
    return () => {
      alive = false;
    };
  }, [transport, job.id, job.status, final]);
  const label = job.cancelRequested && !final ? "Cancelamento solicitado" : JOB_LABEL[job.status];
  const tone = job.cancelRequested && !final ? "warn" : JOB_TONE[job.status];
  return (
    <section className="exec" aria-label="Execução do OneExecutor">
      <header className="run-head">
        <span className={`status-pill ${tone}`}>
          <span className="dot" aria-hidden="true" />
          {label}
        </span>
        {!final && !job.cancelRequested && (
          <button type="button" className="btn" onClick={() => onCancel(job.id)} disabled={cancelling}>
            <Icon name="stop" /> {cancelling ? "Enviando…" : "Cancelar"}
          </button>
        )}
      </header>
      {job.timeline.length > 0 && (
        <ol className="timeline">
          {job.timeline.map((item) => (
            <li key={item.seq} className={`ev ${item.tone}`}>
              {item.label}
            </li>
          ))}
        </ol>
      )}
      {job.status === "succeeded" && evidence && evidence.length > 0 && (
        <p className="muted small">Arquivos usados: {evidence.join(", ")}</p>
      )}
      {job.status === "failed" && job.error && <p className="error small" role="alert">{job.error}</p>}
    </section>
  );
}

function TaskPanel({ task, transport }: { task: TaskView; transport: OneHubTransport }) {
  const [detail, setDetail] = useState<CentralTaskDetail | null>(null);
  const final = TASK_FINAL.includes(task.status);
  useEffect(() => {
    if (!final) return;
    let alive = true;
    transport
      .task(task.id)
      .then((d) => alive && setDetail(d))
      .catch(() => undefined);
    return () => {
      alive = false;
    };
  }, [transport, task.id, final]);
  const res = (detail?.result ?? {}) as Record<string, unknown>;
  return (
    <section className="exec" aria-label="Ferramenta do OneNode">
      <header className="run-head">
        <span className={`status-pill ${task.status === "failed" ? "bad" : task.status === "succeeded" ? "ok" : "info"}`}>
          <span className="dot" aria-hidden="true" />
          Ferramenta {task.tool ?? ""}: {task.status === "queued" ? "na fila" : task.status === "running" ? "em execução" : task.status === "succeeded" ? "concluída" : task.status === "failed" ? "falhou" : "cancelada"}
        </span>
      </header>
      {typeof res.content === "string" && <pre className="task-result">{res.content}</pre>}
      {Array.isArray(res.entries) && (
        <ul className="task-result">
          {(res.entries as { name: string; kind: string }[]).map((e) => (
            <li key={e.name}>
              {e.name} <span className="muted">({e.kind})</span>
            </li>
          ))}
        </ul>
      )}
      {task.status === "failed" && typeof res.error === "string" && <p className="error small" role="alert">{res.error}</p>}
    </section>
  );
}

export function MessageView({
  message,
  jobs,
  tasks,
  transport,
  cancelling,
  onCancel,
}: {
  message: CentralMessage;
  jobs: Record<string, JobView>;
  tasks: Record<string, TaskView>;
  transport: OneHubTransport;
  cancelling: Set<string>;
  onCancel: (id: string) => void;
}) {
  const job = message.job_id ? jobs[message.job_id] : undefined;
  const task = message.task_id ? tasks[message.task_id] : undefined;
  const who = message.role === "user" ? "Você" : message.job_id ? "OneExecutor (Codex)" : "OneOrchestrator";
  const stateNote = message.role === "assistant" ? MESSAGE_STATE_LABEL[message.state] : undefined;
  return (
    <article className={`msg ${message.role}`} data-state={message.state} data-message-id={message.id}>
      <div className="msg-role">
        {who}
        {stateNote && <span className={`msg-state ${stateNote.tone}`}> · {stateNote.text}</span>}
      </div>
      {message.role === "user" ? <p className="user-text">{message.body}</p> : message.body && <Markdown text={message.body} />}
      {job && <ExecutionPanel job={job} transport={transport} cancelling={cancelling.has(job.id)} onCancel={onCancel} />}
      {!job && message.job_id && <p className="muted small">Pedido enviado ao OneExecutor.</p>}
      {task && <TaskPanel task={task} transport={transport} />}
    </article>
  );
}
