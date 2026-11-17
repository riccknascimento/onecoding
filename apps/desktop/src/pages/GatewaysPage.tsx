import { useState } from "react";
import type { LocalGatewayState } from "../App";
import type { AuthState, EngineKind, EngineStatus, GatewayInfo } from "../contracts";
import { DEMO_GATEWAYS } from "../demo/data";
import { Badge, DemoBadge, PageHeader } from "./common";

const ENGINE_NAME = { codex: "Codex CLI", claude: "Claude Code", cursor: "Cursor CLI" } as const;
const AUTH_LABEL: Record<AuthState, { tone: "ok" | "bad" | "muted"; text: string }> = {
  logged_in: { tone: "ok", text: "Logado" },
  logged_out: { tone: "bad", text: "Sem login" },
  unknown: { tone: "muted", text: "Login desconhecido" },
};

type OnApprove = (engine: EngineKind, fingerprint: string) => Promise<void>;

function TrustControls({ e, onApprove }: { e: EngineStatus; onApprove?: OnApprove }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  if (!e.installed || e.trust === "trusted") return null;
  if (e.trust === "rejected") return <Badge tone="bad">Reprovado</Badge>;
  const canApprove = onApprove !== undefined && e.fingerprint !== null;
  return (
    <>
      <Badge tone="warn">{e.trust === "changed" ? "Executável alterado" : "Não aprovado"}</Badge>
      {canApprove && (
        <button
          type="button"
          className="btn"
          disabled={busy}
          onClick={() => {
            setBusy(true);
            setError(null);
            onApprove(e.engine, e.fingerprint as string)
              .catch((err: unknown) => setError(String(err instanceof Error ? err.message : err)))
              .finally(() => setBusy(false));
          }}
        >
          {e.trust === "changed" ? "Validar de novo…" : "Revisar e aprovar…"}
        </button>
      )}
      {error && <span className="muted small">{error}</span>}
    </>
  );
}

function EngineRow({ e, onApprove }: { e: EngineStatus; onApprove?: OnApprove }) {
  const auth = AUTH_LABEL[e.auth];
  return (
    <li className="engine">
      <div className="engine-head">
        <strong>{ENGINE_NAME[e.engine]}</strong>
        {e.installed ? <Badge tone="ok">Instalada {e.version ?? ""}</Badge> : <Badge tone="bad">Não encontrada</Badge>}
        {e.trust === "trusted" && <Badge tone={auth.tone}>{auth.text}</Badge>}
        <TrustControls e={e} onApprove={onApprove} />
      </div>
      {e.path && <code className="path">{e.path}</code>}
      {e.models.length > 0 && (
        <details>
          <summary>{e.models.length} modelos{e.models.every((m) => !m.confirmed) ? " (apelidos não confirmados)" : ""}</summary>
          <ul className="chips">
            {e.models.map((m) => (
              <li key={m.id} className={m.confirmed ? "" : "unconfirmed"}>{m.id}</li>
            ))}
          </ul>
        </details>
      )}
      {e.note && <p className="muted small">{e.note}</p>}
    </li>
  );
}

function GatewayCard({ g, onApprove }: { g: GatewayInfo; onApprove?: OnApprove }) {
  return (
    <article className="card">
      <div className="card-head">
        <h2>{g.name}</h2>
        {g.demo && <DemoBadge />}
        <Badge tone={g.status === "online" ? "ok" : "muted"}>{g.status === "online" ? "Online" : "Status desconhecido"}</Badge>
        <span className="muted small">{g.kind === "local" ? "Local" : "Remoto"} · {g.os} · protocolo v{g.protocol_version}</span>
      </div>
      {g.engines.length > 0 ? (
        <ul className="engines">{g.engines.map((e) => <EngineRow key={e.engine} e={e} onApprove={onApprove} />)}</ul>
      ) : (
        <p className="muted small">Sem detalhes de motores (Gateway remoto não conectado nesta fase).</p>
      )}
    </article>
  );
}

export function GatewaysPage({ local, onApprove }: { local: LocalGatewayState; onApprove?: OnApprove }) {
  return (
    <section>
      <PageHeader title="Agent Gateways" subtitle="Cada Gateway tem ambiente, credenciais e projetos próprios." />
      {local.phase === "loading" && <p className="muted">Detectando CLIs deste Mac…</p>}
      {local.phase === "unavailable" && (
        <p className="notice">Detecção local disponível apenas no app (não no navegador/testes).</p>
      )}
      {local.phase === "error" && <p className="notice bad">Falha na detecção: {local.message}</p>}
      {local.phase === "ready" && <GatewayCard g={local.gateway} onApprove={onApprove} />}
      {DEMO_GATEWAYS.map((g) => <GatewayCard key={g.id} g={g} />)}
    </section>
  );
}
