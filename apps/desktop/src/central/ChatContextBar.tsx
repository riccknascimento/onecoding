// Cabeçalho do chat, em três níveis: alvo (OneNode e projeto), modo (como a análise acontece) e detalhes
// (motor e modelos). Só mostra o que existe de verdade no OneHub. O contexto é persistido por conversa pelo pai.
import { useState } from "react";
import type { CentralExecutor, CentralGrant, CentralNode } from "../transport";

export const CENTRAL_OPTION = "";

type Tone = "ok" | "warn" | "bad" | "info" | "central";

export function nodeStateText(n: CentralNode): { text: string; tone: Exclude<Tone, "central"> } {
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

/**
 * Pronto = algum online com relatório de Codex instalado+autenticado+aprovado.
 * Sem campo `engines` (payload antigo) = desconhecido: não bloqueia só por autenticação.
 */
export function codexReady(executors: CentralExecutor[]): boolean | "unknown" {
  const online = executors.filter((x) => x.status === "online");
  if (online.length === 0) return false;
  const reported = online.filter((x) => Array.isArray(x.engines));
  if (reported.length === 0) return "unknown";
  return reported.some((x) =>
    (x.engines ?? []).some((e) => e.name === "codex" && e.installed && e.authenticated && e.approved),
  );
}

/** Motivo que impede a análise direta neste contexto, ou null quando ela pode ser pedida. */
export function directBlockReason(p: {
  node: CentralNode | null;
  grants: CentralGrant[];
  executorOnline: boolean;
  projectId: string;
  executors?: CentralExecutor[];
}): string | null {
  if (!p.node) return "Contexto OneHub Central: administração do ambiente, sem análise de projeto.";
  if (p.node.status === "revoked") return "Este OneNode foi revogado.";
  if (p.node.suspended) return "Este OneNode está suspenso até ser retomado.";
  if (p.grants.length === 0) return "Este OneNode está conectado, mas sem projetos autorizados.";
  if (!p.executorOnline) return "Nenhum OneExecutor (Codex) está online neste Central.";
  if (p.executors) {
    const ready = codexReady(p.executors);
    if (ready === false) {
      const reports = p.executors.filter((x) => x.status === "online").flatMap((x) => x.engines ?? []);
      const codex = reports.find((e) => e.name === "codex");
      if (codex?.installed && !codex.authenticated) {
        return "O OneExecutor está online, mas o Codex CLI ainda não está autenticado. Conecte a conta em One Provider AI.";
      }
      if (codex?.installed && codex.authenticated && !codex.approved) {
        return "O Codex CLI está instalado, mas ainda não foi aprovado neste OneExecutor.";
      }
      return "O OneExecutor está online, mas o Codex CLI não está pronto para execução.";
    }
  }
  if (!p.projectId && p.grants.length > 1) return "Escolha o projeto para a análise.";
  return null;
}

function Dot({ tone }: { tone: Tone }) {
  return <span className={`cx-dot ${tone}`} aria-hidden="true" />;
}

export function ChatContextBar(p: Props) {
  const node = p.nodes.find((n) => n.id === p.nodeId) ?? null;
  const nodeGrants = p.grants.filter((g) => g.node_id === p.nodeId);
  const executorOnline = p.executors.some((x) => x.status === "online");
  const ready = codexReady(p.executors);
  const executorText =
    p.executors.length === 0
      ? "nenhum cadastrado"
      : !executorOnline
        ? "offline"
        : ready === true
          ? "online · Codex autenticado"
          : ready === "unknown"
            ? "online"
            : "online · Codex sem autenticação";
  const blocked = directBlockReason({
    node,
    grants: nodeGrants,
    executorOnline,
    projectId: p.projectId,
    executors: p.executors,
  });
  const state = node ? nodeStateText(node) : null;
  const tone: Tone = state ? state.tone : "central";
  const projectName = nodeGrants.find((g) => g.project_id === p.projectId)?.project_name ?? "Automático";
  const [detailsOpen, setDetailsOpen] = useState(false);
  const motorTone: Tone = ready === true ? "ok" : executorOnline ? (ready === "unknown" ? "ok" : "warn") : "bad";

  return (
    <section className={`ctx-panel cx tone-${tone}`} aria-label="Contexto do chat">
      <div className="cx-target">
        <div className="cx-field cx-node">
          <label className="cx-label">
            <span className="cx-kicker">OneNode</span>
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
          <span className="cx-state">
            <Dot tone={tone} />
            {state ? state.text : "sem OneNode: contexto de administração"}
          </span>
        </div>

        {node && nodeGrants.length > 0 && (
          <label className="cx-field cx-project">
            <span className="cx-kicker">Projeto</span>
            <select value={p.projectId} onChange={(e) => p.onProject(e.target.value)}>
              <option value="">Automático</option>
              {nodeGrants.map((g) => (
                <option key={g.project_id} value={g.project_id}>{g.project_name}</option>
              ))}
            </select>
          </label>
        )}

        {node && nodeGrants.length === 0 && (
          <div className="cx-field cx-project cx-empty">
            <span className="cx-kicker">Projeto</span>
            <span className="cx-empty-text">Nenhum projeto autorizado.</span>
            {p.isAdmin ? (
              <button type="button" className="cx-action" onClick={p.onOpenOneNodes}>Conceder acesso em OneNodes</button>
            ) : (
              <span className="cx-empty-text">Peça ao administrador uma concessão.</span>
            )}
          </div>
        )}

        <button type="button" className="cx-details-toggle" aria-expanded={detailsOpen} aria-controls="ctx-more" onClick={() => setDetailsOpen((o) => !o)}>
          Motor e modo
        </button>
      </div>

      <div className="cx-mode">
        <div className="cx-segments" role="group" aria-label="Modo da conversa">
          <span className="mode-chip disabled" aria-disabled="true" title="O OneOrchestrator com ChatGPT não roda neste Central">
            Orquestrador · indisponível
          </span>
          <span className={`mode-chip${blocked ? " disabled" : " active"}`} aria-current={blocked ? undefined : "true"}>
            Direto {blocked ? "· indisponível" : "· ativo"}
          </span>
        </div>
        {blocked ? (
          <p className="cx-reason">{blocked}</p>
        ) : (
          <p className="cx-ready">Análise direta pronta · projeto: {projectName}</p>
        )}
      </div>

      <dl id="ctx-more" className={`cx-details${detailsOpen ? " open" : ""}`}>
        <div>
          <dt>Motor</dt>
          <dd>
            <Dot tone={motorTone} />
            Codex CLI (OneExecutor): {executorText}
          </dd>
        </div>
        <div>
          <dt>Outros motores</dt>
          <dd>Claude Code e Cursor · indisponíveis neste Central</dd>
        </div>
        <div>
          <dt>Modelos</dt>
          <dd>CLI: não informado pelo OneExecutor · ChatGPT: indisponível (Orquestrador desligado)</dd>
        </div>
      </dl>
    </section>
  );
}
