// Navegação principal do OneCoding Web. Módulos sem backend aparecem como indisponíveis dentro da própria página.
import type { CentralConversation } from "../transport";
import { Icon } from "../icons";
import type { IconName } from "../icons";

export type CentralView = "chat" | "onenodes" | "provider" | "projects" | "context" | "skills" | "executions" | "settings";

interface NavItem {
  view: CentralView;
  label: string;
  icon: IconName;
  adminOnly?: boolean;
}

/** Ordem definida para o OneCoding Web. */
export const NAV_ITEMS: NavItem[] = [
  { view: "chat", label: "Chat / Orquestrador", icon: "chat" },
  { view: "onenodes", label: "OneNodes", icon: "nodes", adminOnly: true },
  { view: "provider", label: "One Provider AI", icon: "gateway", adminOnly: true },
  { view: "projects", label: "Projetos", icon: "projects" },
  { view: "context", label: "OneContext", icon: "book" },
  { view: "skills", label: "Skills & Agents", icon: "bot" },
  { view: "executions", label: "Execuções", icon: "list" },
  { view: "settings", label: "Configurações", icon: "settings", adminOnly: true },
];

interface SidebarProps {
  view: CentralView;
  isAdmin: boolean;
  email: string;
  conversations: CentralConversation[];
  activeId: string | null;
  open: boolean;
  onView(v: CentralView): void;
  onSelectConversation(id: string): void;
  onNewConversation(): void;
  onLogout(): void;
}

export function Sidebar(p: SidebarProps) {
  const items = NAV_ITEMS.filter((i) => !i.adminOnly || p.isAdmin);
  return (
    <aside className={`rail${p.open ? " open" : ""}`} aria-label="Navegação principal">
      <div className="rail-head">
        <span className="brand">OneCoding</span>
      </div>
      <nav aria-label="Módulos">
        <ul className="nav-list">
          {items.map((i) => {
            const active = p.view === i.view;
            return (
              <li key={i.view}>
                <button
                  type="button"
                  className={`rail-item nav-item${active ? " active" : ""}`}
                  aria-current={active ? "page" : undefined}
                  onClick={() => p.onView(i.view)}
                >
                  <Icon name={i.icon} />
                  <span className="rail-title">{i.label}</span>
                </button>
              </li>
            );
          })}
        </ul>
      </nav>
      <section className="rail-section" aria-label="Conversas">
        <div className="rail-section-head">
          <h3>Conversas</h3>
          <button type="button" className="icon-btn" onClick={p.onNewConversation} aria-label="Nova conversa" title="Nova conversa">
            <Icon name="plus" />
          </button>
        </div>
        {p.conversations.length === 0 ? (
          <p className="rail-empty">Nenhuma conversa ainda. Escreva no chat para começar.</p>
        ) : (
          <ul>
            {p.conversations.map((c) => (
              <li key={c.id}>
                <button
                  type="button"
                  className={`rail-item conv-item${c.id === p.activeId && p.view === "chat" ? " active" : ""}`}
                  aria-current={c.id === p.activeId && p.view === "chat" ? "true" : undefined}
                  onClick={() => p.onSelectConversation(c.id)}
                >
                  <span className="rail-title">{c.title}</span>
                </button>
              </li>
            ))}
          </ul>
        )}
      </section>
      <div className="rail-foot">
        <span className="muted small" title={p.email}>{p.email}</span>
        <button type="button" className="btn" onClick={p.onLogout}>Sair</button>
      </div>
    </aside>
  );
}
