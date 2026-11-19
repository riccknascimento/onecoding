// Contrato de dados do OneHub usado pela interface. A tela NÃO conhece HTTP nem Tauri.
// A sessão fica num cookie HttpOnly do próprio OneHub: a interface nunca vê nem guarda o token.

export interface CentralUser {
  id: string;
  email: string;
  role: "admin" | "member";
}

export interface CentralNode {
  id: string;
  name: string;
  status: "online" | "offline" | "pending" | "revoked";
  last_seen_at: string | null;
  /** Suspenso pelo administrador: não recebe novas operações. Reversível. */
  suspended?: boolean;
  /** O OneNode já aplicou a versão atual do catálogo enviado pelo OneHub. */
  catalog_in_sync?: boolean;
  access_type?: "direct" | "tailscale";
}

/** Detalhe administrativo de um OneNode (somente administradores). */
export interface CentralNodeDetail {
  id: string;
  name: string;
  status: "online" | "offline" | "pending" | "revoked";
  suspended: boolean;
  paired_at: string | null;
  last_seen_at: string | null;
  access_type: "direct" | "tailscale";
  catalog_version: number;
  applied_version: number | null;
  applied_at: string | null;
  catalog_in_sync: boolean;
  /** Relatório da última aplicação: o que o OneNode aceitou e o que recusou (com motivo curto). */
  applied_report: { accepted?: string[]; rejected?: { project_id: string; reason: string }[] } | null;
  grants: { project_id: string; project_name: string; root_path: string | null }[];
}

export interface CentralGrant {
  node_id: string;
  node_name: string;
  project_id: string;
  project_name: string;
  root_path: string | null;
}

export interface CentralConversation {
  id: string;
  title: string;
  updated_at: string;
  node_id: string | null;
  project_id: string | null;
  workspace_id?: string;
}

export interface CentralWorkspace {
  id: string;
  name: string;
  node_id: string | null;
  project_id: string | null;
  updated_at: string;
}

export type MessageState = "completed" | "unavailable" | "rejected" | "task" | "failed" | "cancelled";

export interface CentralMessage {
  id: string;
  role: "user" | "assistant" | "system";
  body: string;
  created_at: string;
  state: MessageState;
  task_id: string | null;
  job_id: string | null;
}

export type CentralTaskStatus = "queued" | "running" | "succeeded" | "failed" | "cancelled";

export interface CentralTaskDetail {
  id: string;
  status: CentralTaskStatus;
  tool: string | null;
  args: { path?: string };
  result: Record<string, unknown> | null;
  events: { seq: number; kind: string; payload: unknown; created_at: string }[];
}

export type CentralJobStatus = "queued" | "running" | "succeeded" | "failed" | "cancelled";

export interface CentralJob {
  id: string;
  status: CentralJobStatus;
  question: string;
  answer: string | null;
  evidence: string[];
  error: string | null;
  cancel_requested: boolean;
}

export interface CentralTaskSummary {
  id: string;
  node_id: string;
  node_name: string;
  project_id: string;
  project_name: string;
  tool: string | null;
  status: CentralTaskStatus;
  created_at: string;
  updated_at: string;
}

/** Estado do motor reportado pelo OneExecutor no heartbeat (sem credenciais). */
export interface CentralEngineReport {
  name: string;
  installed: boolean;
  authenticated: boolean;
  approved: boolean;
  version?: string | null;
}

export interface CentralExecutor {
  id: string;
  name: string;
  engine: string;
  status: "online" | "offline" | "pending" | "revoked";
  engines?: CentralEngineReport[];
}

export type CliEngine = "codex" | "claude" | "cursor";
export type CliAuthState = "unknown" | "authenticated" | "unauthenticated";
export type CliAction = "login" | "status" | "logout";

/** Sessão de login, verificação ou desconexão de uma conta de CLI (sem nenhum segredo). */
export interface CliSessionView {
  id: string;
  action: CliAction;
  state: string;
  url: string | null;
  device_code: string | null;
  awaiting_code: boolean;
  result: string | null;
}

/** Conta de CLI: uma conexão própria, com perfil isolado no OneExecutor que a administra. */
export interface CliConnection {
  id: string;
  name: string;
  engine: CliEngine;
  executor_id: string;
  executor_name: string;
  usage_authorized: boolean;
  auth_state: CliAuthState;
  auth_checked_at: string | null;
  installed?: boolean | null;
  approved?: boolean | null;
  version?: string | null;
  installation_reported_at?: string | null;
  session: CliSessionView | null;
}

export type ApiProvider = "openrouter" | "openai" | "gemini" | "anthropic";

/** Conexão por API Key. A chave nunca volta ao navegador: só a dica dos últimos caracteres. */
export interface ProviderConnection {
  id: string;
  name: string;
  provider: ApiProvider;
  key_hint: string;
  default_model: string | null;
  status: string;
  last_checked_at?: string | null;
  created_at: string;
}

export interface OrchestratorSettings { enabled: false; provider_connection_id: string | null; model: string | null; system_prompt: string; instructions: string; runtime_available: false; updated_at?: string; }
export interface ExecutorSettings { executor_id: string; executor_name: string; cli_connection_id: string | null; instructions: string; runtime_supported: false; }

export interface TurnInput {
  text: string;
  node_id?: string;
  project_id?: string;
}

export interface TurnResult {
  user_message: CentralMessage;
  assistant_message: CentralMessage;
}

/** Eventos do stream da conversa. Só mudanças reais gravadas pelo OneHub. */
export const STREAM_KINDS = ["message.created", "message.updated", "job.status", "job.progress", "task.status"] as const;
export type StreamKind = (typeof STREAM_KINDS)[number];

export interface StreamEvent {
  seq: number;
  kind: StreamKind;
  payload: Record<string, unknown>;
}

/** Estado do canal de tempo real. `closed` = o OneHub encerrou (sessão, concessão ou conversa). */
export type StreamStatus = "idle" | "connecting" | "live" | "reconnecting" | "closed";

export interface ConversationSnapshot {
  last_seq: number;
  messages: CentralMessage[];
  events: { seq: number; kind: string; payload: Record<string, unknown> }[];
  jobs: { id: string; status: CentralJobStatus; question: string; error: string | null; cancel_requested: boolean }[];
  tasks: { id: string; status: CentralTaskStatus; tool: string | null; job_id: string | null }[];
}

export interface Subscription {
  close(): void;
}

export interface SubscribeOptions {
  /** Última sequência já aplicada. Eventos com sequência maior são entregues, uma vez cada. */
  afterSeq: number;
  onEvent(event: StreamEvent): void;
  onStatus(status: StreamStatus, reason?: string): void;
}

export interface OneHubTransport {
  login(email: string, password: string): Promise<void>;
  logout(): Promise<void>;
  /** Lê a sessão atual pelo cookie. Falha com 401 quando não há sessão válida. */
  me(): Promise<CentralUser>;
  nodes(): Promise<CentralNode[]>;
  projects(): Promise<{ id: string; name: string }[]>;
  grants(): Promise<CentralGrant[]>;
  workspaces(): Promise<CentralWorkspace[]>;
  createWorkspace(name: string): Promise<CentralWorkspace>;
  renameWorkspace(id: string, name: string): Promise<CentralWorkspace>;
  setWorkspaceContext(id: string, ctx: { node_id: string | null; project_id: string | null }): Promise<void>;
  conversations(workspaceId?: string): Promise<CentralConversation[]>;
  createConversation(title?: string, workspaceId?: string): Promise<{ id: string; title: string; workspace_id?: string }>;
  /** Persiste o contexto da conversa. Ambos nulos = contexto OneHub Central (sem OneNode externo). */
  setConversationContext(conversationId: string, ctx: { node_id: string | null; project_id: string | null }): Promise<void>;
  /** Execuções do próprio usuário (tarefas enviadas aos OneNodes), mais recentes primeiro. */
  tasks(): Promise<CentralTaskSummary[]>;
  /** Saúde do OneHub Central (sem autenticação). */
  health(): Promise<{ status: string; database: string }>;
  /** Administração (somente administradores; o servidor recusa os demais). */
  createNode(name: string): Promise<{ id: string; pairing_code: string; pairing_expires_at: string }>;
  nodeDetail(id: string): Promise<CentralNodeDetail>;
  suspendNode(id: string): Promise<void>;
  resumeNode(id: string): Promise<void>;
  revokeNode(id: string): Promise<void>;
  createExecutor(name: string): Promise<{ id: string; pairing_code: string; pairing_expires_at: string }>;
  revokeExecutor(id: string): Promise<void>;
  createProject(name: string): Promise<{ id: string; name: string }>;
  /** Concede o projeto ao OneNode com a raiz pedida. A política local do OneNode continua valendo. */
  grantProject(projectId: string, nodeId: string, rootPath: string): Promise<void>;
  revokeGrant(projectId: string, nodeId: string): Promise<void>;
  messages(conversationId: string): Promise<CentralMessage[]>;
  turn(conversationId: string, input: TurnInput): Promise<TurnResult>;
  state(conversationId: string): Promise<ConversationSnapshot>;
  /** One Provider AI: contas de CLI (administração). */
  cliConnections(): Promise<CliConnection[]>;
  createCliConnection(input: { name: string; engine: CliEngine; executor_id: string }): Promise<unknown>;
  startCliAction(id: string, action: CliAction): Promise<CliSessionView>;
  cancelCliSession(sessionId: string): Promise<void>;
  submitCliCode(sessionId: string, code: string): Promise<void>;
  setCliUsage(id: string, allowed: boolean): Promise<void>;
  deleteCliConnection(id: string): Promise<void>;
  /** One Provider AI: conexões por API Key (administração). */
  providerConnections(): Promise<ProviderConnection[]>;
  createProviderConnection(input: { name: string; provider: ApiProvider; api_key: string; default_model?: string }): Promise<unknown>;
  deleteProviderConnection(id: string): Promise<void>;
  testProviderConnection(id: string): Promise<{ status: string; model_count: number }>;
  providerConnectionModels(id: string): Promise<string[]>;
  updateProviderConnection(id: string, input: { name?: string; default_model?: string; api_key?: string }): Promise<void>;
  orchestratorSettings(): Promise<OrchestratorSettings>;
  saveOrchestratorSettings(input: { enabled: boolean; provider_connection_id: string | null; model: string | null; system_prompt: string; instructions: string }): Promise<void>;
  executorSettings(id: string): Promise<ExecutorSettings>;
  saveExecutorSettings(id: string, input: { cli_connection_id: string; instructions: string }): Promise<void>;
  subscribe(conversationId: string, options: SubscribeOptions): Subscription;
  cancelJob(jobId: string): Promise<void>;
  task(id: string): Promise<CentralTaskDetail>;
  job(id: string): Promise<CentralJob>;
  executors(): Promise<CentralExecutor[]>;
}

/** Erro devolvido pelo OneHub com o código estável (ex.: `no_grant`), nunca com detalhes internos. */
export class OneHubError extends Error {
  status: number;
  code: string;
  constructor(status: number, code: string) {
    super(code);
    this.status = status;
    this.code = code;
  }
}
