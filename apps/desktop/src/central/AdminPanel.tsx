// Configurações do OneCoding (somente administradores). Cada seção tem endereço próprio (/configuracoes/<seção>),
// um botão para adicionar e a lista logo abaixo. Dados reais do OneHub; ações só onde há backend.
import { useCallback, useEffect, useState } from "react";
import type { FormEvent } from "react";
import { OneHubError } from "../transport";
import type { CentralExecutor, CentralGrant, CentralNode, CentralUser, OneHubTransport, ProviderConnection, CliConnection, OrchestratorSettings } from "../transport";

export const SETTINGS_SECTIONS = ["OneNodes", "OneExecutors", "Projetos", "Orquestrador"] as const;
export type SettingsSection = (typeof SETTINGS_SECTIONS)[number];

const SECTION_LABEL: Record<SettingsSection, string> = {
  OneNodes: "OneNodes",
  OneExecutors: "OneExecutors",
  Projetos: "Projetos",
  Orquestrador: "Orquestrador",
};

const NODE_LABEL: Record<string, string> = { online: "online", offline: "offline", pending: "aguardando pareamento", revoked: "revogado" };
const EXEC_LABEL: Record<string, string> = { online: "online", offline: "offline", pending: "aguardando pareamento", revoked: "revogado" };

/** Seção indicada pelo endereço; desconhecida ou ausente cai em OneNodes. */
export function sectionFromPath(pathname: string): SettingsSection {
  const part = pathname.replace(/^\/configuracoes\/?/i, "").split("/")[0] ?? "";
  return SETTINGS_SECTIONS.find((s) => s.toLowerCase() === decodeURIComponent(part).toLowerCase()) ?? "OneNodes";
}

export function settingsPath(section: SettingsSection): string {
  return `/configuracoes/${section}`;
}

/** Código de pareamento: exibido uma única vez, só na memória da tela. */
interface Issued {
  what: string;
  code: string;
  expires: string;
}

function errorText(e: unknown): string {
  if (e instanceof OneHubError && e.code === "forbidden") return "Ação restrita a administradores.";
  if (e instanceof OneHubError && e.code === "admin_required") return "Ação restrita a administradores.";
  if (e instanceof OneHubError && e.code === "no_grant") return "Esta concessão não existe ou já foi revogada.";
  return "Não foi possível concluir a operação. Verifique a conexão com o OneHub.";
}

export function AdminPanel({
  transport,
  user,
  section,
  onSection,
  onProvider,
}: {
  transport: OneHubTransport;
  user: CentralUser;
  section: SettingsSection;
  onSection(s: SettingsSection): void;
  onProvider(): void;
}) {
  if (user.role !== "admin") {
    return <p className="muted">Acesso restrito a administradores.</p>;
  }
  return <AdminContent transport={transport} section={section} onSection={onSection} onProvider={onProvider} />;
}

function AdminContent({ transport, section, onSection, onProvider }: { transport: OneHubTransport; section: SettingsSection; onSection(s: SettingsSection): void; onProvider(): void }) {
  const [health, setHealth] = useState<{ status: string; database: string } | "erro" | null>(null);
  const [nodes, setNodes] = useState<CentralNode[]>([]);
  const [executors, setExecutors] = useState<CentralExecutor[]>([]);
  const [projects, setProjects] = useState<{ id: string; name: string }[]>([]);
  const [grants, setGrants] = useState<CentralGrant[]>([]);
  const [issued, setIssued] = useState<Issued | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [adding, setAdding] = useState<SettingsSection | "grant" | null>(null);
  const [nodeName, setNodeName] = useState("");
  const [execName, setExecName] = useState("");
  const [projectName, setProjectName] = useState("");
  const [grantNode, setGrantNode] = useState("");
  const [grantProject, setGrantProject] = useState("");
  const [grantRoot, setGrantRoot] = useState("");

  const load = useCallback(async () => {
    const [h, n, x, p, g] = await Promise.allSettled([
      transport.health(),
      transport.nodes(),
      transport.executors(),
      transport.projects(),
      transport.grants(),
    ]);
    setHealth(h.status === "fulfilled" ? h.value : "erro");
    if (n.status === "fulfilled") setNodes(n.value);
    if (x.status === "fulfilled") setExecutors(x.value);
    if (p.status === "fulfilled") setProjects(p.value);
    if (g.status === "fulfilled") setGrants(g.value);
  }, [transport]);

  useEffect(() => {
    load();
  }, [load]);

  function go(next: SettingsSection) {
    onSection(next);
    setAdding(null);
    setError(null);
  }

  /** Executa uma ação administrativa, mostra o erro sem detalhes internos e recarrega os dados reais. */
  const act = async (fn: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    try {
      await fn();
      await load();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  const issue = async (what: string, make: () => Promise<{ pairing_code: string; pairing_expires_at: string }>, close: () => void) => {
    setBusy(true);
    setError(null);
    try {
      const r = await make();
      setIssued({ what, code: r.pairing_code, expires: new Date(r.pairing_expires_at).toLocaleTimeString("pt-BR") });
      close();
      await load();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  const revoke = (label: string, fn: () => Promise<void>) => {
    if (!window.confirm(`Revogar ${label}? A ação não pode ser desfeita.`)) return;
    void act(fn);
  };

  function submitNode(e: FormEvent) {
    e.preventDefault();
    void issue("o OneNode", () => transport.createNode(nodeName.trim()), () => { setNodeName(""); setAdding(null); });
  }

  function submitExecutor(e: FormEvent) {
    e.preventDefault();
    void issue("o OneExecutor", () => transport.createExecutor(execName.trim()), () => { setExecName(""); setAdding(null); });
  }

  function submitProject(e: FormEvent) {
    e.preventDefault();
    void act(async () => {
      await transport.createProject(projectName.trim());
      setProjectName("");
      setAdding(null);
    });
  }

  function submitGrant(e: FormEvent) {
    e.preventDefault();
    void act(async () => {
      await transport.grantProject(grantProject, grantNode, grantRoot.trim());
      setGrantNode("");
      setGrantProject("");
      setGrantRoot("");
      setAdding(null);
    });
  }

  const addButton = (kind: SettingsSection | "grant", label: string) => (
    <button
      type="button"
      className="btn primary"
      aria-expanded={adding === kind}
      onClick={() => setAdding(adding === kind ? null : kind)}
    >
      {adding === kind ? "Fechar" : label}
    </button>
  );

  return (
    <section className="admin settings" aria-label="Configurações">
      <nav className="settings-nav" aria-label="Seções de configurações">
        {SETTINGS_SECTIONS.map((s) => (
          <a
            key={s}
            href={settingsPath(s)}
            aria-current={section === s ? "page" : undefined}
            onClick={(e) => {
              e.preventDefault();
              go(s);
            }}
          >
            {SECTION_LABEL[s]}
          </a>
        ))}
      </nav>

      <div className="settings-body">
        {error && <p role="alert" className="error banner">{error}</p>}

        {issued && (
          <div className="card issued" role="status">
            <h3>Código de pareamento de {issued.what}</h3>
            <p>Use este código em até {issued.expires}. Ele é exibido somente agora e não é guardado pela interface.</p>
            <code className="pairing-code">{issued.code}</code>
            <button type="button" className="btn" onClick={() => setIssued(null)}>Entendi, ocultar</button>
          </div>
        )}

        {section === "OneNodes" && (
          <div className="card">
            <div className="settings-head">
              <h2>OneNodes</h2>
              {addButton("OneNodes", "Adicionar OneNode")}
            </div>
            {adding === "OneNodes" && (
              <form className="admin-form" onSubmit={submitNode}>
                <label>Nome do OneNode <input value={nodeName} onChange={(e) => setNodeName(e.target.value)} maxLength={120} required /></label>
                <button type="submit" className="btn primary" disabled={busy || nodeName.trim() === ""}>Gerar código do OneNode</button>
                <p className="muted small">O pareamento é feito no próprio servidor com o binário do OneNode. A instalação remota por SSH ainda não está disponível.</p>
              </form>
            )}
            {nodes.length === 0 ? <p className="muted">Nenhum OneNode cadastrado.</p> : (
              <ul className="admin-list">
                {nodes.map((n) => (
                  <li key={n.id}>
                    <span>{n.name}</span> <span className="muted">· {NODE_LABEL[n.status] ?? n.status}</span>
                    {n.status !== "revoked" && (
                      <button type="button" className="btn small" disabled={busy} onClick={() => revoke(`o OneNode ${n.name}`, () => transport.revokeNode(n.id))}>Revogar</button>
                    )}
                  </li>
                ))}
              </ul>
            )}
          </div>
        )}

        {section === "OneExecutors" && (
          <div className="card">
            <div className="settings-head">
              <h2>OneExecutors</h2>
              {addButton("OneExecutors", "Adicionar OneExecutor")}
            </div>
            <p className="muted small">Workers pareados com o OneHub que executam as tarefas autorizadas.</p>
            {adding === "OneExecutors" && (
              <form className="admin-form" onSubmit={submitExecutor}>
                <label>Nome do OneExecutor <input value={execName} onChange={(e) => setExecName(e.target.value)} maxLength={120} required /></label>
                <button type="submit" className="btn primary" disabled={busy || execName.trim() === ""}>Gerar código do OneExecutor</button>
                <p className="muted small">Só o Codex é executado pelo OneExecutor. Claude Code e Cursor CLI seguem bloqueados.</p>
              </form>
            )}
            {executors.length === 0 ? <p className="muted">Nenhum OneExecutor cadastrado.</p> : (
              <ul className="admin-list">
                {executors.map((x) => (
                  <li key={x.id}>
                    <span>{x.name}</span> <span className="muted">· {x.engine} · {EXEC_LABEL[x.status] ?? x.status}</span>
                    <ExecutorProfile transport={transport} executor={x} />
                    {x.status !== "revoked" && (
                      <button type="button" className="btn small" disabled={busy} onClick={() => revoke(`o OneExecutor ${x.name}`, () => transport.revokeExecutor(x.id))}>Revogar</button>
                    )}
                  </li>
                ))}
              </ul>
            )}
            <div className="settings-head">
              <p className="muted small">As contas CLI e conexões API são gerenciadas em Provider AI.</p>
              <button type="button" className="btn" onClick={onProvider}>Gerenciar conexões CLI e API</button>
            </div>
          </div>
        )}

        {section === "Projetos" && (
          <>
            <div className="card">
              <div className="settings-head">
                <h2>Projetos</h2>
                {addButton("Projetos", "Adicionar projeto")}
              </div>
              {adding === "Projetos" && (
                <form className="admin-form" onSubmit={submitProject}>
                  <label>Nome do projeto <input value={projectName} onChange={(e) => setProjectName(e.target.value)} maxLength={120} required /></label>
                  <button type="submit" className="btn primary" disabled={busy || projectName.trim() === ""}>Criar projeto</button>
                </form>
              )}
              {projects.length === 0 ? <p className="muted">Nenhum projeto cadastrado.</p> : (
                <ul className="admin-list">{projects.map((p) => <li key={p.id}>{p.name}</li>)}</ul>
              )}
            </div>

            <div className="card">
              <div className="settings-head">
                <h2>Concessões de acesso</h2>
                {addButton("grant", "Conceder acesso")}
              </div>
              {adding === "grant" && (
                <form className="admin-form" onSubmit={submitGrant}>
                  <label>OneNode
                    <select value={grantNode} onChange={(e) => setGrantNode(e.target.value)} required>
                      <option value="">Escolha…</option>
                      {nodes.filter((n) => n.status !== "revoked").map((n) => <option key={n.id} value={n.id}>{n.name}</option>)}
                    </select>
                  </label>
                  <label>Projeto
                    <select value={grantProject} onChange={(e) => setGrantProject(e.target.value)} required>
                      <option value="">Escolha…</option>
                      {projects.map((p) => <option key={p.id} value={p.id}>{p.name}</option>)}
                    </select>
                  </label>
                  <label>Pasta autorizada neste servidor
                    <input value={grantRoot} onChange={(e) => setGrantRoot(e.target.value)} placeholder="/opt/onenode/workspace/exemplo" required />
                  </label>
                  <button type="submit" className="btn primary" disabled={busy || !grantNode || !grantProject || !grantRoot.trim()}>Conceder acesso</button>
                </form>
              )}
              {grants.length === 0 ? <p className="muted">Nenhuma concessão ativa.</p> : (
                <ul className="admin-list">
                  {grants.map((g) => (
                    <li key={`${g.node_id}:${g.project_id}`}>
                      <span>{g.node_name} → {g.project_name}</span>
                      <button type="button" className="btn small" disabled={busy} onClick={() => revoke(`a concessão de ${g.project_name} em ${g.node_name}`, () => transport.revokeGrant(g.project_id, g.node_id))}>Revogar</button>
                    </li>
                  ))}
                </ul>
              )}
            </div>
          </>
        )}

        {section === "Orquestrador" && (
          <>
            <div className="card">
              <h2>OneOrchestrator</h2>
              <OrchestratorConfig transport={transport} />
            </div>
            <div className="card">
              <h2>OneHub Central</h2>
              {health === null && <p className="muted">Verificando…</p>}
              {health === "erro" && <p role="alert" className="error">OneHub Central indisponível.</p>}
              {health && health !== "erro" && (
                <ul className="admin-list">
                  <li>Serviço: <strong>{health.status === "ok" ? "operacional" : "degradado"}</strong></li>
                  <li>Banco de dados: <strong>{health.database === "ok" ? "conectado" : "indisponível"}</strong></li>
                </ul>
              )}
            </div>
            <div className="card">
              <h2>Ainda não disponível</h2>
              <p className="muted small">
                Descoberta de projetos e infraestrutura, instalação remota de OneNode por SSH, skills, agentes, plugins e MCPs não têm backend validado e não aparecem como ações.
              </p>
            </div>
          </>
        )}
      </div>
    </section>
  );
}

function OrchestratorConfig({ transport }: { transport: OneHubTransport }) {
  const [value, setValue] = useState<OrchestratorSettings | null>(null);
  const [providers, setProviders] = useState<ProviderConnection[]>([]);
  const [models, setModels] = useState<string[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    Promise.all([transport.orchestratorSettings(), transport.providerConnections()]).then(([settings, connections]) => {
      setValue(settings); setProviders(connections.filter((c) => c.status === "connected"));
      if (settings.provider_connection_id) void transport.providerConnectionModels(settings.provider_connection_id).then(setModels).catch(() => undefined);
    }).catch(() => setError("Não foi possível carregar as configurações do Orquestrador."));
  }, [transport]);
  if (!value) return <p className="muted">Carregando configurações…</p>;
  async function save() {
    const current = value;
    if (!current) return;
    setBusy(true); setError(null); setNotice(null);
    try { await transport.saveOrchestratorSettings(current); setNotice("Configuração salva. O runtime permanece indisponível e o Orquestrador desativado."); }
    catch { setError("Não foi possível salvar. Confirme se a conexão e o modelo estão válidos."); }
    finally { setBusy(false); }
  }
  const selected = providers.find((p) => p.id === value.provider_connection_id);
  return <div className="admin-form">
    <p className="muted small">Selecione uma conexão API verificada e configure os prompts. O runtime ainda não executa tarefas, portanto não pode ser ativado.</p>
    <label>Conexão API<select value={value.provider_connection_id ?? ""} onChange={(e) => { setValue({ ...value, provider_connection_id: e.target.value || null, model: null }); setModels([]); }}><option value="">Sem conexão</option>{providers.map((p) => <option key={p.id} value={p.id}>{p.name} · {p.provider}</option>)}</select></label>
    <label>Modelo<select value={value.model ?? ""} disabled={!selected || !models.length} onChange={(e) => setValue({ ...value, model: e.target.value || null })}><option value="">Selecione…</option>{models.map((m) => <option key={m}>{m}</option>)}</select></label>
    {selected && !models.length && <button type="button" className="btn" disabled={busy} onClick={() => { setBusy(true); void transport.providerConnectionModels(selected.id).then(setModels).catch(() => setError("Falha ao consultar modelos disponíveis.")).finally(() => setBusy(false)); }}>Atualizar modelos</button>}
    <label>Prompt de sistema<textarea rows={5} maxLength={16384} value={value.system_prompt} onChange={(e) => setValue({ ...value, system_prompt: e.target.value })} /></label>
    <label>Instruções<textarea rows={4} maxLength={16384} value={value.instructions} onChange={(e) => setValue({ ...value, instructions: e.target.value })} /></label>
    {error && <p role="alert" className="error">{error}</p>}{notice && <p role="status" className="muted small">{notice}</p>}
    <button type="button" className="btn primary" disabled={busy} onClick={() => void save()}>Salvar configuração</button>
  </div>;
}

function ExecutorProfile({ transport, executor }: { transport: OneHubTransport; executor: CentralExecutor }) {
  const [open, setOpen] = useState(false);
  const [accounts, setAccounts] = useState<CliConnection[]>([]);
  const [selected, setSelected] = useState("");
  const [instructions, setInstructions] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  async function toggle() {
    setOpen((v) => !v);
    if (open || accounts.length) return;
    setBusy(true);
    try {
      const [all, current] = await Promise.all([transport.cliConnections(), transport.executorSettings(executor.id)]);
      setAccounts(all.filter((c) => c.executor_id === executor.id && c.engine === "codex" && c.auth_state === "authenticated" && c.usage_authorized));
      setSelected(current.cli_connection_id ?? ""); setInstructions(current.instructions);
    } catch { setError("Não foi possível carregar o perfil deste OneExecutor."); }
    finally { setBusy(false); }
  }
  async function save() {
    setBusy(true); setError(null);
    try { await transport.saveExecutorSettings(executor.id, { cli_connection_id: selected, instructions }); }
    catch { setError("A conta deve estar autenticada, autorizada e associada a este OneExecutor online."); }
    finally { setBusy(false); }
  }
  return <div className="executor-profile"><button type="button" className="btn small" onClick={() => void toggle()} aria-expanded={open}>Configurar perfil</button>
    {open && <div className="admin-form"><p className="muted small">Somente Codex autenticado neste executor. A configuração ainda não está ligada ao runtime de tarefas.</p>
      <label>Conta Codex<select value={selected} onChange={(e) => setSelected(e.target.value)} disabled={busy}><option value="">Selecione…</option>{accounts.map((c) => <option key={c.id} value={c.id}>{c.name}</option>)}</select></label>
      <label>Instruções específicas<textarea rows={3} maxLength={16384} value={instructions} onChange={(e) => setInstructions(e.target.value)} /></label>
      {error && <p role="alert" className="error">{error}</p>}<button type="button" className="btn primary" disabled={busy || !selected} onClick={() => void save()}>Salvar perfil</button>
    </div>}
  </div>;
}
