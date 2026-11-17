import type { RunStatus } from "../contracts";
import { DEMO_RUNS, DEMO_TASKS } from "../demo/data";
import { Badge, DemoNotice, PageHeader } from "./common";

const STATUS: Record<RunStatus, { tone: "ok" | "warn" | "bad" | "muted"; text: string }> = {
  queued: { tone: "muted", text: "Na fila" },
  running: { tone: "warn", text: "Executando" },
  awaiting_approval: { tone: "warn", text: "Aguardando aprovação" },
  succeeded: { tone: "ok", text: "Concluída" },
  failed: { tone: "bad", text: "Falhou" },
  cancelled: { tone: "muted", text: "Cancelada" },
};

export function RunsPage() {
  return (
    <section>
      <PageHeader title="Execuções" subtitle="Histórico de tarefas delegadas. Logs completos ficam no Gateway; aqui só resumo e referências." />
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
    </section>
  );
}
