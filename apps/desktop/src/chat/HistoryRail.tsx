import type { ChatConversation, RunView } from "../contracts";
import { Icon } from "../icons";
import { StatusBadge } from "../pages/RunTimeline";
import { formatWhen } from "./model";

export type HistoryKey = `c:${string}` | `r:${string}`;

type Item =
  | { key: HistoryKey; kind: "conversation"; ts: string; title: string; conv: ChatConversation }
  | { key: HistoryKey; kind: "run"; ts: string; title: string; run: RunView };

interface Props {
  conversations: ChatConversation[];
  runs: RunView[];
  activeKey: HistoryKey | null;
  /** Trocar de conversa enquanto o ChatGPT responde seria ambíguo. */
  conversationsLocked: boolean;
  onOpenConversation: (c: ChatConversation) => void;
  onOpenRun: (r: RunView) => void;
  onNew: () => void;
  newLabel: string;
  open: boolean;
  onClose: () => void;
}

/** Histórico único. Conversas do Orquestrador e execuções do Modo Direto continuam em estruturas
 *  separadas; aqui só aparecem juntas, em ordem cronológica e com o modo de cada item identificado. */
export function HistoryRail({ conversations, runs, activeKey, conversationsLocked, onOpenConversation, onOpenRun, onNew, newLabel, open, onClose }: Props) {
  const items: Item[] = [
    ...conversations.map((conv): Item => ({ key: `c:${conv.id}`, kind: "conversation", ts: conv.updatedAt, title: conv.title, conv })),
    ...runs.map((run): Item => ({ key: `r:${run.run_id}`, kind: "run", ts: run.created_at, title: run.prompt, run })),
  ].sort((a, b) => (a.ts < b.ts ? 1 : a.ts > b.ts ? -1 : 0));

  return (
    <>
      {open && <div className="rail-scrim" onClick={onClose} aria-hidden="true" />}
      <aside className={`rail${open ? " open" : ""}`} aria-label="Histórico de conversas e execuções">
        <div className="rail-head">
          <h2>Histórico</h2>
          <button type="button" className="icon-btn" onClick={onNew} aria-label={newLabel} title={newLabel}>
            <Icon name="plus" />
          </button>
        </div>
        {items.length === 0 ? (
          <p className="rail-empty">Nenhuma conversa ou execução ainda. Elas aparecem aqui, identificadas pelo modo.</p>
        ) : (
          <ul>
            {items.map((it) => {
              const locked = it.kind === "conversation" && conversationsLocked;
              return (
                <li key={it.key}>
                  <button
                    type="button"
                    className={`rail-item${activeKey === it.key ? " active" : ""}`}
                    aria-current={activeKey === it.key ? "true" : undefined}
                    disabled={locked}
                    onClick={() => {
                      if (it.kind === "conversation") onOpenConversation(it.conv);
                      else onOpenRun(it.run);
                      onClose();
                    }}
                  >
                    <span className="rail-title">{it.title}</span>
                    <span className="rail-meta">
                      <span className={`mode-tag ${it.kind === "conversation" ? "orch" : "direct"}`}>{it.kind === "conversation" ? "VibeAgent" : "Coding"}</span>
                      {it.kind === "run" ? (
                        <>
                          <span className="rail-ws">{it.run.workspace_label}</span>
                          <StatusBadge status={it.run.status} />
                        </>
                      ) : (
                        <span className="rail-ws">{formatWhen(it.ts)}</span>
                      )}
                    </span>
                  </button>
                </li>
              );
            })}
          </ul>
        )}
      </aside>
    </>
  );
}
