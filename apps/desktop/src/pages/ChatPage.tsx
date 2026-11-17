import { useMemo, useState } from "react";
import type { LocalGatewayState } from "../App";
import type { EngineKind } from "../contracts";
import { DEMO_CHAT, DEMO_GATEWAYS } from "../demo/data";
import { DemoBadge, PageHeader } from "./common";

const ENGINE_LABEL: Record<EngineKind, string> = { codex: "Codex CLI", claude: "Claude Code", cursor: "Cursor CLI" };

export function ChatPage({ local }: { local: LocalGatewayState }) {
  const gateways = useMemo(
    () => [...(local.phase === "ready" ? [local.gateway] : []), ...DEMO_GATEWAYS],
    [local],
  );
  // Escolha explícita do usuário; sem escolha, vale o primeiro da lista (o local, assim que detectado).
  const [chosenGateway, setChosenGateway] = useState<string | null>(null);
  const gatewayId = chosenGateway ?? gateways[0]?.id ?? "";
  const [engine, setEngine] = useState<EngineKind>("claude");
  const [model, setModel] = useState("");
  const [draft, setDraft] = useState("");

  const gateway = gateways.find((g) => g.id === gatewayId);
  const models = gateway?.engines.find((e) => e.engine === engine)?.models ?? [];

  return (
    <section className="chat">
      <PageHeader title="Chat / Orquestrador" subtitle="Converse com o orquestrador e delegue tarefas aos motores." />
      <div className="selectors">
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
      </div>

      <div className="thread" role="log" aria-label="Conversa de demonstração">
        <div className="notice">
          <DemoBadge /> Conversa de exemplo. O orquestrador não está conectado nesta fase.
        </div>
        {DEMO_CHAT.map((m, i) => (
          <div key={i} className={`msg ${m.role}`}>
            <div className="msg-role">{m.role === "user" ? "Você" : "Orquestrador"}</div>
            <p>{m.text}</p>
          </div>
        ))}
      </div>

      <form
        className="composer"
        onSubmit={(e) => {
          e.preventDefault();
        }}
      >
        <textarea
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          placeholder="Orquestrador desativado nesta fase (sem provedor autenticado)."
          rows={2}
          aria-label="Mensagem"
          disabled
        />
        <button type="submit" disabled aria-label="Enviar">
          Enviar
        </button>
      </form>
    </section>
  );
}
