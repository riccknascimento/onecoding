// Interface Web do OneAgent (OneHub Central). Conversa, execução e estados vêm de eventos reais do OneHub.
import { useCallback, useEffect, useState } from "react";
import type { ReactNode } from "react";
import { OneHubError, webTransport } from "../transport";
import type { CentralConversation, CentralExecutor, CentralGrant, CentralNode, CentralUser, OneHubTransport, StreamStatus, Subscription } from "../transport";
import { Composer } from "../chat/Composer";
import { Icon } from "../icons";
import { addMessages, applyEvent, fromSnapshot } from "./chatState";
import type { ChatState } from "./chatState";
import type { CentralWorkspace } from "../transport";
import { MessageView, ThreadView } from "./Thread";
import { AdminPanel, sectionFromPath, settingsPath } from "./AdminPanel";
import { OneNodesPage } from "./OneNodesPage";
import { Sidebar, type CentralView, type SettingsSection } from "./Sidebar";
import { CENTRAL_OPTION, ChatContextBar } from "./ChatContextBar";
import { ProjectsPage, ExecutionsPage, UnavailablePage } from "./pages";
import { ProviderPage } from "./ProviderPage";
import { WorkspaceTabs, type OpenTab } from "./WorkspaceTabs";
import { SettingsHome } from "./SettingsHome";

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

function LoginLayout({ children }: { children: ReactNode }) {
  return (
    <main className="central-login">
      <section className="central-login-brand" aria-label="Marca OneCoding">
        <img className="central-login-banner" src="/brand/onecoding-login-banner.png" alt="OneCoding — Plataforma de desenvolvimento inteligente" />
      </section>
      <section className="central-login-panel" aria-label="Acesso ao OneCoding">
        {children}
      </section>
    </main>
  );
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
  const [workspaces, setWorkspaces] = useState<CentralWorkspace[]>([]);
  const [openTabs, setOpenTabs] = useState<OpenTab[]>([]);
  const [activeWorkspaceId, setActiveWorkspaceId] = useState<string | null>(null);
  const [activeByWorkspace, setActiveByWorkspace] = useState<Record<string, string | null>>({});
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
  const [view, setView] = useState<CentralView>(() => (
    window.location.pathname === "/configuracoes/provider-ai"
      ? "provider"
      : window.location.pathname.startsWith("/configuracoes") ? "settings" : "chat"
  ));
  const [settingsSection, setSettingsSection] = useState<SettingsSection>(() => sectionFromPath(window.location.pathname));
  const [settingsHome, setSettingsHome] = useState(() => window.location.pathname === "/configuracoes");

  // O endereço representa a área atual; voltar para / sempre reabre o Chat.
  useEffect(() => {
    const onPop = () => {
      const workspaceId = new URLSearchParams(window.location.search).get("workspace");
      if (workspaceId && workspaces.some((workspace) => workspace.id === workspaceId)) {
        setActiveWorkspaceId(workspaceId);
        setOpenTabs((tabs) => tabs.includes(workspaceId) ? tabs : [...tabs, workspaceId]);
        setActiveId(activeByWorkspace[workspaceId] ?? conversations.find((item) => item.workspace_id === workspaceId)?.id ?? null);
        setView("chat");
        setDrawer(false);
        return;
      }
      if (window.location.pathname === "/configuracoes/provider-ai") {
        setView("provider");
      } else if (window.location.pathname.startsWith("/configuracoes")) {
        setSettingsHome(window.location.pathname === "/configuracoes");
        setSettingsSection(sectionFromPath(window.location.pathname));
        setView("settings");
      } else {
        setView("chat");
      }
      setDrawer(false);
    };
    window.addEventListener("popstate", onPop);
    return () => window.removeEventListener("popstate", onPop);
  }, [workspaces, conversations, activeByWorkspace]);

  function openSettings(section: SettingsSection) {
    setError(null);
    setDrawer(false);
    setSettingsSection(section);
    setSettingsHome(false);
    if (window.location.pathname !== settingsPath(section)) window.history.pushState({}, "", settingsPath(section));
    setView("settings");
  }

  function navigate(view: CentralView) {
    setError(null);
    if (view === "provider") {
      if (window.location.pathname !== "/configuracoes/provider-ai") window.history.pushState({}, "", "/configuracoes/provider-ai");
    } else if (window.location.pathname.startsWith("/configuracoes")) {
      window.history.pushState({}, "", "/");
    }
    setView(view);
    setDrawer(false);
  }


  const refreshLists = useCallback(async () => {
    const [c, g, n, x, w] = await Promise.all([transport.conversations(), transport.grants(), transport.nodes(), transport.executors(), transport.workspaces()]);
    setConversations(c);
    setGrants(g);
    setNodes(n);
    setExecutors(x);
    setWorkspaces(w);
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
        if (alive) {
          const available = await transport.workspaces();
          const stored = window.localStorage.getItem(`onecoding.open-workspaces.${u.id}`);
          const storedActive = window.localStorage.getItem(`onecoding.active-workspace.${u.id}`);
          let valid: string[] = [];
          try { valid = stored ? (JSON.parse(stored) as string[]).filter((id) => available.some((workspace) => workspace.id === id)) : []; } catch { valid = []; }
          const requested = new URLSearchParams(window.location.search).get("workspace");
          const savedActive = storedActive && available.some((workspace) => workspace.id === storedActive) ? storedActive : null;
          const chosen = requested && available.some((workspace) => workspace.id === requested)
            ? requested
            : storedActive === "settings" ? null : savedActive ?? valid[0] ?? available[0]?.id ?? null;
          const tabs = Array.from(new Set([...valid, ...(chosen ? [chosen] : [])]));
          setOpenTabs(tabs);
          setActiveWorkspaceId(chosen);
          const current = c.find((item) => item.workspace_id === chosen);
          if (current) setActiveId(current.id);
          setActiveByWorkspace(Object.fromEntries(available.map((workspace) => [workspace.id, c.find((item) => item.workspace_id === workspace.id)?.id ?? null])));
        }
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
      const signedIn = await transport.me();
      setUser(signedIn);
      const c = await refreshLists();
      const available = await transport.workspaces();
      const stored = window.localStorage.getItem(`onecoding.open-workspaces.${signedIn.id}`);
      const storedActive = window.localStorage.getItem(`onecoding.active-workspace.${signedIn.id}`);
      let valid: string[] = [];
      try { valid = stored ? (JSON.parse(stored) as string[]).filter((id) => available.some((workspace) => workspace.id === id)) : []; } catch { valid = []; }
      const savedActive = storedActive && available.some((workspace) => workspace.id === storedActive) ? storedActive : null;
      const chosen = storedActive === "settings" ? null : savedActive ?? valid[0] ?? available[0]?.id ?? null;
      const tabs = Array.from(new Set([...valid, ...(chosen ? [chosen] : [])]));
      setOpenTabs(tabs);
      setActiveWorkspaceId(chosen);
      setActiveId(c.find((item) => item.workspace_id === chosen)?.id ?? null);
      setActiveByWorkspace(Object.fromEntries(available.map((workspace) => [workspace.id, c.find((item) => item.workspace_id === workspace.id)?.id ?? null])));
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
    if (activeWorkspaceId) setActiveByWorkspace((state) => ({ ...state, [activeWorkspaceId]: null }));
    navigate("chat");
    if (activeWorkspaceId) window.history.replaceState({}, "", `/?workspace=${encodeURIComponent(activeWorkspaceId)}`);
    setDrawer(false);
  };

  const activateWorkspace = (id: OpenTab) => {
    if (id === "settings") {
      setSettingsHome(true);
      setView("settings");
      if (window.location.pathname !== "/configuracoes") window.history.pushState({}, "", "/configuracoes");
      return;
    }
    setActiveWorkspaceId(id);
    setOpenTabs((tabs) => tabs.includes(id) ? tabs : [...tabs, id]);
    setActiveId(activeByWorkspace[id] ?? conversations.find((conversation) => conversation.workspace_id === id)?.id ?? null);
    setView("chat");
    window.history.pushState({}, "", `/?workspace=${encodeURIComponent(id)}`);
  };

  const createWorkspace = async (name: string) => {
    try {
      const workspace = await transport.createWorkspace(name);
      setWorkspaces((items) => [workspace, ...items]);
      setOpenTabs((items) => [...items, workspace.id]);
      setActiveWorkspaceId(workspace.id);
      setActiveId(null);
      setChat(null);
      setView("chat");
      window.history.pushState({}, "", `/?workspace=${encodeURIComponent(workspace.id)}`);
    } catch (e) {
      setError(errorText(e));
    }
  };

  const renameWorkspace = async (id: string, name: string) => {
    try {
      const updated = await transport.renameWorkspace(id, name);
      setWorkspaces((items) => items.map((workspace) => workspace.id === id ? { ...workspace, ...updated } : workspace));
    } catch (e) {
      setError(errorText(e));
    }
  };

  const closeWorkspaceTab = (id: string) => {
    const remaining = openTabs.filter((item) => item !== id);
    setOpenTabs(remaining);
    if (user) window.localStorage.setItem(`onecoding.open-workspaces.${user.id}`, JSON.stringify(remaining));
    if (activeWorkspaceId === id) activateWorkspace(remaining[remaining.length - 1] ?? "settings");
  };

  useEffect(() => {
    if (user) {
      window.localStorage.setItem(`onecoding.open-workspaces.${user.id}`, JSON.stringify(openTabs));
      window.localStorage.setItem(`onecoding.active-workspace.${user.id}`, activeWorkspaceId ?? "settings");
    }
  }, [openTabs, activeWorkspaceId, user]);

  /** Persiste o contexto escolhido na conversa ativa (cria a conversa se ainda não existir). */
  const persistContext = async (node: string, project: string) => {
    setError(null);
    try {
      let id = activeId;
      if (!id) {
        const c = await transport.createConversation("Nova conversa", activeWorkspaceId ?? undefined);
        id = c.id;
        setActiveId(id);
        if (activeWorkspaceId) setActiveByWorkspace((state) => ({ ...state, [activeWorkspaceId]: id }));
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
        const c = await transport.createConversation("Nova conversa", activeWorkspaceId ?? undefined);
        id = c.id;
        setActiveId(id);
        if (activeWorkspaceId) setActiveByWorkspace((state) => ({ ...state, [activeWorkspaceId]: id }));
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
      <LoginLayout>
        <p className="muted">Carregando…</p>
      </LoginLayout>
    );
  }

  if (bootError && !user) {
    return (
      <LoginLayout>
        <p role="alert" className="error">OneHub Central indisponível. Verifique a conexão.</p>
        <button type="button" className="btn primary" onClick={() => setBootAttempt((n) => n + 1)}>
          Tentar de novo
        </button>
      </LoginLayout>
    );
  }

  if (!user) {
    return (
      <LoginLayout>
        <div className="central-login-heading">
          <span className="central-login-eyebrow">Plataforma de desenvolvimento</span>
          <h1>Bem-vindo de volta</h1>
          <p className="muted">Entre para acessar seus Workspaces, conversas e execuções.</p>
        </div>
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
      </LoginLayout>
    );
  }

  const active = conversations.find((c) => c.id === activeId);
  const workspaceConversations = activeWorkspaceId ? conversations.filter((conversation) => conversation.workspace_id === activeWorkspaceId) : [];
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
        settingsSection={settingsSection}
        isAdmin={isAdmin}
        email={user.email}
        conversations={workspaceConversations}
        workspaces={workspaces}
        activeWorkspaceId={shown === "chat" ? activeWorkspaceId : null}
        activeId={activeId}
        open={drawer}
        onView={navigate}
        onSettings={openSettings}
        onSelectConversation={(id) => {
          setError(null);
          if (window.location.pathname.startsWith("/configuracoes")) window.history.pushState({}, "", "/");
          setDrawer(false);
          setActiveId(id);
          if (activeWorkspaceId) setActiveByWorkspace((state) => ({ ...state, [activeWorkspaceId]: id }));
          setView("chat");
        }}
        onNewConversation={newConversation}
        onOpenWorkspace={(id) => activateWorkspace(id)}
        onLogout={logout}
      />
      {drawer && <div className="rail-scrim" onClick={() => setDrawer(false)} aria-hidden="true" />}

      {shown === "chat" ? (
        <section className="chat-main">
          <WorkspaceTabs workspaces={workspaces} openTabs={openTabs} activeTab={shown === "chat" ? activeWorkspaceId ?? "settings" : "settings"} onActivate={activateWorkspace} onOpen={(id) => activateWorkspace(id)} onClose={closeWorkspaceTab} onCreate={createWorkspace} onRename={renameWorkspace} />
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
          <WorkspaceTabs workspaces={workspaces} openTabs={openTabs} activeTab="settings" onActivate={activateWorkspace} onOpen={(id) => activateWorkspace(id)} onClose={closeWorkspaceTab} onCreate={createWorkspace} onRename={renameWorkspace} />
          <header className="chat-top page-top">
            <button type="button" className="icon-btn rail-toggle" onClick={() => setDrawer((d) => !d)} aria-label="Abrir menu">
              <Icon name="history" />
            </button>
          </header>
          <div className="page-scroll">
            {shown === "onenodes" && <OneNodesPage transport={transport} />}
            {shown === "provider" && <ProviderPage transport={transport} isAdmin={isAdmin} />}
            {shown === "settings" && (settingsHome
              ? <SettingsHome nodes={nodes} executors={executors} onSettings={openSettings} onView={navigate} />
              : <AdminPanel transport={transport} user={user} section={settingsSection} onSection={openSettings} />)}
            {shown === "agents" && (
              <UnavailablePage
                title="Agent"
                icon="bot"
                reason="Agentes especializados ainda não têm backend neste OneHub. Nenhum agente foi simulado."
                next="As execuções existentes aparecem em Execuções."
              />
            )}
            {shown === "plugins" && (
              <UnavailablePage
                title="Plugins"
                icon="plus"
                reason="Plugins ainda não têm backend neste OneHub. Nenhum plugin foi simulado."
                next="Os motores disponíveis aparecem em One Provider AI."
              />
            )}
            {shown === "mcp" && (
              <UnavailablePage
                title="MCP"
                icon="gateway"
                reason="Servidores MCP ainda não têm backend neste OneHub. Nenhum servidor foi simulado."
                next="Os motores disponíveis aparecem em One Provider AI."
              />
            )}
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
                title="Skill"
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
