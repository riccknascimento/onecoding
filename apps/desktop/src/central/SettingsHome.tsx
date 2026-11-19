import type { CentralExecutor, CentralNode } from "../transport";
import type { CentralView, SettingsSection } from "./Sidebar";
import { Icon } from "../icons";
import type { IconName } from "../icons";

const settings: { label: SettingsSection | "Provider AI"; icon: IconName; description: string }[] = [
  { label: "Orquestrador", icon: "chat", description: "Coordenação e políticas de execução." },
  { label: "OneExecutors", icon: "runs", description: "Motores de execução autorizados." },
  { label: "OneNodes", icon: "nodes", description: "Servidores conectados ao OneHub." },
  { label: "Provider AI", icon: "gateway", description: "Contas e provedores de modelos de IA." },
];
const resources: { label: string; view: CentralView; icon: IconName }[] = [
  { label: "Agent", view: "agents", icon: "bot" },
  { label: "Skill", view: "skills", icon: "skills" },
  { label: "Plugins", view: "plugins", icon: "plus" },
  { label: "MCP", view: "mcp", icon: "gateway" },
  { label: "Context", view: "context", icon: "book" },
];

export function SettingsHome({ nodes, executors, onSettings, onView }: {
  nodes: CentralNode[];
  executors: CentralExecutor[];
  onSettings(section: SettingsSection): void;
  onView(view: CentralView): void;
}) {
  const activate = (label: SettingsSection | "Provider AI") => label === "Provider AI" ? onView("provider") : onSettings(label);
  return <main className="settings-home">
    <header className="settings-home-head"><div><h1>Settings</h1><p>Configuração global da plataforma OneCoding</p></div></header>
    <div className="settings-card-grid">
      {settings.map((item) => <button className="settings-card" type="button" key={item.label} onClick={() => activate(item.label)}>
        <span className="settings-card-icon"><Icon name={item.icon} /></span>
        <span className="settings-card-copy"><strong>{item.label === "OneExecutors" ? "Executors" : item.label === "OneNodes" ? "OneNode" : item.label}</strong><span>{item.description}</span>
          {item.label === "OneExecutors" && <small>{executors.filter((x) => x.status === "online").length} online</small>}
          {item.label === "OneNodes" && <small>{nodes.filter((x) => x.status === "online").length} online</small>}
        </span><span className="settings-card-link">Configurar →</span>
      </button>)}
    </div>
    <section className="settings-resources"><header><Icon name="projects" /><div><h2>Recursos</h2><p>Gerencie os recursos disponíveis na plataforma.</p></div></header>
      <div className="resource-card-grid">{resources.map((item) => <button type="button" key={item.view} onClick={() => onView(item.view)}><Icon name={item.icon} /><strong>{item.label}</strong><span>›</span></button>)}</div>
    </section>
  </main>;
}
