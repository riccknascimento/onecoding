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

export interface CentralExecutor {
  id: string;
  name: string;
  engine: string;
  status: "online" | "offline" | "pending" | "revoked";
}

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
  conversations(): Promise<CentralConversation[]>;
  createConversation(title?: string): Promise<{ id: string; title: string }>;
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
