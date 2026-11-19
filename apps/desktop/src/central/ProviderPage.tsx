// One Provider AI: contas de CLI (Claude Code, Codex, Cursor) e conexões por API Key.
// Todo estado vem do OneHub. O login oficial roda no OneExecutor; a página só mostra o link, o código e o progresso.
import { useCallback, useEffect, useState } from "react";
import type { FormEvent } from "react";
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
  onCancel,
  onCode,
  onUsage,
  onRemove,
}: {
  c: CliConnection;
  isAdmin: boolean;
  onAction(id: string, action: "login" | "status" | "logout"): Promise<void>;
  onCancel(sessionId: string): Promise<void>;
  onCode(sessionId: string, code: string): Promise<void>;
  onUsage(id: string, allowed: boolean): Promise<void>;
  onRemove(id: string): Promise<void>;
}) {
  const st = accountState(c);
  const active = !!c.session && ACTIVE.includes(c.session.state);
  return (
    <li className="card prov-account">
      <div className="card-head">
        <strong>{c.name}</strong>
        <span className={`status-pill ${st.tone === "ok" ? "ok" : st.tone === "bad" ? "bad" : st.tone === "warn" ? "warn" : ""}`}>{st.text}</span>
      </div>
      <p className="muted small">OneExecutor: {c.executor_name} · uso autorizado: {c.usage_authorized ? "sim" : "não"}</p>
      {c.auth_checked_at && <p className="muted small">Última verificação: {new Date(c.auth_checked_at).toLocaleString("pt-BR")}</p>}
      {c.session && c.session.state !== "succeeded" && !active && (
        <p className="muted small">Última operação: {c.session.result ?? c.session.state}</p>
      )}
      {c.session && active && (
        <Session session={c.session} onCancel={() => onCancel(c.session!.id)} onCode={(code) => onCode(c.session!.id, code)} />
      )}
      {isAdmin && (
        <div className="prov-actions">
          <button type="button" className="btn" disabled={active} onClick={() => onAction(c.id, "login")}>Conectar conta</button>
          <button type="button" className="btn ghost" disabled={active} onClick={() => onAction(c.id, "status")}>Verificar</button>
          <button type="button" className="btn ghost" disabled={active || c.auth_state !== "authenticated"} onClick={() => onAction(c.id, "logout")}>Desconectar</button>
          <button type="button" className="btn ghost" disabled={active || c.auth_state === "authenticated"} onClick={() => onRemove(c.id)}>Remover</button>
          <button type="button" className="btn ghost" onClick={() => onUsage(c.id, !c.usage_authorized)}>
            {c.usage_authorized ? "Revogar uso" : "Autorizar uso"}
          </button>
        </div>
      )}
    </li>
  );
}

export function ProviderPage({ transport, isAdmin }: { transport: OneHubTransport; isAdmin: boolean }) {
  const [accounts, setAccounts] = useState<CliConnection[] | null>(null);
  const [apis, setApis] = useState<ProviderConnection[] | null>(null);
  const [executors, setExecutors] = useState<CentralExecutor[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const loadAccounts = useCallback(async () => {
    setAccounts(await transport.cliConnections());
  }, [transport]);

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

  async function run(work: () => Promise<unknown>, ok?: string) {
    setError(null);
    setNotice(null);
    try {
      await work();
      if (ok) setNotice(ok);
      await loadAccounts();
    } catch (e) {
      setError(errorMessage(e));
    }
  }

  return (
    <div className="page prov-page">
      <PageHeader
        title="One Provider AI"
        subtitle="Contas de IA conectadas ao OneHub. O login de cada CLI acontece pelo fluxo oficial do fornecedor, executado no OneExecutor."
      />
      {error && <p role="alert" className="error">{error}</p>}
      {notice && <p role="status" className="muted small">{notice}</p>}

      <section aria-label="Provedores por CLI" className="prov-section">
        <h2>Contas de CLI</h2>
        {accounts === null && !error && <p className="muted">Carregando contas…</p>}
        {ENGINES.map((eng) => {
          const list = (accounts ?? []).filter((c) => c.engine === eng.id);
          return (
            <div key={eng.id} className="prov-engine">
              <h3>{eng.label}</h3>
              {accounts !== null && list.length === 0 && <p className="muted small">Nenhuma conta cadastrada.</p>}
              {list.length > 0 && (
                <ul className="card-list" aria-label={`Contas ${eng.label}`}>
                  {list.map((c) => (
                    <AccountCard
                      key={c.id}
                      c={c}
                      isAdmin={isAdmin}
                      onAction={(id, action) => run(() => transport.startCliAction(id, action), action === "login" ? "Login iniciado. Acompanhe abaixo." : undefined)}
                      onCancel={(sid) => run(() => transport.cancelCliSession(sid), "Cancelamento solicitado.")}
                      onCode={(sid, code) => transport.submitCliCode(sid, code).then(() => loadAccounts())}
                      onUsage={(id, allowed) => run(() => transport.setCliUsage(id, allowed))}
                      onRemove={(id) => run(() => transport.deleteCliConnection(id), "Conta removida.")}
                    />
                  ))}
                </ul>
              )}
            </div>
          );
        })}

        {isAdmin && <AddAccount executors={executors} onCreate={(input) => run(() => transport.createCliConnection(input), "Conta criada. Use Conectar conta para entrar.")} />}
      </section>

      <section aria-label="Conexões por API" className="prov-section">
        <h2>Conexões por API</h2>
        <p className="muted small">
          A chave é cifrada no OneHub e nunca volta ao navegador. Teste de conexão e lista de modelos ainda dependem de chaves reais dos provedores e não estão disponíveis neste Central.
        </p>
        {apis === null && !error && isAdmin && <p className="muted">Carregando conexões…</p>}
        {apis && apis.length > 0 && (
          <ul className="card-list" aria-label="Conexões por API">
            {apis.map((p) => (
              <li key={p.id} className="card prov-api">
                <div className="card-head">
                  <strong>{p.name}</strong>
                  <span className="status-pill">{PROVIDERS.find((x) => x.id === p.provider)?.label ?? p.provider}</span>
                </div>
                <p className="muted small">Chave: <code>{p.key_hint}</code>{p.default_model ? ` · modelo padrão: ${p.default_model}` : ""}</p>
                {isAdmin && (
                  <button type="button" className="btn ghost" onClick={() => run(() => transport.deleteProviderConnection(p.id), "Conexão removida.")}>Remover</button>
                )}
              </li>
            ))}
          </ul>
        )}
        {apis && apis.length === 0 && <p className="muted small">Nenhuma conexão por API cadastrada.</p>}
        {isAdmin && <AddApi onCreate={(input) => run(() => transport.createProviderConnection(input), "Conexão guardada de forma cifrada.")} />}
      </section>
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
