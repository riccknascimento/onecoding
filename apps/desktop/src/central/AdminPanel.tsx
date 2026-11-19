// Configurações do OneCoding (somente administradores). Dados reais do OneHub; ações só onde há backend.
import { useCallback, useEffect, useState } from "react";
import { OneHubError } from "../transport";
import type { CentralExecutor, CentralGrant, CentralNode, CentralUser, OneHubTransport } from "../transport";

const NODE_LABEL: Record<string, string> = { online: "online", offline: "offline", pending: "aguardando pareamento", revoked: "revogado" };
const EXEC_LABEL: Record<string, string> = { online: "online", offline: "offline", pending: "aguardando pareamento", revoked: "revogado" };

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

export function AdminPanel({ transport, user }: { transport: OneHubTransport; user: CentralUser }) {
  if (user.role !== "admin") {
    return <p className="muted">Acesso restrito a administradores.</p>;
  }
  return <AdminContent transport={transport} />;
}

function AdminContent({ transport }: { transport: OneHubTransport }) {
  const [health, setHealth] = useState<{ status: string; database: string } | "erro" | null>(null);
  const [nodes, setNodes] = useState<CentralNode[]>([]);
  const [executors, setExecutors] = useState<CentralExecutor[]>([]);
  const [projects, setProjects] = useState<{ id: string; name: string }[]>([]);
  const [grants, setGrants] = useState<CentralGrant[]>([]);
  const [issued, setIssued] = useState<Issued | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
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

  const issue = async (what: string, make: () => Promise<{ pairing_code: string; pairing_expires_at: string }>) => {
    setBusy(true);
    setError(null);
    try {
      const r = await make();
      setIssued({ what, code: r.pairing_code, expires: new Date(r.pairing_expires_at).toLocaleTimeString("pt-BR") });
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

  return (
    <section className="admin" aria-label="Configurações">
      <h2>Configurações</h2>
      {error && <p role="alert" className="error banner">{error}</p>}

      <div className="card">
        <h3>OneHub Central</h3>
        {health === null && <p className="muted">Verificando…</p>}
        {health === "erro" && <p role="alert" className="error">OneHub Central indisponível.</p>}
        {health && health !== "erro" && (
          <ul className="admin-list">
            <li>Serviço: <strong>{health.status === "ok" ? "operacional" : "degradado"}</strong></li>
            <li>Banco de dados: <strong>{health.database === "ok" ? "conectado" : "indisponível"}</strong></li>
          </ul>
        )}
      </div>

      {issued && (
        <div className="card issued" role="status">
          <h3>Código de pareamento de {issued.what}</h3>
          <p>Use este código em até {issued.expires}. Ele é exibido somente agora e não é guardado pela interface.</p>
          <code className="pairing-code">{issued.code}</code>
          <button type="button" className="btn" onClick={() => setIssued(null)}>Entendi, ocultar</button>
        </div>
      )}

      <div className="card">
        <h3>OneNodes</h3>
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
        <form className="admin-form" onSubmit={(e) => { e.preventDefault(); void issue("o OneNode", () => transport.createNode(nodeName.trim()).then((r) => { setNodeName(""); return r; })); }}>
          <label>Nome do OneNode <input value={nodeName} onChange={(e) => setNodeName(e.target.value)} maxLength={120} required /></label>
          <button type="submit" className="btn primary" disabled={busy || nodeName.trim() === ""}>Gerar código do OneNode</button>
        </form>
        <p className="muted small">O pareamento é feito no próprio servidor com o binário do OneNode. A instalação remota por SSH ainda não está disponível.</p>
      </div>

      <div className="card">
        <h3>OneExecutors</h3>
        {executors.length === 0 ? <p className="muted">Nenhum OneExecutor cadastrado.</p> : (
          <ul className="admin-list">
            {executors.map((x) => (
              <li key={x.id}>
                <span>{x.name}</span> <span className="muted">· {x.engine} · {EXEC_LABEL[x.status] ?? x.status}</span>
                {x.status !== "revoked" && (
                  <button type="button" className="btn small" disabled={busy} onClick={() => revoke(`o OneExecutor ${x.name}`, () => transport.revokeExecutor(x.id))}>Revogar</button>
                )}
              </li>
            ))}
          </ul>
        )}
        <form className="admin-form" onSubmit={(e) => { e.preventDefault(); void issue("o OneExecutor", () => transport.createExecutor(execName.trim()).then((r) => { setExecName(""); return r; })); }}>
          <label>Nome do OneExecutor <input value={execName} onChange={(e) => setExecName(e.target.value)} maxLength={120} required /></label>
          <button type="submit" className="btn primary" disabled={busy || execName.trim() === ""}>Gerar código do OneExecutor</button>
        </form>
        <p className="muted small">Só o Codex é executado pelo OneExecutor. Claude Code e Cursor CLI seguem bloqueados.</p>
      </div>

      <div className="card">
        <h3>Projetos e concessões</h3>
        {projects.length === 0 ? <p className="muted">Nenhum projeto cadastrado.</p> : (
          <ul className="admin-list">{projects.map((p) => <li key={p.id}>{p.name}</li>)}</ul>
        )}
        <form className="admin-form" onSubmit={(e) => { e.preventDefault(); void act(async () => { await transport.createProject(projectName.trim()); setProjectName(""); }); }}>
          <label>Novo projeto <input value={projectName} onChange={(e) => setProjectName(e.target.value)} maxLength={120} required /></label>
          <button type="submit" className="btn" disabled={busy || projectName.trim() === ""}>Criar projeto</button>
        </form>

        <h4>Concessões ativas (OneNode × projeto)</h4>
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
        <form className="admin-form" onSubmit={(e) => { e.preventDefault(); void act(() => transport.grantProject(grantProject, grantNode, grantRoot.trim())); }}>
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
      </div>

      <div className="card">
        <h3>Ainda não disponível</h3>
        <p className="muted small">
          Estes recursos não têm backend validado e por isso não aparecem como ações: descoberta de projetos e infraestrutura, instalação remota de OneNode por SSH, contas das CLIs, skills, agentes, plugins e MCPs, e o OneOrchestrator com ChatGPT.
        </p>
      </div>
    </section>
  );
}
