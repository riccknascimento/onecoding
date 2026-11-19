import { useEffect, useRef, useState } from "react";
import type { CentralWorkspace } from "../transport";
import { Icon } from "../icons";

export type OpenTab = "settings" | string;

export function WorkspaceTabs({
  workspaces,
  openTabs,
  activeTab,
  onActivate,
  onOpen,
  onClose,
  onCreate,
  onRename,
}: {
  workspaces: CentralWorkspace[];
  openTabs: OpenTab[];
  activeTab: OpenTab;
  onActivate(id: OpenTab): void;
  onOpen(id: string): void;
  onClose(id: string): void;
  onCreate(name: string): Promise<void>;
  onRename(id: string, name: string): Promise<void>;
}) {
  const [menu, setMenu] = useState(false);
  const [name, setName] = useState("");
  const [editing, setEditing] = useState<CentralWorkspace | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  const picker = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (!menu) return;
    setError("");
    picker.current?.querySelector("input")?.focus();
    const close = (event: PointerEvent) => {
      if (!picker.current?.contains(event.target as Node)) setMenu(false);
    };
    const escape = (event: KeyboardEvent) => {
      if (event.key === "Escape") { setMenu(false); trigger.current?.focus(); }
    };
    document.addEventListener("pointerdown", close);
    document.addEventListener("keydown", escape);
    return () => {
      document.removeEventListener("pointerdown", close);
      document.removeEventListener("keydown", escape);
    };
  }, [menu]);
  async function save(event: React.FormEvent) {
    event.preventDefault();
    const value = name.trim();
    if (!value || saving) return;
    setSaving(true);
    setError("");
    try {
      if (editing) await onRename(editing.id, value);
      else await onCreate(value);
      setName("");
      setMenu(false);
      trigger.current?.focus();
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "Não foi possível salvar o Workspace.");
    } finally {
      setSaving(false);
    }
  }
  const opened = new Set(openTabs);
  return (
    <nav className="workspace-tabs" aria-label="Abas do OneCoding">
      <button type="button" className={`workspace-tab settings-tab${activeTab === "settings" ? " active" : ""}`} aria-current={activeTab === "settings" ? "page" : undefined} onClick={() => onActivate("settings")}>
        <Icon name="settings" /><span>Settings</span>
      </button>
      <div className="workspace-track">
      {openTabs.filter((id) => id !== "settings").map((id) => {
        const workspace = workspaces.find((item) => item.id === id);
        if (!workspace) return null;
        return (
          <div className={`workspace-tab-wrap${activeTab === id ? " active" : ""}`} key={id}>
            <button type="button" className="workspace-tab" aria-current={activeTab === id ? "page" : undefined} onClick={() => onActivate(id)} title={workspace.name}>
              <Icon name="projects" /><span>{workspace.name}</span>
            </button>
            <button type="button" className="workspace-tab-close" aria-label={`Renomear ${workspace.name}`} title="Renomear Workspace" onClick={() => { setEditing(workspace); setName(workspace.name); setMenu(true); }}><Icon name="edit" /></button>
            <button type="button" className="workspace-tab-close" aria-label={`Fechar ${workspace.name}`} onClick={() => onClose(id)}><Icon name="close" /></button>
          </div>
        );
      })}
      </div>
      <div ref={picker} className="workspace-tab-add-wrap">
        <button type="button" ref={trigger} className="workspace-tab-add" aria-label="Abrir ou criar Workspace" aria-expanded={menu} onClick={() => { setEditing(null); setName(""); setMenu(!menu); }}><Icon name="plus" /></button>
        {menu && <div className="workspace-tab-menu" role="dialog" aria-label={editing ? "Renomear Workspace" : "Abrir Workspace"}>
          <strong>{editing ? "Renomear Workspace" : "Workspaces"}</strong>
          {!editing && workspaces.map((workspace) => <button type="button" key={workspace.id} disabled={opened.has(workspace.id)} onClick={() => { onOpen(workspace.id); setMenu(false); }}>
            {workspace.name}{opened.has(workspace.id) ? " · aberto" : ""}
          </button>)}
          {!editing && workspaces.length === 0 && <span className="muted small">Nenhum Workspace cadastrado.</span>}
          {error && <p role="alert" className="error">{error}</p>}
          <form onSubmit={save}>
            <input aria-label={editing ? "Novo nome" : "Nome do novo Workspace"} value={name} onChange={(event) => setName(event.target.value)} placeholder="Novo Workspace" maxLength={120} />
            <button className="btn primary" type="submit" disabled={saving || !name.trim()}>{editing ? "Salvar" : "Criar"}</button>
          </form>
        </div>}
      </div>
    </nav>
  );
}
