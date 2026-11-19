// Páginas do OneCoding Web. Só mostram dados reais do OneHub. Módulos sem backend dizem que estão indisponíveis.
import { useEffect, useState } from "react";
import type { ReactNode } from "react";
import { OneHubError } from "../transport";
import type { CentralGrant, CentralTaskDetail, CentralTaskSummary, OneHubTransport } from "../transport";
import { Icon } from "../icons";
import type { IconName } from "../icons";

export function PageHeader({ title, subtitle, actions }: { title: string; subtitle?: string; actions?: ReactNode }) {
  return (
    <header className="page-head">
      <div>
        <h1 className="page-title">{title}</h1>
        {subtitle && <p className="muted small page-sub">{subtitle}</p>}
      </div>
      {actions && <div className="page-actions">{actions}</div>}
    </header>
  );
}

/** Módulo sem backend: explica a indisponibilidade real, sem botões nem dados fictícios. */
export function UnavailablePage({ title, icon, reason, next }: { title: string; icon: IconName; reason: string; next: string }) {
  return (
    <section className="unavailable" aria-label={`${title} indisponível`}>
      <div className="unavailable-icon"><Icon name={icon} /></div>
      <h1 className="page-title">{title}</h1>
      <p className="pill-warn">Indisponível neste Central</p>
      <p className="muted">{reason}</p>
      <p className="muted small">{next}</p>
    </section>
  );
}

function errorText(e: unknown): string {
  if (e instanceof OneHubError && e.code === "unauthorized") return "Sua sessão terminou. Entre de novo.";
  return "Não foi possível carregar os dados do OneHub. Verifique a conexão.";
}

export function ProjectsPage({ transport, isAdmin, onOpenOneNodes }: { transport: OneHubTransport; isAdmin: boolean; onOpenOneNodes(): void }) {
  const [projects, setProjects] = useState<{ id: string; name: string }[] | null>(null);
  const [grants, setGrants] = useState<CentralGrant[]>([]);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    Promise.all([transport.projects(), transport.grants()])
      .then(([p, g]) => {
        if (!alive) return;
        setProjects(p);
        setGrants(g);
      })
      .catch((e) => alive && setError(errorText(e)));
    return () => {
      alive = false;
    };
  }, [transport]);

  return (
    <div className="page">
      <PageHeader
        title="Projetos"
        subtitle="Projetos cadastrados no OneHub e os OneNodes que podem analisá-los, com a pasta autorizada em cada servidor."
        actions={isAdmin ? <button type="button" className="btn" onClick={onOpenOneNodes}>Gerenciar em OneNodes</button> : undefined}
      />
      {error && <p role="alert" className="error">{error}</p>}
      {projects === null && !error && <p className="muted">Carregando projetos…</p>}
      {projects && projects.length === 0 && (
        <div className="empty-card">
          <p>Nenhum projeto cadastrado ainda.</p>
          <p className="muted small">Projetos são criados ao conceder acesso a um OneNode, em OneNodes.</p>
        </div>
      )}
      {projects && projects.length > 0 && (
        <ul className="card-list" aria-label="Lista de projetos">
          {projects.map((p) => {
            const onNodes = grants.filter((g) => g.project_id === p.id);
            return (
              <li key={p.id} className="card project-card">
                <div className="card-head">
                  <strong>{p.name}</strong>
                  <span className={`status-pill ${onNodes.length ? "ok" : ""}`}>
                    {onNodes.length ? `${onNodes.length} OneNode(s)` : "sem OneNode"}
                  </span>
                </div>
                {onNodes.length === 0 ? (
                  <p className="muted small">Nenhum OneNode com acesso a este projeto.</p>
                ) : (
                  <ul className="kv-list">
                    {onNodes.map((g) => (
                      <li key={g.node_id}>
                        <span>{g.node_name}</span>
                        <code title="Pasta autorizada neste servidor">{g.root_path ?? "sem pasta"}</code>
                      </li>
                    ))}
                  </ul>
                )}
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}

const TASK_LABEL: Record<string, { text: string; tone: string }> = {
  queued: { text: "na fila", tone: "info" },
  running: { text: "em execução", tone: "info" },
  succeeded: { text: "concluída", tone: "ok" },
  failed: { text: "falhou", tone: "bad" },
  cancelled: { text: "cancelada", tone: "warn" },
};

export function ExecutionsPage({ transport }: { transport: OneHubTransport }) {
  const [tasks, setTasks] = useState<CentralTaskSummary[] | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [detail, setDetail] = useState<CentralTaskDetail | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    transport
      .tasks()
      .then((t) => alive && setTasks(t))
      .catch((e) => alive && setError(errorText(e)));
    return () => {
      alive = false;
    };
  }, [transport]);

  useEffect(() => {
    if (!selected) {
      setDetail(null);
      return;
    }
    let alive = true;
    transport
      .task(selected)
      .then((d) => alive && setDetail(d))
      .catch((e) => alive && setError(errorText(e)));
    return () => {
      alive = false;
    };
  }, [transport, selected]);

  return (
    <div className="page">
      <PageHeader title="Execuções" subtitle="Tarefas enviadas aos OneNodes a partir das suas conversas, com o resultado real de cada uma." />
      {error && <p role="alert" className="error">{error}</p>}
      {tasks === null && !error && <p className="muted">Carregando execuções…</p>}
      {tasks && tasks.length === 0 && (
        <div className="empty-card">
          <p>Nenhuma execução ainda.</p>
          <p className="muted small">As execuções aparecem quando uma análise é enviada a um OneNode com projeto autorizado.</p>
        </div>
      )}
      {tasks && tasks.length > 0 && (
        <div className="exec-layout">
          <table className="exec-table" aria-label="Lista de execuções">
            <thead>
              <tr><th>Ferramenta</th><th>OneNode</th><th>Projeto</th><th>Estado</th><th>Atualizada</th></tr>
            </thead>
            <tbody>
              {tasks.map((t) => {
                const st = TASK_LABEL[t.status] ?? { text: t.status, tone: "" };
                return (
                  <tr
                    key={t.id}
                    className={selected === t.id ? "selected" : undefined}
                    onClick={() => setSelected(t.id)}
                  >
                    <td><button type="button" className="link-btn" onClick={() => setSelected(t.id)}>{t.tool ?? "—"}</button></td>
                    <td>{t.node_name}</td>
                    <td>{t.project_name}</td>
                    <td><span className={`status-pill ${st.tone}`}>{st.text}</span></td>
                    <td className="muted small">{new Date(t.updated_at).toLocaleString("pt-BR")}</td>
                  </tr>
                );
              })}
            </tbody>
          </table>
          {detail && (
            <aside className="exec-detail card" aria-label="Detalhes da execução">
              <h2>{detail.tool ?? "Execução"}</h2>
              <p className="muted small">Estado: {TASK_LABEL[detail.status]?.text ?? detail.status}</p>
              {detail.events.length > 0 && (
                <ol className="timeline">
                  {detail.events.map((e) => (
                    <li key={e.seq}><span className="muted small">{e.kind}</span></li>
                  ))}
                </ol>
              )}
              {detail.result && <pre className="task-result">{JSON.stringify(detail.result, null, 2)}</pre>}
            </aside>
          )}
        </div>
      )}
    </div>
  );
}
