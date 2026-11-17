// Teste de contrato entre Rust e TypeScript: os fixtures em /protocol/fixtures são os mesmos que o
// Rust (crates/one-protocol) serializa/desserializa. As anotações de tipo abaixo usam os tipos
// GERADOS: se o contrato Rust mudar, `tsc` ou este teste falha.
import gatewayFixture from "../../../../protocol/fixtures/gateway.json";
import runFixture from "../../../../protocol/fixtures/run.json";
import taskFixture from "../../../../protocol/fixtures/task.json";
import runEventFixture from "../../../../protocol/fixtures/run_event.json";
import startFixture from "../../../../protocol/fixtures/start_run_request.json";
import type { GatewayInfo, Run, RunEvent, StartRunRequest, Task } from "../contracts";
import { DEMO_CAPABILITIES, DEMO_GATEWAYS, DEMO_PROJECTS, DEMO_RUNS, DEMO_TASKS } from "../demo/data";

const gateway: GatewayInfo = {
  id: "local",
  name: "Este Mac",
  kind: "local",
  status: "online",
  os: "macos-aarch64",
  protocol_version: 1,
  engines: [
    {
      engine: "codex",
      installed: true,
      path: "/usr/local/bin/codex",
      version: "0.0.0",
      auth: "logged_in",
      trust: "trusted",
      fingerprint: "ab12",
      models: [{ id: "example-model", label: null, source: "catalog", confirmed: true }],
      note: null,
    },
  ],
  demo: false,
};

const task: Task = {
  id: "t1",
  gateway_id: "local",
  project_id: "p1",
  engine: "claude",
  model: null,
  mode: "read_only",
  prompt: "Resuma a arquitetura",
  created_at: "2026-11-17T12:00:00Z",
};

const run: Run = {
  id: "r1",
  task_id: "t1",
  status: "awaiting_approval",
  started_at: null,
  finished_at: null,
  exit_code: null,
  evidence_refs: ["gw://local/runs/r1/log"],
};

const start: StartRunRequest = { workspace_id: "ws1", engine: "codex", model: null, prompt: "Resuma a arquitetura" };
const runEvent: RunEvent = {
  run_id: "r1",
  seq: 3,
  ts: "2026-11-17T12:00:00Z",
  payload: { type: "usage", usage: { input_tokens: 100, output_tokens: 5, cached_tokens: null, reasoning_tokens: null } },
};

describe("contratos Rust ↔ TypeScript", () => {
  it("StartRunRequest bate com o fixture do Rust", () => expect(start).toEqual(startFixture));
  it("RunEvent bate com o fixture do Rust", () => expect(runEvent).toEqual(runEventFixture));
  it("GatewayInfo bate com o fixture do Rust", () => expect(gateway).toEqual(gatewayFixture));
  it("Task bate com o fixture do Rust", () => expect(task).toEqual(taskFixture));
  it("Run bate com o fixture do Rust", () => expect(run).toEqual(runFixture));

  it("dados de demonstração respeitam os tipos e estão marcados", () => {
    expect(DEMO_GATEWAYS.every((g) => g.demo)).toBe(true);
    expect(DEMO_PROJECTS.every((p) => p.demo)).toBe(true);
    expect(DEMO_CAPABILITIES.every((c) => c.demo)).toBe(true);
    // Toda execução de demonstração aponta para uma tarefa existente.
    for (const r of DEMO_RUNS) expect(DEMO_TASKS.some((t) => t.id === r.task_id)).toBe(true);
  });
});
