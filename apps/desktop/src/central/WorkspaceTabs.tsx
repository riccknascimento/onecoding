import { useState } from "react";
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
  const opened = new Set(openTabs);
  return (
    <nav className="workspace-tabs" aria-label="Abas do OneCoding">
      <button type="button" className={`workspace-tab settings-tab${activeTab === "settings" ? " active" : ""}`} onClick={() => onActivate("settings")}>
        <Icon name="settings" /><span>Settings</span>
      </button>
      {openTabs.filter((id) => id !== "settings").map((id) => {
        const workspace = workspaces.find((item) => item.id === id);
        if (!workspace) return null;
        return (
          <div className={`workspace-tab-wrap${activeTab === id ? " active" : ""}`} key={id}>
            <button type="button" className="workspace-tab" onClick={() => onActivate(id)} title={workspace.name}>
              <Icon name="projects" /><span>{workspace.name}</span>
            </button>
            <button type="button" className="workspace-tab-close" aria-label={`Renomear ${workspace.name}`} title="Renomear Workspace" onClick={() => { const next = window.prompt("Nome do Workspace", workspace.name)?.trim(); if (next && next !== workspace.name) void onRename(id, next); }}>✎</button>
            <button type="button" className="workspace-tab-close" aria-label={`Fechar ${workspace.name}`} onClick={() => onClose(id)}>×</button>
          </div>
        );
      })}
      <div className="workspace-tab-add-wrap">
        <button type="button" className="workspace-tab-add" aria-label="Abrir ou criar Workspace" aria-expanded={menu} onClick={() => setMenu((value) => !value)}><Icon name="plus" /></button>
        {menu && <div className="workspace-tab-menu" role="dialog" aria-label="Abrir Workspace">
          <strong>Workspaces</strong>
          {workspaces.map((workspace) => <button type="button" key={workspace.id} disabled={opened.has(workspace.id)} onClick={() => { onOpen(workspace.id); setMenu(false); }}>
            {workspace.name}{opened.has(workspace.id) ? " · aberto" : ""}
          </button>)}
          {workspaces.length === 0 && <span className="muted small">Nenhum Workspace cadastrado.</span>}
          <form onSubmit={(event) => { event.preventDefault(); const value = name.trim(); if (!value) return; void onCreate(value).then(() => { setName(""); setMenu(false); }); }}>
            <input aria-label="Nome do novo Workspace" value={name} onChange={(event) => setName(event.target.value)} placeholder="Novo Workspace" maxLength={120} />
            <button className="btn primary" type="submit" disabled={!name.trim()}>Criar</button>
          </form>
        </div>}
      </div>
    </nav>
  );
}
