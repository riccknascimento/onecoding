import { useCallback, useEffect, useState } from "react";
import { approveExecutable, detectLocalGateway, inTauri } from "./api";
import type { EngineKind, GatewayInfo } from "./contracts";
import { Icon, type IconName } from "./icons";
import { ChatPage } from "./pages/ChatPage";
import { ContextPage } from "./pages/ContextPage";
import { GatewaysPage } from "./pages/GatewaysPage";
import { ProjectsPage } from "./pages/ProjectsPage";
import { RunsPage } from "./pages/RunsPage";
import { SettingsPage } from "./pages/SettingsPage";
import { SkillsPage } from "./pages/SkillsPage";

export type LocalGatewayState =
  | { phase: "loading" }
  | { phase: "unavailable" } // fora do app (navegador/testes)
  | { phase: "error"; message: string }
  | { phase: "ready"; gateway: GatewayInfo };

const SECTIONS: { id: string; label: string; icon: IconName }[] = [
  { id: "chat", label: "Chat / Orquestrador", icon: "chat" },
  { id: "gateways", label: "Agent Gateways", icon: "gateway" },
  { id: "projects", label: "Projetos", icon: "projects" },
  { id: "context", label: "OneContext", icon: "context" },
  { id: "skills", label: "Skills & Agents", icon: "skills" },
  { id: "runs", label: "Execuções", icon: "runs" },
  { id: "settings", label: "Configurações", icon: "settings" },
];

function useLocalGateway(): { local: LocalGatewayState; refresh: () => void } {
  const [local, setLocal] = useState<LocalGatewayState>(inTauri() ? { phase: "loading" } : { phase: "unavailable" });
  const [tick, setTick] = useState(0);
  useEffect(() => {
    if (!inTauri()) return;
    let alive = true;
    detectLocalGateway(true)
      .then((gateway) => alive && setLocal(gateway ? { phase: "ready", gateway } : { phase: "unavailable" }))
      .catch((e: unknown) => alive && setLocal({ phase: "error", message: String(e) }));
    return () => {
      alive = false;
    };
  }, [tick]);
  const refresh = useCallback(() => setTick((t) => t + 1), []);
  return { local, refresh };
}

export function App() {
  const [section, setSection] = useState("chat");
  const { local, refresh } = useLocalGateway();
  const onApprove = async (engine: EngineKind, fingerprint: string) => {
    await approveExecutable(engine, fingerprint);
    refresh();
  };

  return (
    <div className="shell">
      <aside className="sidebar">
        <div className="drag" data-tauri-drag-region />
        <div className="brand">
          <span className="brand-mark" aria-hidden="true" />
          <span>OneAgent</span>
        </div>
        <nav aria-label="Principal">
          {SECTIONS.map((s) => (
            <button
              key={s.id}
              type="button"
              className={`nav-item${section === s.id ? " active" : ""}`}
              aria-current={section === s.id ? "page" : undefined}
              onClick={() => setSection(s.id)}
            >
              <Icon name={s.icon} />
              {s.label}
            </button>
          ))}
        </nav>
        <div className="sidebar-foot">Fase 2A · execução local</div>
      </aside>
      <main className="main">
        <div className="drag" data-tauri-drag-region />
        {section === "chat" && <ChatPage local={local} />}
        {section === "gateways" && <GatewaysPage local={local} onApprove={onApprove} />}
        {section === "projects" && <ProjectsPage />}
        {section === "context" && <ContextPage />}
        {section === "skills" && <SkillsPage />}
        {section === "runs" && <RunsPage />}
        {section === "settings" && <SettingsPage />}
      </main>
    </div>
  );
}
