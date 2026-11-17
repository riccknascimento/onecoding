// DADOS DE DEMONSTRAÇÃO — nada aqui vem de integração real. Toda entrada carrega `demo: true`
// (ou vive neste módulo, que só as telas marcadas com o selo DEMO importam).
import type { Capability, GatewayInfo, Project, Run, Task } from "../contracts";

export const DEMO_GATEWAYS: GatewayInfo[] = [
  {
    id: "demo-apliflow",
    name: "Apliflow (demo)",
    kind: "remote",
    status: "unknown",
    os: "linux-x86_64",
    protocol_version: 1,
    engines: [],
    demo: true,
  },
];

export const DEMO_PROJECTS: Project[] = [
  { id: "demo-p1", name: "SystemFlow (demo)", gateway_id: "demo-apliflow", path: "/srv/systemflow", demo: true },
  { id: "demo-p2", name: "OneAgent (demo)", gateway_id: "local", path: "~/Documents/OneAgent", demo: true },
];

export const DEMO_TASKS: Task[] = [
  {
    id: "demo-t1",
    gateway_id: "demo-apliflow",
    project_id: "demo-p1",
    engine: "claude",
    model: null,
    mode: "read_only",
    prompt: "Localize o projeto, leia a documentação e resuma a arquitetura.",
    created_at: "2026-11-17T12:00:00Z",
  },
  {
    id: "demo-t2",
    gateway_id: "local",
    project_id: "demo-p2",
    engine: "codex",
    model: null,
    mode: "write",
    prompt: "Criar testes dos contratos.",
    created_at: "2026-11-17T12:30:00Z",
  },
];

export const DEMO_RUNS: Run[] = [
  {
    id: "demo-r1",
    task_id: "demo-t1",
    status: "succeeded",
    started_at: "2026-11-17T12:00:05Z",
    finished_at: "2026-11-17T12:03:40Z",
    exit_code: 0,
    evidence_refs: ["gw://demo-apliflow/runs/demo-r1/log"],
  },
  {
    id: "demo-r2",
    task_id: "demo-t2",
    status: "awaiting_approval",
    started_at: null,
    finished_at: null,
    exit_code: null,
    evidence_refs: [],
  },
];

export const DEMO_CAPABILITIES: Capability[] = [
  { id: "demo-c1", kind: "skill", name: "revisao-de-codigo", scope: "global", engine: "claude", version: "1.2.0", source: "local", demo: true },
  { id: "demo-c2", kind: "agent", name: "investigador-de-arquitetura", scope: "project", engine: null, version: "0.3.1", source: "projeto SystemFlow", demo: true },
  { id: "demo-c3", kind: "mcp", name: "supabase", scope: "gateway", engine: "codex", version: null, source: "config do Gateway", demo: true },
  { id: "demo-c4", kind: "command", name: "/plano", scope: "cli", engine: "claude", version: null, source: "~/.claude", demo: true },
];
