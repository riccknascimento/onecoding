import { useEffect, useRef } from "react";
import type { ChatMode, ProviderStatus } from "../contracts";
import { Icon } from "../icons";

const MODES: { id: ChatMode; label: string; hint: string }[] = [
  { id: "orchestrator", label: "Orquestrador", hint: "Converse com o OneOrchestrator (ChatGPT), que planeja e, no futuro, delega ao Gateway." },
  { id: "direct", label: "Direto", hint: "Execute no Gateway sem o ChatGPT." },
];

function pill(status: ProviderStatus | null): { tone: "ok" | "warn" | "bad" | "muted"; text: string } {
  if (status === null) return { tone: "muted", text: "ChatGPT · consultando…" };
  switch (status.state) {
    case "connected":
      return status.plan_usage_authorized ? { tone: "ok", text: "ChatGPT conectado" } : { tone: "warn", text: "ChatGPT · autorização pendente" };
    case "simulated":
      return { tone: "warn", text: "Provedor simulado" };
    case "error":
      return { tone: "bad", text: "ChatGPT indisponível" };
    default:
      return { tone: "muted", text: "ChatGPT desconectado" };
  }
}

interface Props {
  title: string;
  mode: ChatMode;
  onMode: (m: ChatMode) => void;
  status: ProviderStatus | null;
  details: string[];
  busy: boolean;
  onRefresh: () => void;
  onDisconnect: () => void;
  onToggleRail: () => void;
}

/** Título da conversa, seletor de modo e indicador discreto de conexão. Os detalhes técnicos
 *  (conta, autorização) ficam num painel que só abre sob demanda. */
export function TopBar({ title, mode, onMode, status, details, busy, onRefresh, onDisconnect, onToggleRail }: Props) {
  const p = pill(status);
  const ref = useRef<HTMLDetailsElement>(null);
  useEffect(() => {
    const close = (e: Event) => {
      const el = ref.current;
      if (el?.open && e.target instanceof Node && !el.contains(e.target)) el.open = false;
    };
    const esc = (e: KeyboardEvent) => {
      if (e.key === "Escape" && ref.current?.open) ref.current.open = false;
    };
    document.addEventListener("pointerdown", close);
    document.addEventListener("keydown", esc);
    return () => {
      document.removeEventListener("pointerdown", close);
      document.removeEventListener("keydown", esc);
    };
  }, []);
  const connected = status?.state === "connected";
  const authorized = status?.state === "connected" && status.plan_usage_authorized;
  return (
    <div className="chat-top">
      <button type="button" className="icon-btn rail-toggle" onClick={onToggleRail} aria-label="Abrir histórico" title="Histórico">
        <Icon name="history" />
      </button>
      <h2 className="chat-title" title={title}>{title}</h2>
      <div className="mode-switch" role="radiogroup" aria-label="Modo">
        {MODES.map((m) => (
          <button key={m.id} type="button" role="radio" aria-checked={mode === m.id} className={mode === m.id ? "on" : ""} title={m.hint} onClick={() => onMode(m.id)}>
            {m.label}
          </button>
        ))}
      </div>
      <details className="status" ref={ref}>
        <summary className={`status-pill ${p.tone}`} aria-label="Estado da conexão com o ChatGPT">
          <span className="dot" aria-hidden="true" />
          {p.text}
          <Icon name="chevron" />
        </summary>
        <div className="status-panel">
          {details.map((d) => (
            <p key={d}>{d}</p>
          ))}
          {(authorized || connected) && (
            <div className="row">
              {authorized && (
                <button type="button" className="btn" onClick={onRefresh} disabled={busy}>
                  {busy ? "Atualizando…" : "Atualizar modelos"}
                </button>
              )}
              {connected && (
                <button type="button" className="btn" onClick={onDisconnect} disabled={busy}>
                  Desconectar
                </button>
              )}
            </div>
          )}
        </div>
      </details>
    </div>
  );
}
