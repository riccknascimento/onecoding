// Interface Web do OneAgent (OneHub Central). Conversa, execução e estados vêm de eventos reais do OneHub.
import { useCallback, useEffect, useState } from "react";
import { OneHubError, webTransport } from "../transport";
import type { CentralConversation, CentralExecutor, CentralGrant, CentralNode, CentralUser, OneHubTransport, StreamStatus, Subscription } from "../transport";
import { Composer } from "../chat/Composer";
import { Icon } from "../icons";
import { addMessages, applyEvent, fromSnapshot } from "./chatState";
import type { ChatState } from "./chatState";
import { MessageView, ThreadView } from "./Thread";
import { AdminPanel } from "./AdminPanel";
import { OneNodesPage } from "./OneNodesPage";
import { Sidebar, type CentralView } from "./Sidebar";
import { CENTRAL_OPTION, ChatContextBar } from "./ChatContextBar";
import { ProjectsPage, ExecutionsPage, UnavailablePage } from "./pages";
import { ProviderPage } from "./ProviderPage";

const STREAM_LABEL: Record<StreamStatus, { text: string; tone: string }> = {
  idle: { text: "Aguardando conversa", tone: "info" },
  connecting: { text: "Conectando…", tone: "warn" },
  live: { text: "Tempo real", tone: "ok" },
  reconnecting: { text: "Reconectando…", tone: "warn" },
  closed: { text: "Sem atualizações", tone: "bad" },
};
/** Motivos devolvidos pelo stream quando o OneHub encerra o acesso. */
const CLOSED_LABEL: Record<string, string> = {
  session_ended: "Sua sessão terminou. Entre de novo para continuar.",
  grant_revoked: "O acesso deste OneNode a este projeto foi revogado. A conversa continua visível, sem novas atualizações.",
  access_ended: "Esta conversa não está mais disponível para você.",
  http: "O OneHub recusou o acompanhamento desta conversa.",
};

const ERROR_LABEL: Record<string, string> = {
  unauthorized: "E-mail ou senha inválidos.",
  too_many_attempts: "Muitas tentativas de entrada. Aguarde 15 minutos e tente de novo.",
  origin_not_allowed: "Requisição recusada por segurança. Recarregue a página.",
  no_grant: "Este OneNode não tem permissão para este projeto.",
  node_revoked: "Este OneNode foi revogado.",
  context_incomplete: "Escolha o OneNode e o projeto juntos.",
  invalid_body: "Mensagem vazia ou longa demais (máximo 16 KB).",
  already_finished: "Este pedido já terminou.",
  not_found: "Conversa ou pedido não encontrado.",
  internal: "O OneHub Central retornou erro interno.",
};

function errorText(e: unknown): string {
  if (e instanceof OneHubError) return ERROR_LABEL[e.code] ?? "Não foi possível concluir a operação.";
  return "OneHub Central indisponível. Verifique a conexão.";
}

function isUnauthorized(e: unknown): boolean {
  return e instanceof OneHubError && e.status === 401;
}

export function CentralApp({ transport = webTransport }: { transport?: OneHubTransport }) {
  const [user, setUser] = useState<CentralUser | null>(null);
  const [booting, setBooting] = useState(true);
  const [bootError, setBootError] = useState(false);
  const [bootAttempt, setBootAttempt] = useState(0);
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [conversations, setConversations] = useState<CentralConversation[]>([]);
  const [grants, setGrants] = useState<CentralGrant[]>([]);
  const [nodes, setNodes] = useState<CentralNode[]>([]);
  const [executors, setExecutors] = useState<CentralExecutor[]>([]);
  const [activeId, setActiveId] = useState<string | null>(null);
  const [chat, setChat] = useState<ChatState | null>(null);
  const [stream, setStream] = useState<{ status: StreamStatus; reason?: string }>({ status: "idle" });
  const [draft, setDraft] = useState("");
  const [sending, setSending] = useState(false);
  const [cancelling, setCancelling] = useState<Set<string>>(new Set());
  const [drawer, setDrawer] = useState(false);
  const [view, setView] = useState<CentralView>(() => (window.location.pathname.startsWith("/configuracoes") ? "settings" : "chat"));


  const refreshLists = useCallback(async () => {
    const [c, g, n, x] = await Promise.all([transport.conversations(), transport.grants(), transport.nodes(), transport.executors()]);
    setConversations(c);
    setGrants(g);
    setNodes(n);
    setExecutors(x);
    return c;
  }, [transport]);

  const leave = useCallback(() => {
    setUser(null);
    setConversations([]);
    setGrants([]);
    setNodes([]);
    setExecutors([]);
    setActiveId(null);
    setChat(null);
    setDraft("");
  }, []);

  // sessão restaurada pelo cookie: a página pode ser recarregada sem perder o acesso.
  // Só 401 leva à tela de entrada; falha de rede ou do servidor mostra indisponibilidade e permite tentar de novo.
  useEffect(() => {
    let alive = true;
    setBooting(true);
    setBootError(false);
    transport
      .me()
      .then(async (u) => {
        if (!alive) return;
        setUser(u);
        const c = await refreshLists();
        if (alive && c[0]) setActiveId(c[0].id);
        if (alive) setBooting(false);
      })
      .catch((e) => {
        if (!alive) return;
        if (isUnauthorized(e)) setBooting(false);
        else {
          setBootError(true);
          setBooting(false);
        }
      });
    return () => {
      alive = false;
    };
  }, [transport, refreshLists, bootAttempt]);

  // status de nós e executores se atualiza sem recarregar a página
  useEffect(() => {
    if (!user) return;
    const timer = setInterval(() => refreshLists().catch(() => undefined), 20_000);
    return () => clearInterval(timer);
  }, [user, refreshLists]);

  // Escape fecha o menu no celular (atalho de teclado)
  useEffect(() => {
    if (!drawer) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setDrawer(false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [drawer]);

  // conversa ativa: snapshot e, em seguida, stream a partir da sequência do snapshot
  useEffect(() => {
    if (!user || !activeId) {
      setChat(null);
      setStream({ status: "idle" });
      return;
    }
    let alive = true;
    let sub: Subscription | null = null;
    setChat(null);
    setStream({ status: "connecting" });
    transport
      .state(activeId)
      .then((snap) => {
        if (!alive) return;
        setChat(fromSnapshot(snap));
        sub = transport.subscribe(activeId, {
          afterSeq: snap.last_seq,
          onEvent: (ev) => {
            if (alive) setChat((s) => (s ? applyEvent(s, ev) : s));
          },
          onStatus: (status, reason) => {
            if (!alive) return;
            setStream({ status, reason });
            if (status !== "closed") return;
            // o OneHub encerrou: verifica se a própria sessão ainda existe antes de dizer o motivo
            transport
              .me()
              .catch((e) => {
                if (alive && isUnauthorized(e)) leave();
              });
          },
        });
      })
      .catch((e) => {
        if (!alive) return;
        if (isUnauthorized(e)) leave();
        else setError(errorText(e));
      });
    return () => {
      alive = false;
      sub?.close();
    };
  }, [transport, user, activeId, leave]);

  const submitLogin = async (ev: React.FormEvent) => {
    ev.preventDefault();
    setBusy(true);
    setError(null);
    try {
      await transport.login(email.trim(), password);
      setPassword("");
      setUser(await transport.me());
      const c = await refreshLists();
      setActiveId(c[0]?.id ?? null);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  const logout = async () => {
    await transport.logout().catch(() => undefined);
    leave();
  };

  const newConversation = () => {
    setError(null);
    setActiveId(null);
    setDrawer(false);
  };

  /** Persiste o contexto escolhido na conversa ativa (cria a conversa se ainda não existir). */
  const persistContext = async (node: string, project: string) => {
    setError(null);
    try {
      let id = activeId;
      if (!id) {
        const c = await transport.createConversation();
        id = c.id;
        setActiveId(id);
      }
      await transport.setConversationContext(id, { node_id: node || null, project_id: project || null });
      await refreshLists();
    } catch (e) {
      setError(errorText(e));
    }
  };

  const send = async (ev?: React.FormEvent) => {
    ev?.preventDefault();
    const text = draft.trim();
    if (!text || sending) return;
    setSending(true);
    setError(null);
    try {
      let id = activeId;
      if (!id) {
        const c = await transport.createConversation();
        id = c.id;
        setActiveId(id);
      }
      const result = await transport.turn(id, { text });
      setDraft("");
      setChat((s) => (s ? addMessages(s, [result.user_message, result.assistant_message]) : s));
      await refreshLists();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setSending(false);
    }
  };

  const cancelJob = async (jobId: string) => {
    setCancelling((s) => new Set(s).add(jobId));
    setError(null);
    try {
      await transport.cancelJob(jobId);
    } catch (e) {
      setError(errorText(e));
      setCancelling((s) => {
        const next = new Set(s);
        next.delete(jobId);
        return next;
      });
    }
  };

  if (booting) {
    return (
      <main className="central-login">
        <p className="muted">Carregando…</p>
      </main>
    );
  }

  if (bootError && !user) {
    return (
      <main className="central-login">
        <h1>OneCoding</h1>
        <p role="alert" className="error">OneHub Central indisponível. Verifique a conexão.</p>
        <button type="button" className="btn primary" onClick={() => setBootAttempt((n) => n + 1)}>
          Tentar de novo
        </button>
      </main>
    );
  }

  if (!user) {
    return (
      <main className="central-login">
        <h1>OneCoding</h1>
        <p className="muted">Entre no OneCoding para conversar e acompanhar execuções nos seus OneNodes.</p>
        <form onSubmit={submitLogin} aria-label="Entrar no OneHub Central">
          <label>
            E-mail
            <input type="email" autoComplete="username" value={email} onChange={(e) => setEmail(e.target.value)} required />
          </label>
          <label>
            Senha
            <input type="password" autoComplete="current-password" value={password} onChange={(e) => setPassword(e.target.value)} required />
          </label>
          <button type="submit" className="btn primary" disabled={busy}>
            {busy ? "Entrando…" : "Entrar"}
          </button>
        </form>
        {error && <p role="alert" className="error">{error}</p>}
      </main>
    );
  }

  const active = conversations.find((c) => c.id === activeId);
  const streamInfo = STREAM_LABEL[stream.status];
  const closedNote = stream.status === "closed" ? CLOSED_LABEL[stream.reason ?? "http"] ?? CLOSED_LABEL.http : null;
  const messages = chat?.messages ?? [];
  const tick = chat ? `${chat.lastSeq}:${messages.length}:${Object.values(chat.jobs).map((j) => `${j.status}${j.timeline.length}`).join(",")}` : null;

  const isAdmin = user.role === "admin";
  const shown: CentralView = (view === "onenodes" || view === "settings" || view === "provider") && !isAdmin ? "chat" : view;
  const ctxNodeId = active?.node_id ?? CENTRAL_OPTION;
  const ctxProjectId = active?.project_id ?? "";

  return (
    <div className="chat-shell">
      <Sidebar
        view={shown}
        isAdmin={isAdmin}
        email={user.email}
        conversations={conversations}
        activeId={activeId}
        open={drawer}
        onView={(v) => {
          setError(null);
          // cada área de configurações tem endereço próprio; sair dela volta para a raiz
          if (v === "settings") {
            if (!window.location.pathname.startsWith("/configuracoes")) window.history.pushState({}, "", "/configuracoes/OneNodes");
          } else if (window.location.pathname.startsWith("/configuracoes")) {
            window.history.pushState({}, "", "/");
          }
          setView(v);
          setDrawer(false);
        }}
        onSelectConversation={(id) => {
          setError(null);
          setDrawer(false);
          setActiveId(id);
          setView("chat");
        }}
        onNewConversation={newConversation}
        onLogout={logout}
      />
      {drawer && <div className="rail-scrim" onClick={() => setDrawer(false)} aria-hidden="true" />}

      {shown === "chat" ? (
        <section className="chat-main">
          <header className="chat-top">
            <button type="button" className="icon-btn rail-toggle" onClick={() => setDrawer((d) => !d)} aria-label="Abrir menu">
              <Icon name="history" />
            </button>
            <h1 className="chat-title">{active?.title ?? "Nova conversa"}</h1>
            <span className={`status-pill ${streamInfo.tone}`} aria-live="polite">
              <span className="dot" aria-hidden="true" />
              {streamInfo.text}
            </span>
          </header>

          <ChatContextBar
            nodes={nodes}
            grants={grants}
            executors={executors}
            nodeId={ctxNodeId}
            projectId={ctxProjectId}
            isAdmin={isAdmin}
            onNode={(n) => void persistContext(n, "")}
            onProject={(p) => void persistContext(ctxNodeId, p)}
            onOpenOneNodes={() => setView("onenodes")}
          />

          {error && <p role="alert" className="error banner">{error}</p>}
          {closedNote && <p role="status" className="notice banner">{closedNote}</p>}

          <ThreadView tick={tick}>
            {messages.length === 0 && chat && (
              <div className="empty">
                <h3>Peça uma análise</h3>
                <p>Escreva o que quer investigar. Pedidos de leitura são executados pelo OneExecutor, somente leitura, no projeto escolhido.</p>
                <p className="muted small">No contexto OneHub Central você administra o ambiente. Para analisar um projeto, escolha um OneNode com projeto autorizado.</p>
              </div>
            )}
            {!chat && activeId && <p className="muted">Carregando conversa…</p>}
            {messages.map((m) => (
              <MessageView
                key={m.id}
                message={m}
                jobs={chat?.jobs ?? {}}
                tasks={chat?.tasks ?? {}}
                transport={transport}
                cancelling={cancelling}
                onCancel={cancelJob}
              />
            ))}
          </ThreadView>

          <div className="dock">
            <div className="quick" role="group" aria-label="Comandos rápidos">
              <button type="button" className="chip" onClick={() => setDraft("/investigar ")}>/investigar</button>
              <button type="button" className="chip is-disabled" disabled title="Ainda não disponível neste OneHub">/corrigir · em breve</button>
              <button type="button" className="chip is-disabled" disabled title="Ainda não disponível neste OneHub">/melhorar · em breve</button>
              <button type="button" className="chip is-disabled" disabled title="Ainda não disponível neste OneHub">/testar · em breve</button>
            </div>
            <Composer
              value={draft}
              onChange={setDraft}
              onSubmit={() => void send()}
              placeholder="Descreva o que precisa ou use /investigar caminho"
              disabled={false}
              canSend={!sending && draft.trim() !== ""}
              hint="Enter envia · Shift+Enter quebra linha · Esc fecha o menu"
            />
          </div>
        </section>
      ) : (
        <section className="chat-main">
          <header className="chat-top page-top">
            <button type="button" className="icon-btn rail-toggle" onClick={() => setDrawer((d) => !d)} aria-label="Abrir menu">
              <Icon name="history" />
            </button>
          </header>
          <div className="page-scroll">
            {shown === "onenodes" && <OneNodesPage transport={transport} />}
            {shown === "provider" && <ProviderPage transport={transport} isAdmin={isAdmin} />}
            {shown === "settings" && <AdminPanel transport={transport} user={user} />}
            {shown === "projects" && <ProjectsPage transport={transport} isAdmin={isAdmin} onOpenOneNodes={() => setView("onenodes")} />}
            {shown === "executions" && <ExecutionsPage transport={transport} />}
            {shown === "context" && (
              <UnavailablePage
                title="OneContext"
                icon="book"
                reason="Documentação, memória e contexto técnico ainda não têm backend neste OneHub. Nenhum conteúdo foi simulado."
                next="Os projetos continuam disponíveis em Projetos."
              />
            )}
            {shown === "skills" && (
              <UnavailablePage
                title="Skills & Agents"
                icon="bot"
                reason="Agentes, skills e ferramentas ainda não têm backend neste OneHub. Nenhum agente foi simulado."
                next="As execuções existentes aparecem em Execuções."
              />
            )}
          </div>
        </section>
      )}
    </div>
  );
}
