import { useState } from "react";
import type { ReactNode } from "react";
import type { CentralConversation } from "../transport";
import type { CentralWorkspace } from "../transport";
import { Icon } from "../icons";
import type { IconName } from "../icons";

export type CentralView =
  | "chat"
  | "onenodes"
  | "provider"
  | "projects"
  | "context"
  | "skills"
  | "agents"
  | "plugins"
  | "mcp"
  | "executions"
  | "settings";

/** Seções de Configurações (cada uma tem endereço em /configuracoes/<seção>). */
export type SettingsSection = "Orquestrador" | "OneExecutors" | "OneNodes" | "Projetos";

interface NavItem {
  view: CentralView;
  label: string;
  icon: IconName;
}

export const RESOURCE_ITEMS: NavItem[] = [
  { view: "agents", label: "Agent", icon: "bot" },
  { view: "skills", label: "Skill", icon: "skills" },
  { view: "plugins", label: "Plugins", icon: "plus" },
  { view: "mcp", label: "MCP", icon: "gateway" },
  { view: "context", label: "Context", icon: "book" },
  { view: "projects", label: "Projetos", icon: "projects" },
  { view: "executions", label: "Execuções", icon: "list" },
];

/** Configurações: Orquestrador, Executors, OneNode e Provider AI. */
export const CONFIG_ITEMS: { section?: SettingsSection; view?: CentralView; label: string; icon: IconName }[] = [
  { section: "Orquestrador", label: "Orquestrador", icon: "chat" },
  { section: "OneExecutors", label: "Executors", icon: "runs" },
  { section: "OneNodes", label: "OneNode", icon: "nodes" },
  { view: "provider", label: "Provider AI", icon: "gateway" },
];

interface SidebarProps {
  view: CentralView;
  settingsSection: SettingsSection | null;
  isAdmin: boolean;
  email: string;
  conversations: CentralConversation[];
  workspaces: CentralWorkspace[];
  activeWorkspaceId: string | null;
  activeId: string | null;
  open: boolean;
  onView(v: CentralView): void;
  onSettings(section: SettingsSection): void;
  onSelectConversation(id: string): void;
  onNewConversation(): void;
  onOpenWorkspace(id: string): void;
  onLogout(): void;
}

/** Grupo recolhível: o cabeçalho é um botão com `aria-expanded`; começa aberto. */
function Group({ title, children, action }: { title: string; children: ReactNode; action?: ReactNode }) {
  const [open, setOpen] = useState(true);
  return (
    <section className="rail-group" aria-label={title}>
      <div className="rail-group-head">
        <button type="button" className="rail-group-toggle" aria-expanded={open} onClick={() => setOpen((o) => !o)}>
          <Icon name="chevron" />
          <span>{title}</span>
        </button>
        {action}
      </div>
      {open && <div className="rail-group-body">{children}</div>}
    </section>
  );
}

export function Sidebar(p: SidebarProps) {
  return (
    <aside id="studio-rail" className={`rail${p.open ? " open" : ""}`} aria-label="Navegação principal">
      <div className="rail-sections">
      <Group title="Workspaces">
        <ul className="rail-list">
          {p.workspaces.map((workspace) => <li key={workspace.id}>
            <button type="button" className={`rail-item nav-item${workspace.id === p.activeWorkspaceId ? " active" : ""}`} aria-current={workspace.id === p.activeWorkspaceId ? "page" : undefined} onClick={() => p.onOpenWorkspace(workspace.id)}>
              <Icon name="projects" /><span className="rail-title">{workspace.name}</span>
            </button>
          </li>)}
          {p.workspaces.length === 0 && <li className="rail-empty">Nenhum Workspace disponível.</li>}
        </ul>
      </Group>

      {p.activeWorkspaceId && <Group
        title="Conversas"
      >
        <button type="button" className="new-chat" onClick={p.onNewConversation}><Icon name="plus" /> Nova conversa</button>
        {p.conversations.length === 0 ? (
          <p className="rail-empty">Nenhuma conversa ainda. Escreva no chat para começar.</p>
        ) : (
          <ul className="rail-list">
            {p.conversations.map((c) => {
              const active = c.id === p.activeId && p.view === "chat";
              return (
                <li key={c.id}>
                  <button
                    type="button"
                    className={`rail-item conv-item${active ? " active" : ""}`}
                    aria-current={active ? "true" : undefined}
                    onClick={() => p.onSelectConversation(c.id)}
                  >
                    <span className="rail-title">{c.title}</span>
                  </button>
                </li>
              );
            })}
          </ul>
        )}
      </Group>}

      {p.view !== "chat" && <Group title="Recursos">
        <ul className="rail-list">
          {RESOURCE_ITEMS.map((i) => {
            const active = p.view === i.view;
            return (
              <li key={i.view}>
                <button type="button" className={`rail-item nav-item${active ? " active" : ""}`} aria-current={active ? "page" : undefined} onClick={() => p.onView(i.view)}>
                  <Icon name={i.icon} />
                  <span className="rail-title">{i.label}</span>
                </button>
              </li>
            );
          })}
        </ul>
      </Group>}

      {p.view !== "chat" && p.isAdmin && (
        <Group title="Configurações">
          <ul className="rail-list">
            {CONFIG_ITEMS.map((i) => {
              const active = i.section ? p.view === "settings" && p.settingsSection === i.section : p.view === i.view;
              return (
                <li key={i.label}>
                  <button
                    type="button"
                    className={`rail-item nav-item${active ? " active" : ""}`}
                    aria-current={active ? "page" : undefined}
                    onClick={() => (i.section ? p.onSettings(i.section) : p.onView(i.view!))}
                  >
                    <Icon name={i.icon} />
                    <span className="rail-title">{i.label}</span>
                  </button>
                </li>
              );
            })}
          </ul>
        </Group>
      )}

      </div>
      <div className="rail-foot">
        <span className="muted small" title={p.email}>{p.email}</span>
        <button type="button" className="btn" onClick={p.onLogout}>Sair</button>
      </div>
    </aside>
  );
}
