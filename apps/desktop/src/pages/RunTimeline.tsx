import type { RunEvent, RunView } from "../contracts";
import { Badge } from "./common";

export function formatDuration(ms: number | null): string {
  if (ms === null) return "—";
  return ms < 1000 ? `${ms} ms` : `${(ms / 1000).toFixed(1)} s`;
}

const STATUS = {
  queued: { tone: "muted", text: "Na fila" },
  running: { tone: "warn", text: "Executando" },
  awaiting_approval: { tone: "warn", text: "Aguardando aprovação" },
  succeeded: { tone: "ok", text: "Concluída" },
  failed: { tone: "bad", text: "Falhou" },
  cancelled: { tone: "muted", text: "Cancelada" },
} as const;

export function StatusBadge({ status }: { status: RunView["status"] }) {
  const s = STATUS[status];
  return <Badge tone={s.tone}>{s.text}</Badge>;
}

/** Métricas OneUsage da execução: só o que o executor informou ("—" quando ausente). */
export function RunMetrics({ run }: { run: RunView }) {
  const n = (v: number | null) => (v === null ? "—" : v.toLocaleString("pt-BR"));
  return (
    <p className="muted small metrics" aria-label="Métricas da execução">
      {run.engine} · modelo {run.model_reported ?? run.model_requested ?? "padrão"}
      {run.model_reported === null && " (não informado pelo motor)"} · {formatDuration(run.duration_ms)} · ferramentas{" "}
      {run.tool_calls} · tokens entrada {n(run.usage.input_tokens)} / saída {n(run.usage.output_tokens)} / cache{" "}
      {n(run.usage.cached_tokens)} · tentativas {run.attempts}
    </p>
  );
}

export function RunTimeline({ events }: { events: RunEvent[] }) {
  return (
    <ol className="timeline" aria-label="Eventos da execução">
      {events.map((e) => {
        const p = e.payload;
        switch (p.type) {
          case "message":
            return (
              <li key={e.seq} className="msg orchestrator">
                <div className="msg-role">Codex</div>
                <p>{p.text}</p>
              </li>
            );
          case "tool_started":
            return (
              <li key={e.seq} className="ev tool">
                <span className="muted">▸ {p.name}</span> <code>{p.summary}</code>
              </li>
            );
          case "tool_finished":
            return (
              <li key={e.seq} className="ev tool">
                <span className={p.ok ? "ok" : "bad"}>{p.ok ? "✓" : "✗"}</span> {p.name}
                {p.exit_code !== null && <span className="muted"> (código {p.exit_code})</span>}
              </li>
            );
          case "warning":
            return (
              <li key={e.seq} className="ev warn">
                Aviso: {p.message}
              </li>
            );
          case "error":
            return (
              <li key={e.seq} className="ev bad" role="alert">
                Erro: {p.message}
              </li>
            );
          case "status":
          case "usage":
            return null; // aparecem no cabeçalho e nas métricas
        }
      })}
    </ol>
  );
}
