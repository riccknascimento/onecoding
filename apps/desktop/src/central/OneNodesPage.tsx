// OneNodes (somente administradores): cadastro, estado real, projetos e concessões pelo OneHub.
// Tudo o que aparece é dado do OneHub ou de uma ação com backend. Instalação por SSH ainda não existe.
import { useCallback, useEffect, useState } from "react";
import { OneHubError } from "../transport";
import type { CentralNode, CentralNodeDetail, OneHubTransport } from "../transport";

const ERROR_LABEL: Record<string, string> = {
  root_path_invalid: "Caminho inválido: informe um caminho absoluto, sem “..”.",
  invalid_name: "Nome inválido: use de 1 a 120 caracteres.",
  admin_required: "Ação restrita a administradores.",
  forbidden: "Ação restrita a administradores.",
  unauthorized: "Sua sessão terminou. Entre de novo.",
  not_found: "OneNode ou projeto não encontrado (pode já ter sido revogado).",
  internal: "O OneHub retornou erro interno.",
};

function errorText(e: unknown): string {
  if (e instanceof OneHubError) return ERROR_LABEL[e.code] ?? `Não foi possível concluir (${e.code}).`;
  return "Não foi possível falar com o OneHub. Verifique a conexão.";
}

/** Estado exibido ao administrador, a partir dos dados reais do OneHub. */
export function nodeStateLabel(n: { status: string; suspended?: boolean }): { text: string; tone: "ok" | "warn" | "bad" | "info" } {
  if (n.status === "revoked") return { text: "Revogado", tone: "bad" };
  if (n.suspended) return { text: "Suspenso", tone: "warn" };
  if (n.status === "online") return { text: "Online", tone: "ok" };
  if (n.status === "offline") return { text: "Offline", tone: "bad" };
  return { text: "Aguardando pareamento", tone: "info" };
}

function when(iso: string | null): string {
  if (!iso) return "nunca";
  return new Date(iso).toLocaleString("pt-BR", { dateStyle: "short", timeStyle: "short" });
}

type Confirm = { kind: "revoke-node" } | { kind: "revoke-grant"; projectId: string } | null;

export function OneNodesPage({ transport }: { transport: OneHubTransport }) {
  const [nodes, setNodes] = useState<CentralNode[]>([]);
  const [projects, setProjects] = useState<{ id: string; name: string }[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [detail, setDetail] = useState<CentralNodeDetail | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [newName, setNewName] = useState("");
  const [issued, setIssued] = useState<{ name: string; code: string; expires: string } | null>(null);
  const [projectId, setProjectId] = useState("");
  const [newProjectName, setNewProjectName] = useState("");
  const [rootPath, setRootPath] = useState("");
  const [confirm, setConfirm] = useState<Confirm>(null);

  const loadList = useCallback(async () => {
    try {
      const [n, p] = await Promise.all([transport.nodes(), transport.projects()]);
      setNodes(n);
      setProjects(p);
      setLoadError(null);
    } catch (e) {
      setLoadError(errorText(e));
    }
  }, [transport]);

  const loadDetail = useCallback(
    async (id: string) => {
      try {
        setDetail(await transport.nodeDetail(id));
        setActionError(null);
      } catch (e) {
        setDetail(null);
        setActionError(errorText(e));
      }
    },
    [transport],
  );

  useEffect(() => {
    void loadList();
    const timer = setInterval(() => void loadList(), 20000);
    return () => clearInterval(timer);
  }, [loadList]);

  useEffect(() => {
    if (selectedId) void loadDetail(selectedId);
    else setDetail(null);
  }, [selectedId, loadDetail]);

  async function run(fn: () => Promise<void>) {
    setBusy(true);
    setActionError(null);
    try {
      await fn();
    } catch (e) {
      setActionError(errorText(e));
    } finally {
      setBusy(false);
    }
  }

  async function refreshAll() {
    await loadList();
    if (selectedId) await loadDetail(selectedId);
  }

  function createNode() {
    const name = newName.trim();
    void run(async () => {
      const res = await transport.createNode(name);
      setIssued({ name, code: res.pairing_code, expires: res.pairing_expires_at });
      setNewName("");
      await refreshAll();
    });
  }

  function suspendOrResume() {
    if (!detail) return;
    const id = detail.id;
    const suspend = !detail.suspended;
    void run(async () => {
      if (suspend) await transport.suspendNode(id);
      else await transport.resumeNode(id);
      await refreshAll();
    });
  }

  function revokeNode() {
    if (!detail) return;
    const id = detail.id;
    void run(async () => {
      await transport.revokeNode(id);
      setConfirm(null);
      setSelectedId(null);
      await loadList();
    });
  }

  function grant() {
    if (!detail) return;
    const nodeId = detail.id;
    const root = rootPath.trim();
    const name = newProjectName.trim();
    void run(async () => {
      let pid = projectId;
      if (!pid) {
        if (!name) throw new OneHubError(400, "invalid_name");
        const created = await transport.createProject(name);
        pid = created.id;
      }
      await transport.grantProject(pid, nodeId, root);
      setProjectId("");
      setNewProjectName("");
      setRootPath("");
      await refreshAll();
    });
  }

  function revokeGrant(projectIdToRevoke: string) {
    if (!detail) return;
    const nodeId = detail.id;
    void run(async () => {
      await transport.revokeGrant(projectIdToRevoke, nodeId);
      setConfirm(null);
      await refreshAll();
    });
  }

  const selected = nodes.find((n) => n.id === selectedId) ?? null;

  return (
    <section className="page nodes-page" aria-label="OneNodes">
      <h1 className="page-title">OneNodes</h1>
      <div className="nodes-layout">
      <div className="nodes-side">
      {loadError && <p role="alert" className="error">{loadError}</p>}

      <div className="card">
        <h3>Novo OneNode</h3>
        <p className="muted small">
          Cadastre o servidor e gere um código de pareamento de uso único. A instalação automática por SSH ainda não está
          disponível nesta versão: o OneNode é instalado no servidor e pareado com o código.
        </p>
        <form
          className="admin-form"
          onSubmit={(e) => {
            e.preventDefault();
            createNode();
          }}
        >
          <label>
            Nome do servidor
            <input value={newName} onChange={(e) => setNewName(e.target.value)} maxLength={120} required />
          </label>
          <button type="submit" className="btn" disabled={busy || !newName.trim()}>
            Gerar código de pareamento
          </button>
        </form>
        {issued && (
          <div className="issued card" role="status">
            <strong>Código de {issued.name}</strong> (exibido uma única vez):
            <p className="pairing-code">{issued.code}</p>
            <p className="muted small">Expira em {when(issued.expires)}. Não guarde este código em outro lugar.</p>
          </div>
        )}
      </div>

      <div className="card">
        <div className="run-head">
          <h3>Servidores</h3>
          <button type="button" className="btn small" onClick={() => void refreshAll()} disabled={busy}>
            Atualizar
          </button>
        </div>
        {nodes.length === 0 ? (
          <p className="muted">Nenhum OneNode cadastrado.</p>
        ) : (
          <ul className="admin-list node-list" aria-label="Lista de OneNodes">
            {nodes.map((n) => {
              const st = nodeStateLabel(n);
              return (
                <li key={n.id}>
                  <button
                    type="button"
                    className={`btn small${selectedId === n.id ? " active" : ""}`}
                    aria-pressed={selectedId === n.id}
                    onClick={() => setSelectedId(n.id)}
                  >
                    {n.name}
                  </button>
                  <span className={`status-pill ${st.tone}`}>{st.text}</span>
                  <span className="muted small">último sinal: {when(n.last_seen_at)}</span>
                  {n.catalog_in_sync === false && <span className="muted small">· configuração ainda não aplicada</span>}
                </li>
              );
            })}
          </ul>
        )}
      </div>

      </div>
      <div className="nodes-main">
      {!selected && <div className="empty-card">Selecione um OneNode para ver o estado, a sincronização, os projetos e as ações administrativas.</div>}
      {selected && !detail && !actionError && <p className="muted">Carregando {selected.name}…</p>}
      {actionError && <p role="alert" className="error">{actionError}</p>}

      {detail && (
        <div className="card" aria-label={`Detalhes de ${detail.name}`}>
          <h3>{detail.name}</h3>
          <ul className="admin-list small">
            <li>Estado: <strong>{nodeStateLabel(detail).text}</strong></li>
            <li>Último sinal (heartbeat): {when(detail.last_seen_at)}</li>
            <li>Pareado em: {when(detail.paired_at)}</li>
            <li>Acesso de rede: {detail.access_type === "tailscale" ? "Tailscale" : "SSH público ou direto"}</li>
            <li>
              Catálogo: versão {detail.catalog_version}
              {detail.catalog_in_sync
                ? ` · aplicada pelo OneNode em ${when(detail.applied_at)}`
                : detail.applied_version === null
                  ? " · ainda não aplicada pelo OneNode"
                  : ` · aplicada: versão ${detail.applied_version} (aguardando a nova)`}
            </li>
          </ul>

          {detail.applied_report?.rejected && detail.applied_report.rejected.length > 0 && (
            <div role="status" className="notice">
              <strong>Raízes recusadas pela política local deste servidor:</strong>
              <ul>
                {detail.applied_report.rejected.map((r) => {
                  const p = projects.find((x) => x.id === r.project_id);
                  return <li key={r.project_id}>{p?.name ?? "Projeto"}: {r.reason}</li>;
                })}
              </ul>
            </div>
          )}

          <h4>Projetos com acesso</h4>
          {detail.grants.length === 0 ? (
            <p className="muted small">Nenhum projeto. Um OneNode sem projetos continua Online.</p>
          ) : (
            <ul className="admin-list">
              {detail.grants.map((g) => (
                <li key={g.project_id}>
                  <strong>{g.project_name}</strong> <span className="muted small">{g.root_path ?? "sem caminho"}</span>
                  {confirm?.kind === "revoke-grant" && confirm.projectId === g.project_id ? (
                    <>
                      <span className="muted small">Revogar o acesso deste projeto?</span>
                      <button type="button" className="btn small" onClick={() => revokeGrant(g.project_id)} disabled={busy}>
                        Confirmar revogação
                      </button>
                      <button type="button" className="btn small" onClick={() => setConfirm(null)}>Cancelar</button>
                    </>
                  ) : (
                    <button type="button" className="btn small" onClick={() => setConfirm({ kind: "revoke-grant", projectId: g.project_id })} disabled={busy}>
                      Revogar acesso
                    </button>
                  )}
                </li>
              ))}
            </ul>
          )}

          <h4>Conceder acesso a um projeto</h4>
          <form
            className="admin-form"
            onSubmit={(e) => {
              e.preventDefault();
              grant();
            }}
          >
            <label>
              Projeto existente
              <select value={projectId} onChange={(e) => setProjectId(e.target.value)}>
                <option value="">— criar novo —</option>
                {projects.map((p) => (
                  <option key={p.id} value={p.id}>{p.name}</option>
                ))}
              </select>
            </label>
            {!projectId && (
              <label>
                Nome do novo projeto
                <input value={newProjectName} onChange={(e) => setNewProjectName(e.target.value)} maxLength={120} />
              </label>
            )}
            <label>
              Pasta autorizada neste servidor
              <input value={rootPath} onChange={(e) => setRootPath(e.target.value)} placeholder="/opt/onenode/workspace/exemplo" required />
            </label>
            <button type="submit" className="btn" disabled={busy || !rootPath.trim() || (!projectId && !newProjectName.trim())}>
              Conceder acesso
            </button>
          </form>
          <p className="muted small">
            A pasta precisa estar dentro de uma pasta autorizada pela política local do servidor. Conceder acesso no OneHub não amplia
            essa política.
          </p>

          <h4>Ações do OneNode</h4>
          <div className="admin-form">
            <button type="button" className="btn" onClick={suspendOrResume} disabled={busy}>
              {detail.suspended ? "Retomar OneNode" : "Suspender OneNode"}
            </button>
            {confirm?.kind === "revoke-node" ? (
              <>
                <span className="muted small">
                  Revogar encerra o acesso deste servidor ao OneHub. O serviço continua no servidor até ser removido manualmente.
                </span>
                <button type="button" className="btn" onClick={revokeNode} disabled={busy}>Confirmar revogação</button>
                <button type="button" className="btn" onClick={() => setConfirm(null)}>Cancelar</button>
              </>
            ) : (
              <button type="button" className="btn" onClick={() => setConfirm({ kind: "revoke-node" })} disabled={busy}>
                Revogar OneNode
              </button>
            )}
          </div>
          <p className="muted small">
            Suspender é reversível e bloqueia novas operações. Revogar é definitivo. Desinstalar do servidor não é oferecido nesta versão.
          </p>
        </div>
      )}
      </div>
      </div>
    </section>
  );
}
