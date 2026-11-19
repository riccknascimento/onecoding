// One Provider AI: contas de CLI (Claude Code, Codex, Cursor) e conexões por API Key.
// Todo estado vem do OneHub. O login oficial roda no OneExecutor; a página só mostra o link, o código e o progresso.
import { useCallback, useEffect, useState } from "react";
import type { FormEvent } from "react";
import { Icon } from "../icons";
import { OneHubError } from "../transport";
import type {
  ApiProvider,
  CentralExecutor,
  CliConnection,
  CliEngine,
  CliSessionView,
  OneHubTransport,
  ProviderConnection,
} from "../transport";
import { PageHeader } from "./pages";

const ENGINES: { id: CliEngine; label: string }[] = [
  { id: "claude", label: "Claude Code" },
  { id: "codex", label: "Codex CLI" },
  { id: "cursor", label: "Cursor CLI" },
];

const PROVIDERS: { id: ApiProvider; label: string }[] = [
  { id: "openrouter", label: "OpenRouter" },
  { id: "openai", label: "OpenAI" },
  { id: "gemini", label: "Google Gemini" },
  { id: "anthropic", label: "Anthropic" },
];

const ACTIVE = ["pending", "running", "awaiting_code"];

/** Mensagens em português para os códigos do OneHub. Erro desconhecido nunca mostra detalhe técnico. */
const ERROR_TEXT: Record<string, string> = {
  executor_offline: "O OneExecutor desta conta está offline. Verifique o servidor e tente de novo.",
  session_in_progress: "Já existe uma operação em andamento nesta conta.",
  usage_not_authorized: "Autorize o uso desta conta antes de continuar.",
  name_in_use: "Já existe uma conta com este nome neste OneExecutor.",
  logout_first: "Desconecte a conta antes de removê-la.",
  not_awaiting_code: "A CLI não está aguardando um código agora.",
  invalid_code: "O código colado não tem um formato válido.",
  invalid_name: "Informe um nome entre 1 e 120 caracteres.",
  invalid_api_key: "A chave de API não tem um formato válido.",
  invalid_provider: "Escolha um provedor da lista.",
  provider_key_missing: "A proteção das chaves não está configurada neste OneHub. Peça ao administrador.",
  forbidden: "Somente administradores podem gerenciar provedores.",
  not_found: "Item não encontrado. Atualize a página.",
};

export function errorMessage(e: unknown): string {
  if (e instanceof OneHubError) {
    if (e.code === "unauthorized") return "Sua sessão terminou. Entre de novo.";
    return ERROR_TEXT[e.code] ?? "Não foi possível concluir a operação. Tente de novo.";
  }
  return "Não foi possível falar com o OneHub. Verifique a conexão.";
}

/** Estado da conta para o usuário: prioriza a operação em andamento sobre o último estado verificado. */
export function accountState(c: CliConnection): { text: string; tone: "ok" | "warn" | "bad" | "info" } {
  if (c.session && ACTIVE.includes(c.session.state)) {
    return c.session.state === "awaiting_code"
      ? { text: "Aguardando o código", tone: "info" }
      : { text: "Login em andamento", tone: "info" };
  }
  if (c.auth_state === "authenticated") return { text: "Conectada", tone: "ok" };
  if (c.auth_state === "unauthenticated") return { text: "Desconectada", tone: "bad" };
  return { text: "Não verificada", tone: "warn" };
}

function Session({ session, onCancel, onCode }: { session: CliSessionView; onCancel(): void; onCode(code: string): Promise<void> }) {
  const [code, setCode] = useState("");
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const label = session.action === "login" ? "Conexão" : session.action === "status" ? "Verificação" : "Desconexão";

  async function send(e: FormEvent) {
    e.preventDefault();
    setBusy(true);
    setErr(null);
    try {
      await onCode(code.trim());
      setCode("");
    } catch (error) {
      setErr(errorMessage(error));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="prov-session" role="status" aria-live="polite">
      <p className="prov-session-title">{label} em andamento</p>
      {session.url && (
        <p>
          <a className="btn" href={session.url} target="_blank" rel="noopener noreferrer">Abrir página oficial de autorização</a>
        </p>
      )}
      {session.device_code && (
        <p className="prov-code">
          <span className="muted small">Digite este código na página oficial:</span>
          <code aria-label="Código de autorização">{session.device_code}</code>
        </p>
      )}
      {session.awaiting_code && (
        <form className="prov-code-form" onSubmit={send} aria-label="Código da autorização">
          <label>
            <span className="muted small">Cole aqui o código mostrado na página oficial</span>
            <input value={code} onChange={(e) => setCode(e.target.value)} autoComplete="off" spellCheck={false} />
          </label>
          <button type="submit" className="btn" disabled={busy || code.trim() === ""}>Enviar código</button>
        </form>
      )}
      {err && <p role="alert" className="error">{err}</p>}
      <button type="button" className="btn ghost" onClick={onCancel}>Cancelar</button>
    </div>
  );
}

function AccountCard({
  c,
  isAdmin,
  onAction,
  onUsage,
  onRemove,
}: {
  c: CliConnection;
  isAdmin: boolean;
  onAction(id: string, action: "login" | "status" | "logout"): Promise<unknown>;
  onUsage(id: string, allowed: boolean): Promise<unknown>;
  onRemove(id: string): Promise<unknown>;
}) {
  const active = !!c.session && ACTIVE.includes(c.session.state);
  return isAdmin ? (
    <div className="prov-actions">
      <button type="button" className="btn" disabled={active} onClick={() => onAction(c.id, "login")}>Conectar</button>
      <button type="button" className="btn ghost" disabled={active} onClick={() => onAction(c.id, "status")}>Verificar</button>
      <button type="button" className="btn ghost" disabled={active || c.auth_state !== "authenticated"} onClick={() => onAction(c.id, "logout")}>Desconectar</button>
      <button type="button" className="btn ghost" disabled={active || c.auth_state === "authenticated"} onClick={() => onRemove(c.id)}>Remover</button>
      <button type="button" className="btn ghost" onClick={() => onUsage(c.id, !c.usage_authorized)}>{c.usage_authorized ? "Revogar uso" : "Autorizar uso"}</button>
    </div>
  ) : null;
}

export function ProviderPage({ transport, isAdmin }: { transport: OneHubTransport; isAdmin: boolean }) {
  const [accounts, setAccounts] = useState<CliConnection[] | null>(null);
  const [apis, setApis] = useState<ProviderConnection[] | null>(null);
  const [executors, setExecutors] = useState<CentralExecutor[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [adding, setAdding] = useState<"choose" | "cli" | "api" | null>(null);

  const loadAccounts = useCallback(async () => {
    setAccounts(await transport.cliConnections());
  }, [transport]);

  const loadConnections = useCallback(async () => {
    const [cli, api] = await Promise.all([
      transport.cliConnections(),
      isAdmin ? transport.providerConnections() : Promise.resolve([] as ProviderConnection[]),
    ]);
    setAccounts(cli);
    setApis(api);
  }, [transport, isAdmin]);

  useEffect(() => {
    let alive = true;
    Promise.all([
      transport.cliConnections(),
      isAdmin ? transport.providerConnections() : Promise.resolve([] as ProviderConnection[]),
      isAdmin ? transport.executors() : Promise.resolve([] as CentralExecutor[]),
    ])
      .then(([a, p, ex]) => {
        if (!alive) return;
        setAccounts(a);
        setApis(p);
        setExecutors(ex);
      })
      .catch((e) => alive && setError(errorMessage(e)));
    return () => {
      alive = false;
    };
  }, [transport, isAdmin]);

  // enquanto houver operação ativa, atualiza o progresso a cada 2 s (o estado vem sempre do OneHub)
  const anyActive = !!accounts?.some((c) => c.session && ACTIVE.includes(c.session.state));
  useEffect(() => {
    if (!anyActive) return;
    const t = setInterval(() => {
      loadAccounts().catch(() => undefined);
    }, 2000);
    return () => clearInterval(t);
  }, [anyActive, loadAccounts]);

  async function run(work: () => Promise<unknown>, ok?: string): Promise<boolean> {
    setError(null);
    setNotice(null);
    try {
      await work();
      if (ok) setNotice(ok);
      await loadConnections();
      return true;
    } catch (e) {
      setError(errorMessage(e));
      return false;
    }
  }

  const rows = [
    ...(accounts ?? []).map((connection) => ({
      id: connection.id,
      name: connection.name,
      type: ENGINES.find((engine) => engine.id === connection.engine)?.label ?? connection.engine,
      executor: connection.executor_name,
      state: accountState(connection),
      detail: connection.session && ACTIVE.includes(connection.session.state)
        ? <Session session={connection.session} onCancel={() => void run(() => transport.cancelCliSession(connection.session!.id))} onCode={(code) => transport.submitCliCode(connection.session!.id, code).then(loadAccounts)} />
        : null,
      actions: <AccountCard
        c={connection}
        isAdmin={isAdmin}
        onAction={(id, action) => run(() => transport.startCliAction(id, action), action === "login" ? "Login iniciado." : undefined)}
        onUsage={(id, allowed) => run(() => transport.setCliUsage(id, allowed))}
        onRemove={(id) => run(() => transport.deleteCliConnection(id), "Conta removida.")}
      />,
    })),
    ...(apis ?? []).map((connection) => ({
      id: connection.id,
      name: connection.name,
      type: PROVIDERS.find((provider) => provider.id === connection.provider)?.label ?? connection.provider,
      executor: "API",
      state: { text: "Configurada", tone: "info" as const },
      detail: <p className="muted small">Chave: <code>{connection.key_hint}</code>{connection.default_model ? ` · modelo: ${connection.default_model}` : ""}. O teste de conexão não está disponível.</p>,
      actions: isAdmin ? <button type="button" className="btn ghost" onClick={() => void run(() => transport.deleteProviderConnection(connection.id), "Conexão removida.")}>Remover</button> : null,
    })),
  ];

  return (
    <div className="page provider-page">
      <PageHeader title="One Provider AI" subtitle="Gerencie as contas CLI e conexões API usadas pelo OneCoding." actions={isAdmin ? <button className="btn primary provider-add" type="button" onClick={() => setAdding("choose")}><Icon name="plus" /> Adicionar Provider</button> : undefined} />
      {error && <p role="alert" className="error">{error}</p>}
      {notice && <p role="status" className="muted small">{notice}</p>}
      <div className="provider-table-wrap">
        <table className="provider-table">
          <thead><tr><th>Conexão</th><th>Tipo</th><th>Executor</th><th>Estado</th>{isAdmin && <th>Ações</th>}</tr></thead>
          <tbody>
            {accounts === null || (isAdmin && apis === null) ? <tr><td colSpan={isAdmin ? 5 : 4} className="muted">Carregando conexões…</td></tr>
              : rows.length === 0 ? <tr><td colSpan={isAdmin ? 5 : 4} className="provider-empty">Nenhum provider cadastrado. Use “Adicionar Provider” para conectar uma conta.</td></tr>
              : rows.map((row) => <tr key={row.id}>
                <td><strong>{row.name}</strong>{row.detail && <div className="provider-detail">{row.detail}</div>}</td>
                <td>{row.type}</td><td>{row.executor}</td>
                <td><span className={`status-pill ${row.state.tone}`}>{row.state.text}</span></td>
                {isAdmin && <td>{row.actions}</td>}
              </tr>)}
          </tbody>
        </table>
      </div>
      {adding && <div className="provider-modal-backdrop" onMouseDown={(event) => { if (event.target === event.currentTarget) setAdding(null); }}>
        <section className="provider-modal" role="dialog" aria-modal="true" aria-labelledby="provider-modal-title">
          <header><div><h2 id="provider-modal-title">Adicionar Provider</h2><p className="muted small">Escolha o tipo de conexão.</p></div><button className="icon-btn" type="button" aria-label="Fechar" onClick={() => setAdding(null)}><Icon name="close" /></button></header>
          <label className="provider-kind"><span>Tipo de conexão</span><select value={adding === "choose" ? "" : adding} onChange={(event) => setAdding(event.target.value as "cli" | "api")}><option value="">Selecione…</option><option value="cli">CLI</option><option value="api">API</option></select></label>
          {adding === "cli"
            ? <AddAccount executors={executors} onCreate={async (input) => { if (await run(() => transport.createCliConnection(input), "Conta adicionada. Conecte-a para confirmar a autenticação.")) setAdding(null); }} />
            : adding === "api" ? <AddApi onCreate={async (input) => { if (await run(() => transport.createProviderConnection(input), "Chave salva de forma cifrada.")) setAdding(null); }} /> : null}
        </section>
      </div>}
    </div>
  );
}

function AddAccount({ executors, onCreate }: { executors: CentralExecutor[]; onCreate(input: { name: string; engine: CliEngine; executor_id: string }): Promise<void> }) {
  const online = executors.filter((x) => x.status !== "revoked");
  const [name, setName] = useState("");
  const [engine, setEngine] = useState<CliEngine>("claude");
  const [executorId, setExecutorId] = useState("");
  const [allowed, setAllowed] = useState(false);
  const chosen = executorId || online[0]?.id || "";

  async function submit(e: FormEvent) {
    e.preventDefault();
    if (!allowed || !chosen) return;
    await onCreate({ name: name.trim(), engine, executor_id: chosen });
    setName("");
    setAllowed(false);
  }

  return (
    <form className="prov-form card" aria-label="Adicionar conta de CLI" onSubmit={submit}>
      <h3>Adicionar conta de CLI</h3>
      {online.length === 0 && <p className="muted small">Cadastre e pareie um OneExecutor antes de adicionar contas.</p>}
      <label>
        <span>Nome amigável</span>
        <input value={name} onChange={(e) => setName(e.target.value)} placeholder="Claude Empresa 01" maxLength={120} />
      </label>
      <label>
        <span>Motor</span>
        <select value={engine} onChange={(e) => setEngine(e.target.value as CliEngine)}>
          {ENGINES.map((x) => <option key={x.id} value={x.id}>{x.label}</option>)}
        </select>
      </label>
      <label>
        <span>OneExecutor</span>
        <select value={chosen} onChange={(e) => setExecutorId(e.target.value)} disabled={online.length === 0}>
          {online.map((x) => <option key={x.id} value={x.id}>{x.name}</option>)}
        </select>
      </label>
      <label className="prov-check">
        <input type="checkbox" checked={allowed} onChange={(e) => setAllowed(e.target.checked)} />
        <span>Autorizo o uso desta conta neste OneExecutor, conforme os termos do fornecedor.</span>
      </label>
      <button type="submit" className="btn" disabled={!allowed || !chosen || name.trim() === ""}>Criar conta</button>
    </form>
  );
}

function AddApi({ onCreate }: { onCreate(input: { name: string; provider: ApiProvider; api_key: string; default_model?: string }): Promise<void> }) {
  const [name, setName] = useState("");
  const [provider, setProvider] = useState<ApiProvider>("openrouter");
  const [key, setKey] = useState("");
  const [model, setModel] = useState("");

  async function submit(e: FormEvent) {
    e.preventDefault();
    const value = key;
    // a chave sai da tela antes mesmo da resposta: nunca fica em estado depois de enviada
    setKey("");
    try {
      await onCreate({ name: name.trim(), provider, api_key: value.trim(), default_model: model.trim() || undefined });
      setName("");
      setModel("");
    } catch {
      // o erro já é mostrado pela página; a chave não é reaproveitada
    }
  }

  return (
    <form className="prov-form card" aria-label="Adicionar conexão por API" onSubmit={submit} autoComplete="off">
      <h3>Adicionar conexão por API</h3>
      <label>
        <span>Nome da conexão</span>
        <input value={name} onChange={(e) => setName(e.target.value)} maxLength={120} />
      </label>
      <label>
        <span>Provedor</span>
        <select value={provider} onChange={(e) => setProvider(e.target.value as ApiProvider)}>
          {PROVIDERS.map((x) => <option key={x.id} value={x.id}>{x.label}</option>)}
        </select>
      </label>
      <label>
        <span>Chave de API</span>
        <input type="password" value={key} onChange={(e) => setKey(e.target.value)} autoComplete="new-password" spellCheck={false} />
      </label>
      <label>
        <span>Modelo padrão (opcional)</span>
        <input value={model} onChange={(e) => setModel(e.target.value)} maxLength={120} />
      </label>
      <button type="submit" className="btn" disabled={name.trim() === "" || key.trim().length < 16}>Guardar chave</button>
    </form>
  );
}
