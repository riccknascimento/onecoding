import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { authorizeWorkspace, cancelChatGeneration, cancelRun, connectChatGptPlan, disconnectChatGptPlan, getOrchestratorStatus, getRun, listChatConversations, listChatMessages, listRuns, listWorkspaces, refreshChatGptModels, sendChatMessage, setChatConversationContext, startRun } from "../api";
import type { LocalGatewayState } from "../App";
import { Composer } from "../chat/Composer";
import { ContextBar } from "../chat/ContextBar";
import { HistoryRail, type HistoryKey } from "../chat/HistoryRail";
import { chatModelsOf, EMPTY_SELECTION, errText, fromConversation, fromRun, mergeContext, NO_CONTEXT, type Selection } from "../chat/model";
import { TopBar } from "../chat/TopBar";
import type { ChatConversation, ChatMode, EngineKind, ExecutorPreference, PersistedChatMessage, ProviderStatus, RunEvent, RunView, Workspace } from "../contracts";
import { DEMO_GATEWAYS } from "../demo/data";
import { RunMetrics, RunTimeline, StatusBadge } from "./RunTimeline";

const POLL_MS = 400;
const FINAL = new Set(["succeeded", "failed", "cancelled"]);

export function ChatPage({ local }: { local: LocalGatewayState }) {
  const [mode, setMode] = useState<ChatMode>("orchestrator");
  const [railOpen, setRailOpen] = useState(false);

  // ── Provedor do orquestrador (ChatGPT Plan Usage) ──
  const [providerStatus, setProviderStatus] = useState<ProviderStatus | null>(null);
  const [providerBusy, setProviderBusy] = useState(false);
  const [providerError, setProviderError] = useState<string | null>(null);

  // ── Conversas do Orquestrador ──
  const [conversations, setConversations] = useState<ChatConversation[]>([]);
  const [chatId, setChatId] = useState<string | null>(null);
  const [chatMessages, setChatMessages] = useState<PersistedChatMessage[]>([]);
  const [chatDraft, setChatDraft] = useState("");
  const [chatBusy, setChatBusy] = useState(false);
  const [chatError, setChatError] = useState<string | null>(null);
  const chatSendLock = useRef(false);

  // ── Contexto operacional (compartilhado pelos dois modos) ──
  // `sel` guarda o que foi pedido; o que vale (`selection`) é validado a cada render contra o que existe
  // agora. Assim, uma pasta ou modelo que ainda não carregou nunca apaga a escolha salva.
  const [sel, setSel] = useState<Selection>(EMPTY_SELECTION);
  const touched = useRef(false);
  const seededFromRuns = useRef(false);

  const [workspaces, setWorkspaces] = useState<Workspace[]>([]);
  const [workspacesLoaded, setWorkspacesLoaded] = useState(false);
  const [runs, setRuns] = useState<RunView[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [run, setRun] = useState<RunView | null>(null);
  const [events, setEvents] = useState<RunEvent[]>([]);
  const [error, setError] = useState<string | null>(null);
  const lastSeq = useRef(0);
  const threadRef = useRef<HTMLDivElement>(null);
  const firstConversationLoad = useRef(true);

  useEffect(() => {
    getOrchestratorStatus()
      .then(setProviderStatus)
      .catch((e: unknown) => setProviderStatus({ state: "error", message: errText(e) }));
  }, []);

  // O histórico é leitura local: aparece mesmo com o ChatGPT desconectado.
  useEffect(() => {
    listChatConversations()
      .then((items) => {
        setConversations(items);
        if (!firstConversationLoad.current) return;
        firstConversationLoad.current = false;
        const latest = items[0];
        if (!latest) return;
        seededFromRuns.current = true; // a conversa salva manda; a execução só serve de fallback
        setChatId((current) => current ?? latest.id);
        if (!touched.current) setSel((base) => fromConversation(latest, base));
      })
      .catch((e: unknown) => setChatError(errText(e)));
  }, []);

  useEffect(() => {
    if (!chatBusy || chatId) return;
    let alive = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const poll = async () => {
      try {
        const items = await listChatConversations();
        if (!alive) return;
        setConversations(items);
        if (items[0]) setChatId(items[0].id);
      } catch (e) { if (alive) setChatError(errText(e)); }
      if (alive && !chatId) timer = setTimeout(poll, 250);
    };
    void poll();
    return () => { alive = false; if (timer) clearTimeout(timer); };
  }, [chatBusy, chatId]);

  useEffect(() => {
    if (!chatId) { setChatMessages([]); return; }
    let alive = true;
    const load = () => listChatMessages(chatId).then((items) => { if (alive) setChatMessages(items); }).catch((e: unknown) => { if (alive) setChatError(errText(e)); });
    void load();
    let timer: ReturnType<typeof setTimeout> | undefined;
    const poll = async () => { await load(); if (alive && chatBusy) timer = setTimeout(poll, 500); };
    if (chatBusy) timer = setTimeout(poll, 500);
    return () => { alive = false; if (timer) clearTimeout(timer); };
  }, [chatId, chatBusy]);

  // ── Gateway, motor e modelos ──
  const gateways = useMemo(
    () => [...(local.phase === "ready" ? [local.gateway] : []), ...DEMO_GATEWAYS],
    [local],
  );
  // Só OneNodes reais são selecionáveis; os remotos aparecem como desconectados.
  // No Modo Direto a escolha é explícita: sem fallback para o primeiro disponível.
  const selectable = gateways.filter((g) => !g.demo);
  const gateway = selectable.find((g) => g.id === sel.gatewayId) ?? (mode === "direct" ? undefined : selectable[0]);
  const gatewayId = gateway?.id ?? "";

  const engine: ExecutorPreference = mode === "direct" && sel.engine === "auto" ? "codex" : sel.engine;
  const engineStatus = engine === "auto" ? undefined : gateway?.engines.find((e) => e.engine === engine);
  const models = engineStatus?.models ?? [];
  const executorModel = models.length === 0 || sel.executorModel === "" || models.some((m) => m.id === sel.executorModel) ? sel.executorModel : "";

  const workspaceId = !workspacesLoaded || workspaces.some((w) => w.id === sel.workspaceId) ? sel.workspaceId : (workspaces[0]?.id ?? "");

  const chatModels = chatModelsOf(providerStatus);
  const chatModel = chatModels.length === 0 || chatModels.some((m) => m.slug === sel.chatModel) ? sel.chatModel : chatModels[0]!.slug;

  const selection: Selection = { gatewayId, workspaceId, engine, executorModel, chatModel };
  const storedContext = conversations.find((c) => c.id === chatId)?.context ?? NO_CONTEXT;

  const updateSelection = (patch: Partial<Selection>) => {
    const next: Selection = { ...selection, ...patch };
    if (patch.engine !== undefined && patch.engine !== selection.engine) next.executorModel = "";
    if (patch.gatewayId !== undefined && patch.gatewayId !== selection.gatewayId) { next.workspaceId = ""; next.executorModel = ""; }
    touched.current = true;
    setSel(next);
    // Persiste na conversa aberta (SQLite é a fonte da verdade). É só preferência: não concede execução.
    if (mode === "orchestrator" && chatId) {
      const id = chatId;
      const context = mergeContext(next, storedContext);
      setChatConversationContext(id, context)
        .then(() => setConversations((cs) => cs.map((c) => (c.id === id ? { ...c, context } : c))))
        .catch((e: unknown) => setChatError(errText(e)));
    }
  };

  // Execução real só com Gateway local, Codex instalado e aprovado.
  const realCapable = gateway?.id === "local" && engine === "codex" && engineStatus?.trust === "trusted";
  const whyNot = !gateway || gateway.demo
    ? "Selecione o OneNode “Este Mac” para executar de verdade."
    : engine !== "codex"
      ? "Nesta fase só o Codex CLI executa de verdade."
      : engineStatus?.trust !== "trusted"
        ? "Aprove o Codex CLI em Agent Gateways para habilitar a execução."
        : null;

  const refreshRuns = useCallback(() => {
    listRuns(50).then(setRuns).catch(() => undefined);
  }, []);

  useEffect(() => {
    refreshRuns();
  }, [refreshRuns]);

  // Sem conversas salvas, as últimas preferências válidas vêm da execução mais recente.
  useEffect(() => {
    const latest = runs[0];
    if (seededFromRuns.current || !latest || touched.current || chatId) return;
    seededFromRuns.current = true;
    setSel((base) => fromRun(latest, base));
  }, [runs, chatId]);

  useEffect(() => {
    if (gateway?.id !== "local") return;
    listWorkspaces()
      .then((w) => {
        setWorkspaces(w);
        setWorkspacesLoaded(true);
      })
      .catch((e: unknown) => setError(errText(e)));
  }, [gateway?.id]);

  // Consulta o estado da execução selecionada até ela terminar.
  useEffect(() => {
    if (!selected) return;
    let alive = true;
    lastSeq.current = 0;
    setEvents([]);
    setRun(null);
    const tick = () =>
      getRun(selected, lastSeq.current)
        .then((d) => {
          if (!alive) return true;
          if (d.events.length > 0) {
            lastSeq.current = d.events[d.events.length - 1]!.seq;
            setEvents((prev) => [...prev, ...d.events]);
          }
          setRun(d.run);
          if (FINAL.has(d.run.status)) {
            refreshRuns();
            return true;
          }
          return false;
        })
        .catch((e: unknown) => {
          if (alive) setError(errText(e));
          return true;
        });
    let timer: ReturnType<typeof setTimeout> | undefined;
    const loop = async () => {
      const done = await tick();
      if (alive && !done) timer = setTimeout(loop, POLL_MS);
    };
    void loop();
    return () => {
      alive = false;
      if (timer) clearTimeout(timer);
    };
  }, [selected, refreshRuns]);

  const [draft, setDraft] = useState("");
  const active = run !== null && !FINAL.has(run.status);
  const canSend = realCapable && workspaceId !== "" && draft.trim() !== "" && !active;

  const send = () => {
    setError(null);
    startRun({ workspace_id: workspaceId, engine: engine as EngineKind, model: executorModel || null, prompt: draft })
      .then((id) => {
        setDraft("");
        setSelected(id);
        refreshRuns();
      })
      .catch((e: unknown) => setError(errText(e)));
  };

  const authorize = () => {
    setError(null);
    authorizeWorkspace()
      .then((w) => {
        if (!w) return;
        setWorkspaces((prev) => (prev.some((x) => x.id === w.id) ? prev : [...prev, w]));
        updateSelection({ workspaceId: w.id });
      })
      .catch((e: unknown) => setError(errText(e)));
  };

  // ── Provedor ──
  const connected = providerStatus?.state === "connected";
  const authorized = providerStatus?.state === "connected" && providerStatus.plan_usage_authorized;
  const needsConnect = !authorized;

  const providerStateText = providerStatus?.state === "connected"
    ? `Estado: conectado · ${providerStatus.label}`
    : providerStatus?.state === "simulated"
      ? "Estado: simulado (somente testes)"
      : providerStatus?.state === "error"
        ? `Estado: indisponível · ${providerStatus.message}`
        : providerStatus?.state === "disconnected"
          ? `Estado: desconectado · ${providerStatus.reason}`
          : "Estado: consultando o provedor…";

  const orchestratorNotice = providerStatus?.state === "simulated"
    ? "O provedor simulado existe somente nos testes e não produz respostas nesta interface."
    : providerStatus?.state === "connected"
      ? providerStatus.plan_usage_authorized
        ? "ChatGPT Plan Usage conectado. O chat não executa ferramentas nem tarefas nesta fase."
        : "Login concluído. Autorize ChatGPT Plan Usage para habilitar o provedor."
      : providerStatus?.state === "error"
        ? "Não foi possível confirmar a conexão. O VibeAgent permanece indisponível."
        : providerStatus?.state === "disconnected"
          ? "ChatGPT não conectado. Conecte uma conta para usar o VibeAgent."
          : "Consultando o estado do provedor…";

  const configuredProvider = providerStatus?.state === "connected"
    ? `Provedor configurado: ChatGPT Plan Usage · ${providerStatus.label}`
    : providerStatus?.state === "simulated"
      ? `Provedor configurado: simulado · ${providerStatus.label}`
      : "Provedor configurado: nenhum";

  const planUsageText = providerStatus?.state === "connected"
    ? providerStatus.plan_usage_authorized
      ? "ChatGPT Plan Usage: autorizado"
      : "Login ChatGPT concluído · ChatGPT Plan Usage não autorizado"
    : "ChatGPT Plan Usage: ainda não autorizado";

  const connectChatGpt = async () => {
    setProviderBusy(true);
    setProviderError(null);
    try {
      setProviderStatus(await connectChatGptPlan());
    } catch (e) {
      setProviderError(errText(e));
      getOrchestratorStatus().then(setProviderStatus).catch(() => undefined);
    } finally {
      setProviderBusy(false);
    }
  };

  const refreshChatGpt = async () => {
    setProviderBusy(true);
    setProviderError(null);
    try {
      setProviderStatus(await refreshChatGptModels());
    } catch (e) {
      setProviderError(errText(e));
    } finally {
      setProviderBusy(false);
    }
  };

  const disconnectChatGpt = async () => {
    setProviderBusy(true);
    setProviderError(null);
    try {
      await disconnectChatGptPlan();
      setProviderStatus(await getOrchestratorStatus());
    } catch (e) {
      setProviderError(errText(e));
      getOrchestratorStatus().then(setProviderStatus).catch(() => undefined);
    } finally {
      setProviderBusy(false);
    }
  };

  const sendOrchestratorMessage = async () => {
    const text = chatDraft.trim();
    if (!text || chatBusy || chatSendLock.current || !chatModel) return;
    chatSendLock.current = true;
    setChatError(null);
    setChatBusy(true);
    setChatDraft("");
    try {
      const id = await sendChatMessage(chatId, text, chatModel, mergeContext(selection, storedContext));
      setChatId(id);
      const [items, messages] = await Promise.all([listChatConversations(), listChatMessages(id)]);
      setConversations(items);
      setChatMessages(messages);
    } catch (e) {
      setChatError(errText(e));
      if (chatId) setChatMessages(await listChatMessages(chatId).catch(() => chatMessages));
    } finally {
      chatSendLock.current = false;
      setChatBusy(false);
    }
  };

  // ── Navegação entre modos e histórico ──
  // Cada modo lembra o item que estava aberto; ao voltar, a barra mostra o contexto desse item.
  const switchMode = (next: ChatMode) => {
    if (next === mode) return;
    setMode(next);
    touched.current = true;
    if (next === "direct") {
      const r = runs.find((x) => x.run_id === selected);
      if (r) setSel((base) => fromRun(r, base));
    } else {
      const c = conversations.find((x) => x.id === chatId);
      if (c) setSel((base) => fromConversation(c, base));
    }
  };

  const openConversation = (c: ChatConversation) => {
    touched.current = true;
    setMode("orchestrator");
    setChatId(c.id);
    setChatError(null);
    setSel((base) => fromConversation(c, base));
  };

  const openRun = (r: RunView) => {
    touched.current = true;
    setMode("direct");
    setError(null);
    setSelected(r.run_id);
    setSel((base) => fromRun(r, base));
  };

  const startNew = () => {
    if (mode === "orchestrator") {
      setChatId(null);
      setChatMessages([]);
      setChatError(null);
    } else {
      setSelected(null);
      setRun(null);
      setEvents([]);
      setError(null);
    }
  };

  const activeKey: HistoryKey | null = mode === "orchestrator" ? (chatId ? `c:${chatId}` : null) : selected ? `r:${selected}` : null;
  const title = mode === "orchestrator"
    ? (conversations.find((c) => c.id === chatId)?.title ?? "Nova conversa")
    : (run?.prompt ?? "Nova execução");

  useEffect(() => {
    const el = threadRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [chatMessages, events, mode, chatId]);

  const canChat = authorized && !chatBusy;
  const orchestratorMode = mode === "orchestrator";

  return (
    <section className={`chat chat-${mode}`}>
      <h1 className="sr-only">Chat</h1>
      <HistoryRail
        conversations={conversations}
        runs={runs}
        activeKey={activeKey}
        conversationsLocked={chatBusy}
        onOpenConversation={openConversation}
        onOpenRun={openRun}
        onNew={startNew}
        newLabel={orchestratorMode ? "Nova conversa" : "Nova execução"}
        open={railOpen}
        onClose={() => setRailOpen(false)}
      />
      <div className="chat-main">
        <TopBar
          title={title}
          mode={mode}
          onMode={switchMode}
          status={providerStatus}
          details={[configuredProvider, providerStateText, planUsageText]}
          busy={providerBusy}
          onRefresh={() => void refreshChatGpt()}
          onDisconnect={() => void disconnectChatGpt()}
          onToggleRail={() => setRailOpen((o) => !o)}
        />
        <ContextBar
          mode={mode}
          gateways={gateways}
          localPhase={local.phase}
          gatewayId={gatewayId}
          onGateway={(id) => updateSelection({ gatewayId: id })}
          workspaces={workspaces}
          workspaceId={workspaceId}
          workspaceEnabled={gateway?.id === "local"}
          onWorkspace={(id) => updateSelection({ workspaceId: id })}
          onAuthorize={authorize}
          engine={engine}
          onEngine={(e) => updateSelection({ engine: e })}
          models={models}
          executorModel={executorModel}
          onExecutorModel={(m) => updateSelection({ executorModel: m })}
          chatModels={chatModels}
          chatModel={chatModel}
          onChatModel={(m) => updateSelection({ chatModel: m })}
          chatBusy={chatBusy}
        />

        <div className="thread-scroll" ref={threadRef}>
          {orchestratorMode ? (
            <div className="thread" role="log" aria-label="Conversa do VibeAgent">
              {chatMessages.length === 0 && (
                <div className="empty">
                  <span className="empty-mark" aria-hidden="true" />
                  <h3>OneOrchestrator</h3>
                  <p>{orchestratorNotice}</p>
                  <p className="muted small">Ele conversa, planeja e pode delegar tarefas somente leitura ao OneNode e ao projeto escolhidos abaixo.</p>
                  {needsConnect && (
                    <button type="button" className="btn primary" onClick={() => void connectChatGpt()} disabled={providerBusy || providerStatus === null}>
                      {providerBusy ? "Aguardando navegador…" : connected ? "Autorizar ChatGPT Plan Usage" : "Conectar com ChatGPT"}
                    </button>
                  )}
                </div>
              )}
              {chatMessages.map((message) => <div className={`msg ${message.role === "user" ? "user" : "assistant"}`} key={message.id}>
                <div className="msg-role">{message.role === "user" ? "Você" : "ChatGPT"} · {message.model ?? chatModel}</div>
                <p>{message.text || (message.state === "streaming" ? "Gerando…" : "")}</p>
                {message.state === "error" && <div className="ev bad" role="alert">{message.errorMessage ?? "Falha na resposta."}</div>}
                {message.state === "interrupted" && <div className="muted small">Geração interrompida.</div>}
                {message.role === "orchestrator" && message.state === "completed" && <div className="muted small">Tokens: {message.usage.input_tokens ?? "—"} entrada · {message.usage.output_tokens ?? "—"} saída</div>}
              </div>)}
              {chatBusy && <div className="notice" role="status">Resposta em andamento… <button type="button" className="btn" disabled={!chatId} onClick={() => chatId && void cancelChatGeneration(chatId)}>Interromper</button></div>}
              {providerError && <div className="notice bad" role="alert">{providerError}</div>}
              {chatError && <div className="ev bad" role="alert">{chatError}</div>}
            </div>
          ) : (
            <div className="thread" role="log" aria-label="Conversa do Coding">
              {!realCapable && (
                <div className="empty">
                  <span className="empty-mark" aria-hidden="true" />
                  <h3>Execução direta</h3>
                  <p>{`Execução Direta indisponível. ${whyNot ?? ""}`}</p>
                </div>
              )}
              {realCapable && !run && !selected && (
                <div className="empty">
                  <span className="empty-mark" aria-hidden="true" />
                  <h3>Execução direta</h3>
                  <p>Execução real habilitada (Codex, somente leitura). Autorize uma pasta, escreva a tarefa e envie.</p>
                </div>
              )}
              {realCapable && run && (
                <>
                  <div className="msg user">
                    <div className="msg-role">Você · {run.workspace_label}</div>
                    <p>{run.prompt}</p>
                  </div>
                  <div className="run-head">
                    <StatusBadge status={run.status} />
                    {active && (
                      <button type="button" className="btn" onClick={() => void cancelRun(run.run_id)}>
                        Cancelar
                      </button>
                    )}
                  </div>
                  <RunTimeline events={events} />
                  {run.error_message && (
                    <div className="ev bad" role="alert">
                      {run.error_message}
                    </div>
                  )}
                  <RunMetrics run={run} />
                </>
              )}
              {error && (
                <div className="ev bad" role="alert">
                  {error}
                </div>
              )}
            </div>
          )}
        </div>

        <div className="dock">
          {orchestratorMode ? (
            <>
              {needsConnect && chatMessages.length > 0 && (
                <div className="notice">
                  {orchestratorNotice}
                  <button type="button" className="btn primary" onClick={() => void connectChatGpt()} disabled={providerBusy || providerStatus === null}>
                    {providerBusy ? "Aguardando navegador…" : connected ? "Autorizar ChatGPT Plan Usage" : "Conectar com ChatGPT"}
                  </button>
                </div>
              )}
              <p className="mode-note">VibeAgent · OneNode, projeto e motor são preferências desta conversa e limitam as tarefas que o OneOrchestrator pode delegar.</p>
              <Composer
                value={chatDraft}
                onChange={setChatDraft}
                onSubmit={() => void sendOrchestratorMessage()}
                placeholder={authorized ? "Descreva sua ideia, solicite uma análise ou planeje uma tarefa…" : "Conecte o ChatGPT Plan Usage para conversar."}
                disabled={!canChat}
                canSend={canChat && chatDraft.trim() !== "" && chatModel !== ""}
                hint="Enter envia · Shift+Enter quebra a linha"
              />
            </>
          ) : (
            <>
              <p className="mode-note">Coding · ChatGPT não participa desta execução.</p>
              <Composer
                value={draft}
                onChange={setDraft}
                onSubmit={send}
                placeholder={
                  realCapable
                    ? workspaceId
                      ? "Descreva a investigação (somente leitura)…"
                      : "Autorize uma pasta de projeto para começar."
                    : `Desativado: ${whyNot ?? ""}`
                }
                disabled={!realCapable || active}
                canSend={canSend}
                hint="Enter envia · somente leitura"
              />
            </>
          )}
        </div>
      </div>
    </section>
  );
}
