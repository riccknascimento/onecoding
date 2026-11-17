import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { authorizeWorkspace, cancelRun, getOrchestratorStatus, getRun, listRuns, listWorkspaces, startRun } from "../api";
import type { LocalGatewayState } from "../App";
import type { ChatMode, EngineKind, ProviderStatus, RunEvent, RunView, Workspace } from "../contracts";
import { DEMO_GATEWAYS } from "../demo/data";
import { PageHeader } from "./common";
import { RunMetrics, RunTimeline, StatusBadge } from "./RunTimeline";

const ENGINE_LABEL: Record<EngineKind, string> = { codex: "Codex CLI", claude: "Claude Code", cursor: "Cursor CLI" };
const POLL_MS = 400;
const FINAL = new Set(["succeeded", "failed", "cancelled"]);

function errText(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
}

export function ChatPage({ local }: { local: LocalGatewayState }) {
  const [mode, setMode] = useState<ChatMode>("direct");
  const [providerStatus, setProviderStatus] = useState<ProviderStatus | null>(null);
  useEffect(() => {
    getOrchestratorStatus()
      .then(setProviderStatus)
      .catch((e: unknown) => setProviderStatus({ state: "error", message: errText(e) }));
  }, []);

  const gateways = useMemo(
    () => [...(local.phase === "ready" ? [local.gateway] : []), ...DEMO_GATEWAYS],
    [local],
  );
  // Escolha explícita do usuário; sem escolha, vale o primeiro da lista (o local, assim que detectado).
  const [chosenGateway, setChosenGateway] = useState<string | null>(null);
  const gatewayId = chosenGateway ?? gateways[0]?.id ?? "";
  const [engine, setEngine] = useState<EngineKind>("codex");
  const [model, setModel] = useState("");
  const [draft, setDraft] = useState("");

  const gateway = gateways.find((g) => g.id === gatewayId);
  const engineStatus = gateway?.engines.find((e) => e.engine === engine);
  const models = engineStatus?.models ?? [];

  // Execução real só com Gateway local, Codex instalado e aprovado.
  const realCapable = gateway?.id === "local" && engine === "codex" && engineStatus?.trust === "trusted";
  const whyNot = !gateway || gateway.demo
    ? "Selecione “Este Mac” para executar de verdade."
    : engine !== "codex"
      ? "Nesta fase só o Codex CLI executa de verdade."
      : engineStatus?.trust !== "trusted"
        ? "Aprove o Codex CLI em Agent Gateways para habilitar a execução."
        : null;

  const [workspaces, setWorkspaces] = useState<Workspace[]>([]);
  const [workspaceId, setWorkspaceId] = useState("");
  const [runs, setRuns] = useState<RunView[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [run, setRun] = useState<RunView | null>(null);
  const [events, setEvents] = useState<RunEvent[]>([]);
  const [error, setError] = useState<string | null>(null);
  const lastSeq = useRef(0);

  const refreshRuns = useCallback(() => {
    listRuns(50).then(setRuns).catch(() => undefined);
  }, []);

  useEffect(() => {
    if (gateway?.id !== "local") return;
    listWorkspaces()
      .then((w) => {
        setWorkspaces(w);
        setWorkspaceId((cur) => cur || w[0]?.id || "");
      })
      .catch((e: unknown) => setError(errText(e)));
  }, [gateway?.id]);

  useEffect(() => {
    if (!realCapable) return;
    refreshRuns();
  }, [realCapable, refreshRuns]);

  // Consulta o estado da execução selecionada até ela terminar.
  useEffect(() => {
    if (!selected) return;
    let alive = true;
    lastSeq.current = 0;
    setEvents([]);
    setRun(null);
    const tick = () =>
      getRun(selected, lastSeq.current)
        .then((d) => {
          if (!alive) return true;
          if (d.events.length > 0) {
            lastSeq.current = d.events[d.events.length - 1]!.seq;
            setEvents((prev) => [...prev, ...d.events]);
          }
          setRun(d.run);
          if (FINAL.has(d.run.status)) {
            refreshRuns();
            return true;
          }
          return false;
        })
        .catch((e: unknown) => {
          if (alive) setError(errText(e));
          return true;
        });
    let timer: ReturnType<typeof setTimeout> | undefined;
    const loop = async () => {
      const done = await tick();
      if (alive && !done) timer = setTimeout(loop, POLL_MS);
    };
    void loop();
    return () => {
      alive = false;
      if (timer) clearTimeout(timer);
    };
  }, [selected, refreshRuns]);

  const active = run !== null && !FINAL.has(run.status);
  const canSend = realCapable && workspaceId !== "" && draft.trim() !== "" && !active;

  const send = () => {
    setError(null);
    startRun({ workspace_id: workspaceId, engine: "codex", model: model || null, prompt: draft })
      .then((id) => {
        setDraft("");
        setSelected(id);
        refreshRuns();
      })
      .catch((e: unknown) => setError(errText(e)));
  };

  const authorize = () => {
    setError(null);
    authorizeWorkspace()
      .then((w) => {
        if (!w) return;
        setWorkspaces((prev) => (prev.some((x) => x.id === w.id) ? prev : [...prev, w]));
        setWorkspaceId(w.id);
      })
      .catch((e: unknown) => setError(errText(e)));
  };

  const providerStateText = providerStatus?.state === "connected"
    ? `Estado: conectado · ${providerStatus.label}`
    : providerStatus?.state === "simulated"
      ? "Estado: simulado (somente testes)"
      : providerStatus?.state === "error"
        ? `Estado: indisponível · ${providerStatus.message}`
        : providerStatus?.state === "disconnected"
          ? `Estado: desconectado · ${providerStatus.reason}`
          : "Estado: consultando o provedor…";

  const orchestratorNotice = providerStatus?.state === "simulated"
    ? "O provedor simulado existe somente nos testes e não produz respostas nesta interface."
    : providerStatus?.state === "connected"
      ? "Provedor conectado; o envio do Modo Orquestrador ainda não está disponível nesta fase."
      : providerStatus?.state === "error"
        ? "Não foi possível confirmar a conexão. O Modo Orquestrador permanece indisponível."
        : providerStatus?.state === "disconnected"
          ? "ChatGPT não conectado. Conecte uma conta na Fase 3B para usar o Modo Orquestrador."
          : "Consultando o estado do provedor…";

  const configuredProvider = providerStatus?.state === "connected"
    ? `Provedor configurado: ${providerStatus.label}`
    : providerStatus?.state === "simulated"
      ? `Provedor configurado: simulado · ${providerStatus.label}`
      : "Provedor configurado: nenhum · ChatGPT Plan Usage previsto na Fase 3B";

  return (
    <section className="chat">
      <PageHeader title="Chat" subtitle="Escolha entre delegar ao orquestrador ou executar diretamente no Gateway." />
      <div className="selectors">
        <label>
          Modo
          <select value={mode} onChange={(e) => setMode(e.target.value as ChatMode)} aria-label="Modo">
            <option value="direct">Direto</option>
            <option value="orchestrator">Orquestrador</option>
          </select>
        </label>
        {mode === "direct" && <>
        <label>
          Gateway
          <select value={gatewayId} onChange={(e) => setChosenGateway(e.target.value)} aria-label="Gateway">
            {gateways.map((g) => (
              <option key={g.id} value={g.id}>
                {g.name}
              </option>
            ))}
          </select>
        </label>
        <label>
          Motor
          <select value={engine} onChange={(e) => setEngine(e.target.value as EngineKind)} aria-label="Motor">
            {(Object.keys(ENGINE_LABEL) as EngineKind[]).map((k) => (
              <option key={k} value={k}>
                {ENGINE_LABEL[k]}
              </option>
            ))}
          </select>
        </label>
        <label>
          Modelo
          <select value={model} onChange={(e) => setModel(e.target.value)} aria-label="Modelo">
            <option value="">Padrão da CLI</option>
            {models.map((m) => (
              <option key={m.id} value={m.id}>
                {m.label ?? m.id}
                {m.confirmed ? "" : " (não confirmado)"}
              </option>
            ))}
          </select>
        </label>
        {gateway?.id === "local" && (
          <label>
            Workspace (somente leitura)
            <span className="row">
              <select value={workspaceId} onChange={(e) => setWorkspaceId(e.target.value)} aria-label="Workspace" disabled={!realCapable}>
                {workspaces.length === 0 && <option value="">Nenhuma pasta autorizada</option>}
                {workspaces.map((w) => (
                  <option key={w.id} value={w.id} title={w.path}>
                    {w.label}
                  </option>
                ))}
              </select>
              <button type="button" className="btn" onClick={authorize} disabled={!realCapable}>
                Autorizar pasta…
              </button>
            </span>
          </label>
        )}
        {gateway?.id !== "local" && (
          <label>
            Workspace
            <span className="muted">Indisponível até selecionar “Este Mac”.</span>
          </label>
        )}
        </>}
      </div>

      {mode === "orchestrator" ? (
        <>
          <div className="notice" role="status">{configuredProvider}</div>
          <div className="notice">{providerStateText}</div>
          <div className="chat-body">
            <div className="thread" role="log" aria-label="Conversa do Modo Orquestrador">
              <div className="notice">{orchestratorNotice}</div>
            </div>
          </div>
          <form className="composer" onSubmit={(e) => e.preventDefault()}>
            <textarea value="" placeholder="Indisponível enquanto não houver provedor conectado." rows={2} aria-label="Mensagem" disabled />
            <button type="button" disabled aria-label="Enviar">Enviar</button>
          </form>
        </>
      ) : (
      <>
      <div className="notice">Modo Direto · ChatGPT não participa desta execução.</div>
      <div className="chat-body">
        <div className="thread" role="log" aria-label="Conversa do Modo Direto">
          {!realCapable && (
            <div className="notice">Execução Direta indisponível. {whyNot}</div>
          )}
          {realCapable && !run && !selected && (
            <div className="notice">
              Execução real habilitada (Codex, somente leitura). Autorize uma pasta, escreva a tarefa e envie.
            </div>
          )}
          {realCapable && run && (
            <>
              <div className="msg user">
                <div className="msg-role">Você · {run.workspace_label}</div>
                <p>{run.prompt}</p>
              </div>
              <div className="run-head">
                <StatusBadge status={run.status} />
                {active && (
                  <button type="button" className="btn" onClick={() => void cancelRun(run.run_id)}>
                    Cancelar
                  </button>
                )}
              </div>
              <RunTimeline events={events} />
              {run.error_message && (
                <div className="ev bad" role="alert">
                  {run.error_message}
                </div>
              )}
              <RunMetrics run={run} />
            </>
          )}
          {error && (
            <div className="ev bad" role="alert">
              {error}
            </div>
          )}
        </div>

        {realCapable && (
          <aside className="history" aria-label="Histórico de execuções">
            <h2>Histórico</h2>
            {runs.length === 0 && <p className="muted small">Nenhuma execução ainda.</p>}
            <ul>
              {runs.map((r) => (
                <li key={r.run_id}>
                  <button
                    type="button"
                    className={`hist-item${selected === r.run_id ? " active" : ""}`}
                    onClick={() => setSelected(r.run_id)}
                  >
                    <span className="hist-prompt">{r.prompt}</span>
                    <span className="muted small">
                      {r.workspace_label} · <StatusBadge status={r.status} />
                    </span>
                  </button>
                </li>
              ))}
            </ul>
          </aside>
        )}
      </div>

      <form
        className="composer"
        onSubmit={(e) => {
          e.preventDefault();
          if (canSend) send();
        }}
      >
        <textarea
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          placeholder={
            realCapable
              ? workspaceId
                ? "Descreva a investigação (somente leitura)…"
                : "Autorize uma pasta de projeto para começar."
              : `Desativado: ${whyNot ?? ""}`
          }
          rows={2}
          aria-label="Mensagem"
          disabled={!realCapable || active}
        />
        <button type="submit" disabled={!canSend} aria-label="Enviar">
          Enviar
        </button>
      </form>
      </>
      )}
    </section>
  );
}
