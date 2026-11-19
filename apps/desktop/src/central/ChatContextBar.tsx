// Cabeçalho do chat: OneNode, projeto, motor e modo. Mostra só o que existe de verdade no OneHub.
// O contexto é persistido por conversa pelo pai (PUT /v1/conversations/{id}/context); aqui só se exibe e se escolhe.
import { useState } from "react";
import type { CentralExecutor, CentralGrant, CentralNode } from "../transport";

export const CENTRAL_OPTION = "";

export function nodeStateText(n: CentralNode): { text: string; tone: "ok" | "warn" | "bad" | "info" } {
  if (n.status === "revoked") return { text: "revogado", tone: "bad" };
  if (n.suspended) return { text: "suspenso", tone: "warn" };
  if (n.status === "online") return { text: "online", tone: "ok" };
  if (n.status === "offline") return { text: "offline", tone: "bad" };
  return { text: "aguardando pareamento", tone: "info" };
}

interface Props {
  nodes: CentralNode[];
  grants: CentralGrant[];
  executors: CentralExecutor[];
  nodeId: string;
  projectId: string;
  isAdmin: boolean;
  onNode(nodeId: string): void;
  onProject(projectId: string): void;
  onOpenOneNodes(): void;
}

/** Motivo que impede a análise direta neste contexto, ou null quando ela pode ser pedida. */
export function directBlockReason(p: { node: CentralNode | null; grants: CentralGrant[]; executorOnline: boolean; projectId: string }): string | null {
  if (!p.node) return "Contexto OneHub Central: administração do ambiente, sem análise de projeto.";
  if (p.node.status === "revoked") return "Este OneNode foi revogado.";
  if (p.node.suspended) return "Este OneNode está suspenso até ser retomado.";
  if (p.grants.length === 0) return "Este OneNode está conectado, mas sem projetos autorizados.";
  if (!p.executorOnline) return "Nenhum OneExecutor (Codex) está online neste Central.";
  if (!p.projectId && p.grants.length > 1) return "Escolha o projeto para a análise.";
  return null;
}

export function ChatContextBar(p: Props) {
  const node = p.nodes.find((n) => n.id === p.nodeId) ?? null;
  const nodeGrants = p.grants.filter((g) => g.node_id === p.nodeId);
  const executorOnline = p.executors.some((x) => x.status === "online");
  const executorText = p.executors.length === 0 ? "nenhum cadastrado" : executorOnline ? "online" : "offline";
  const blocked = directBlockReason({ node, grants: nodeGrants, executorOnline, projectId: p.projectId });
  const state = node ? nodeStateText(node) : null;
  // no celular o motor e o modo ficam recolhidos; no desktop ficam sempre visíveis (CSS)
  const [moreOpen, setMoreOpen] = useState(false);

  return (
    <div className="ctx-panel" aria-label="Contexto do chat">
      <div className="ctx-row">
        <label className="ctx-field">
          <span className="chip-k">OneNode</span>
          <select value={p.nodeId} onChange={(e) => p.onNode(e.target.value)}>
            <option value={CENTRAL_OPTION}>OneHub Central (administração)</option>
            {p.nodes.map((n) => {
              const st = nodeStateText(n);
              return (
                <option key={n.id} value={n.id} disabled={n.status === "revoked"}>
                  {n.name} · {st.text}
                </option>
              );
            })}
          </select>
        </label>

        {node && (
          nodeGrants.length > 0 ? (
            <label className="ctx-field">
              <span className="chip-k">Projeto</span>
              <select value={p.projectId} onChange={(e) => p.onProject(e.target.value)}>
                <option value="">Automático</option>
                {nodeGrants.map((g) => (
                  <option key={g.project_id} value={g.project_id}>{g.project_name}</option>
                ))}
              </select>
            </label>
          ) : (
            <div className="ctx-field">
              <span className="chip-k">Projeto</span>
              <span className="muted small">Nenhum projeto autorizado.</span>
              {p.isAdmin ? (
                <button type="button" className="btn small" onClick={p.onOpenOneNodes}>Conceder acesso em OneNodes</button>
              ) : (
                <span className="muted small">Peça ao administrador uma concessão.</span>
              )}
            </div>
          )
        )}

        <div className="ctx-status" aria-label="Estado do OneNode">
          {state ? (
            <span className={`status-pill ${state.tone}`}><span className="dot" aria-hidden="true" />{state.text}</span>
          ) : (
            <span className="status-pill info"><span className="dot" aria-hidden="true" />OneHub Central</span>
          )}
          <button type="button" className="ctx-toggle" aria-expanded={moreOpen} aria-controls="ctx-more" onClick={() => setMoreOpen((o) => !o)}>
            Motor e modo
          </button>
        </div>
      </div>

      <div id="ctx-more" className={`ctx-more${moreOpen ? " open" : ""}`}>
      <div className="ctx-row ctx-meta">
        <span className={`status-pill ${executorOnline ? "ok" : "bad"}`} title="OneExecutor que executa o Codex CLI">
          <span className="dot" aria-hidden="true" />
          Codex CLI (OneExecutor): {executorText}
        </span>
        <span className="status-pill" title="Claude Code ainda não tem integração neste Central">
          <span className="dot" aria-hidden="true" />Claude Code · indisponível
        </span>
        <span className="status-pill" title="Cursor ainda não tem integração neste Central">
          <span className="dot" aria-hidden="true" />Cursor · indisponível
        </span>
        <span className="muted small" title="O OneExecutor não informa a lista de modelos">Modelo CLI: não informado pelo OneExecutor</span>
      </div>

      <div className="ctx-row ctx-meta" role="group" aria-label="Modo da conversa">
        <span className="mode-chip disabled" aria-disabled="true" title="O OneOrchestrator com ChatGPT não roda neste Central">
          Orquestrador · indisponível
        </span>
        <span className={`mode-chip${blocked ? " disabled" : " active"}`} aria-current={blocked ? undefined : "true"}>
          Direto {blocked ? "· indisponível" : "· ativo"}
        </span>
        <span className="muted small">Modelo ChatGPT: indisponível (Orquestrador desligado)</span>
      </div>

      </div>

      {blocked && <p className="ctx-blocked">{blocked}</p>}
    </div>
  );
}
