import type { ReactNode } from "react";
import type { ChatMode, EngineKind, ExecutorPreference, GatewayInfo, ModelInfo, Workspace } from "../contracts";
import { Icon } from "../icons";
import type { ChatModelOption } from "./model";

const AUTHORIZE = "__authorize__";

const ENGINES: { kind: EngineKind; label: string; available: boolean }[] = [
  { kind: "codex", label: "Codex CLI", available: true },
  { kind: "claude", label: "Claude Code", available: false },
  { kind: "cursor", label: "Cursor CLI", available: false },
];

function Chip({ label, title, disabled, children }: { label: string; title?: string; disabled?: boolean; children: ReactNode }) {
  return (
    <label className={`chip${disabled ? " is-disabled" : ""}`} title={title}>
      <span className="chip-k">{label}</span>
      {children}
      <Icon name="chevron" />
    </label>
  );
}

/** `select` nativo ocupa a largura da maior opção; aqui ele acompanha só o texto escolhido (CSSOM, sem
 *  `style=""` no HTML, então a CSP continua fechada). */
function Sel({ label, value, display, disabled, onChange, children }: { label: string; value: string; display: string; disabled?: boolean; onChange: (v: string) => void; children: ReactNode }) {
  const width = `${Math.min(Math.max(display.length, 5), 26) + 1}ch`;
  return (
    <select aria-label={label} value={value} disabled={disabled} style={{ width }} onChange={(e) => onChange(e.target.value)}>
      {children}
    </select>
  );
}

interface Props {
  mode: ChatMode;
  gateways: GatewayInfo[];
  localPhase: "loading" | "unavailable" | "error" | "ready";
  gatewayId: string;
  onGateway: (id: string) => void;
  workspaces: Workspace[];
  workspaceId: string;
  workspaceEnabled: boolean;
  onWorkspace: (id: string) => void;
  onAuthorize: () => void;
  engine: ExecutorPreference;
  onEngine: (e: ExecutorPreference) => void;
  models: ModelInfo[];
  executorModel: string;
  onExecutorModel: (m: string) => void;
  chatModels: ChatModelOption[];
  chatModel: string;
  onChatModel: (m: string) => void;
  chatBusy: boolean;
}

/** Barra compacta de contexto operacional. Igual nos dois modos; só o modelo do ChatGPT é exclusivo do
 *  Orquestrador. Escolher um recurso aqui nunca concede permissão de execução. */
export function ContextBar(p: Props) {
  const orchestrator = p.mode === "orchestrator";
  const autoEngine = p.engine === "auto";
  return (
    <div className="ctx-bar" role="group" aria-label="Contexto operacional">
      <div className="ctx-group">
        <Chip
          label="OneNode"
          title={orchestrator ? "OneNode que executa as tarefas. “Automático” depende do roteamento do OneHub, ainda não disponível." : "OneNode que executa a tarefa. Escolha um explicitamente."}
        >
          <Sel label="OneNode" value={p.gatewayId} display={p.gateways.find((g) => g.id === p.gatewayId)?.name ?? (orchestrator ? "Automático · em breve" : "Selecione um OneNode")} onChange={p.onGateway}>
            {p.gatewayId === "" && (
              <option value="" disabled>
                {p.localPhase === "loading" ? "Detectando…" : orchestrator ? "Automático · em breve" : "Selecione um OneNode"}
              </option>
            )}
            {orchestrator && (
              <option value="auto" disabled>
                Automático · em breve
              </option>
            )}
            {p.gateways.map((g) => (
              <option key={g.id} value={g.id} disabled={g.demo || g.status !== "online"}>
                {g.name}
                {g.demo || g.status !== "online" ? " — desconectado" : ""}
              </option>
            ))}
          </Sel>
        </Chip>
        <Chip label="Projeto / Workspace" title="Pasta autorizada, somente leitura." disabled={!p.workspaceEnabled}>
          <Sel
            label="Projeto / Workspace"
            value={p.workspaceId}
            display={p.workspaces.find((w) => w.id === p.workspaceId)?.label ?? "Nenhuma pasta autorizada"}
            disabled={!p.workspaceEnabled}
            onChange={(v) => (v === AUTHORIZE ? p.onAuthorize() : p.onWorkspace(v))}
          >
            {!p.workspaceEnabled && <option value="">Indisponível sem OneNode</option>}
            {p.workspaceEnabled && p.workspaces.length === 0 && <option value="">Nenhuma pasta autorizada</option>}
            {p.workspaces.map((w) => (
              <option key={w.id} value={w.id} title={w.path}>
                {w.label}
              </option>
            ))}
            {p.workspaceEnabled && <option value={AUTHORIZE}>Autorizar pasta…</option>}
          </Sel>
        </Chip>
        <Chip
          label="Motor executor"
          title={orchestrator ? "Executor preferido. “Automático” é só uma preferência futura: nada é delegado nesta etapa." : "Executor obrigatório da execução direta."}
        >
          <Sel label="Motor executor" value={p.engine} display={p.engine === "auto" ? "Automático · futuro" : (ENGINES.find((e) => e.kind === p.engine)?.label ?? "")} onChange={(v) => p.onEngine(v as ExecutorPreference)}>
            {orchestrator && <option value="auto">Automático · futuro</option>}
            {ENGINES.map((e) => (
              <option key={e.kind} value={e.kind} disabled={!e.available}>
                {e.label}
                {e.available ? "" : " — indisponível"}
              </option>
            ))}
          </Sel>
        </Chip>
        <Chip label="Modelo executor" title="Modelo do executor (não é o modelo do ChatGPT)." disabled={autoEngine}>
          <Sel label="Modelo executor" value={p.executorModel} display={p.models.find((m) => m.id === p.executorModel)?.label ?? (p.executorModel || "Padrão da CLI")} disabled={autoEngine} onChange={p.onExecutorModel}>
            <option value="">Padrão da CLI</option>
            {p.models.map((m) => (
              <option key={m.id} value={m.id}>
                {m.label ?? m.id}
                {m.confirmed ? "" : " (não confirmado)"}
              </option>
            ))}
          </Sel>
        </Chip>
      </div>
      {orchestrator && (
        <div className="ctx-group ctx-orch">
          <Chip label="Modelo ChatGPT — OneOrchestrator" title="Modelo que o OneOrchestrator usa para conversar e planejar." disabled={p.chatModels.length === 0}>
            <Sel label="Modelo do ChatGPT" value={p.chatModel} display={p.chatModels.find((m) => m.slug === p.chatModel)?.label ?? "Conecte para listar"} disabled={p.chatBusy || p.chatModels.length === 0} onChange={p.onChatModel}>
              {p.chatModels.length === 0 && <option value="">Conecte para listar</option>}
              {p.chatModels.map((m) => (
                <option key={m.slug} value={m.slug}>
                  {m.label}
                </option>
              ))}
            </Sel>
          </Chip>
        </div>
      )}
    </div>
  );
}
