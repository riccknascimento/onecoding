import { useEffect, useState } from "react";
import { inTauri, listRuns } from "../api";
import type { RunStatus, RunView } from "../contracts";
import { DEMO_RUNS, DEMO_TASKS } from "../demo/data";
import { Badge, DemoNotice, PageHeader } from "./common";
import { formatDuration } from "./RunTimeline";

const STATUS: Record<RunStatus, { tone: "ok" | "warn" | "bad" | "muted"; text: string }> = {
  queued: { tone: "muted", text: "Na fila" },
  running: { tone: "warn", text: "Executando" },
  awaiting_approval: { tone: "warn", text: "Aguardando aprovação" },
  succeeded: { tone: "ok", text: "Concluída" },
  failed: { tone: "bad", text: "Falhou" },
  cancelled: { tone: "muted", text: "Cancelada" },
};

function RealRuns() {
  const [runs, setRuns] = useState<RunView[] | null>(null);
  useEffect(() => {
    listRuns(100).then(setRuns).catch(() => setRuns([]));
  }, []);
  if (runs === null) return <p className="muted">Carregando…</p>;
  if (runs.length === 0) return <p className="muted">Nenhuma execução ainda. Envie uma tarefa no Chat.</p>;
  return (
    <table className="table">
      <thead>
        <tr><th>Tarefa</th><th>Motor</th><th>Workspace</th><th>Status</th><th>Duração</th><th>Tokens (ent./saída)</th></tr>
      </thead>
      <tbody>
        {runs.map((r) => {
          const s = STATUS[r.status];
          const n = (v: number | null) => (v === null ? "—" : v.toLocaleString("pt-BR"));
          return (
            <tr key={r.run_id}>
              <td>{r.prompt}</td>
              <td>{r.engine}</td>
              <td>{r.workspace_label}</td>
              <td><Badge tone={s.tone}>{s.text}</Badge></td>
              <td>{formatDuration(r.duration_ms)}</td>
              <td>{n(r.usage.input_tokens)} / {n(r.usage.output_tokens)}</td>
            </tr>
          );
        })}
      </tbody>
    </table>
  );
}

function DemoRuns() {
  return (
    <>
      <DemoNotice />
      <table className="table">
        <thead>
          <tr><th>Tarefa</th><th>Motor</th><th>Modo</th><th>Status</th><th>Evidências</th></tr>
        </thead>
        <tbody>
          {DEMO_RUNS.map((r) => {
            const t = DEMO_TASKS.find((x) => x.id === r.task_id);
            const s = STATUS[r.status];
            return (
              <tr key={r.id}>
                <td>{t?.prompt ?? r.task_id}</td>
                <td>{t?.engine ?? "—"}</td>
                <td>{t?.mode === "write" ? "Escrita" : "Somente leitura"}</td>
                <td><Badge tone={s.tone}>{s.text}</Badge></td>
                <td>{r.evidence_refs.length ? <code>{r.evidence_refs[0]}</code> : "—"}</td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </>
  );
}

export function RunsPage() {
  return (
    <section>
      <PageHeader title="Execuções" subtitle="Histórico de tarefas delegadas. Logs completos ficam no Gateway; aqui só resumo, eventos e métricas." />
      {inTauri() ? <RealRuns /> : <DemoRuns />}
    </section>
  );
}
