# Provider AI, Orchestrator and Executors Integration Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Persist and connect real AI provider checks, orchestration preferences, and supported executor profiles without duplicating secrets or widening execution permissions.

**Architecture:** Extend OneHub APIs and frontend transport for Provider AI checks/models and persisted configuration. A new additive migration stores global Orchestrator settings and per-OneExecutor profiles. The only task runtime wired in this increment remains Codex, with an authenticated CLI profile, current OneNode/project grants, and read-only snapshots.

**Tech Stack:** Rust Axum, PostgreSQL migrations, reqwest, React/TypeScript, Vitest, Rust integration tests.

**Spec:** `docs/superpowers/specs/2026-11-19-provider-orchestrator-executors-integration-design.md`

## Global Constraints

- Preserve encrypted API-key storage; secrets never return to the browser or logs.
- Preserve OneNode/project grants and the read-only Codex snapshot boundary.
- Never mark an untested API key connected; never fall back to another CLI account.
- Orchestrator settings stay disabled while `UnavailableProvider` is active.
- Configure only a CLI profile on its owning OneExecutor; currently only Codex analysis jobs are supported.
- Do not install CLIs remotely, enable shell/write, alter client VPSs, or add Git/Worktree/VibeAgent runtime.
- Deploy to production only after isolated database tests, backup/restore verification, and service/artifact checks.

## Review Focus

- API responses, errors, audit records, and HTTP logs contain no API key or provider response body.
- Revoked or disconnected provider connections are not usable by Orchestrator or Executor configuration.
- CLI login/auth state is account-specific and tied to the correct OneExecutor profile.
- OneExecutor receives only its own selected connection ID and cannot select another executor's profile.
- Old executors that do not support profile-bound jobs cannot be given a configured profile job.
- An Orchestrator config cannot be enabled while its inference runtime is unavailable.
- Model selection accepts only IDs returned by a successful provider model check.
- Existing users, Workspaces, conversations, OneNode grants, and chat permissions remain unchanged.

---

### Task 1: Provider API check, models, and credential edit

**Files:**
- Modify: `crates/one-central/Cargo.toml`
- Create: `crates/one-central/src/provider_http.rs`
- Modify: `crates/one-central/src/routes.rs`
- Modify: `apps/desktop/src/transport/types.ts`
- Modify: `apps/desktop/src/transport/http.ts`
- Modify: `apps/desktop/src/central/ProviderPage.tsx`
- Test: `crates/one-central/tests/central.rs`
- Test: `apps/desktop/src/__tests__/provider-page.test.tsx`

**Interfaces:**
- `POST /v1/provider-connections/{id}/test` authenticates the key upstream, stores `connected`/`error` plus `last_checked_at`, returns safe status only.
- `GET /v1/provider-connections/{id}/models` returns provider model IDs only after a successful upstream request.
- `PATCH /v1/provider-connections/{id}` updates name/default model and optionally replaces a key; never returns the key.
- `OneHubTransport` adds `testProviderConnection`, `providerModels`, and `updateProviderConnection`.

- [ ] **Step 1: Add failing backend and frontend behavior tests** for successful/invalid provider checks, model listing, key non-disclosure, key update, and UI feedback.
- [ ] **Step 2: Run focused tests and confirm they fail for missing endpoints/controls.**
- [ ] **Step 3: Implement provider-specific OpenRouter/OpenAI/Anthropic/Gemini requests** with a bounded timeout/body, authorization headers, safe errors, and no secret-bearing logs or URLs.
- [ ] **Step 4: Implement tests and frontend controls** for Testar, Modelos, Editar and Revogar; keep API state “Configurada” until test succeeds.
- [ ] **Step 5: Run provider backend/frontend tests.**
- [ ] **Step 6: Commit `feat(provider-ai): test API connections and list models`.**

### Task 2: Real CLI installation/auth status

**Files:**
- Modify: `crates/one-central/src/cli_auth.rs`
- Modify: `crates/one-central/src/executor.rs`
- Modify: `crates/one-executor/src/engines.rs`
- Modify: `apps/desktop/src/transport/types.ts`
- Modify: `apps/desktop/src/central/ProviderPage.tsx`
- Test: `crates/one-central/tests/central.rs`
- Test: `crates/one-executor/tests/*`
- Test: `apps/desktop/src/__tests__/provider-page.test.tsx`

**Interfaces:**
- CLI connection listing includes only the actual OneExecutor motor report: installed, approved, version and reported timestamp alongside account-specific authentication.
- CLI session creation fails closed with a clear code unless that engine is installed and approved on the associated executor.
- The OneExecutor reports only engines it can safely inspect; absence is “não reportado”, not installed/authenticated.

- [ ] **Step 1: Add failing tests** for report-to-account association, stale/missing report, and refusal to start login for an uninstalled/unapproved engine.
- [ ] **Step 2: Verify the tests fail for missing live installation gating/report data.**
- [ ] **Step 3: Extend supported engine probing and backend validation** without adding remote installation or new approvals.
- [ ] **Step 4: Show separate installation, approval and authentication states in Provider AI.**
- [ ] **Step 5: Run targeted Central/OneExecutor/frontend tests.**
- [ ] **Step 6: Commit `feat(provider-ai): report real CLI readiness`.**

### Task 3: Persist Orchestrator and Executor configuration

**Files:**
- Create: `crates/one-central/migrations/0012_provider_orchestrator_executor_settings.sql`
- Modify: `crates/one-central/src/routes.rs`
- Modify: `crates/one-central/src/executor.rs`
- Modify: `apps/desktop/src/transport/types.ts`
- Modify: `apps/desktop/src/transport/http.ts`
- Modify: `apps/desktop/src/central/AdminPanel.tsx`
- Modify: `apps/desktop/src/central/CentralApp.tsx`
- Test: `crates/one-central/tests/central.rs`
- Test: `apps/desktop/src/__tests__/central-web.test.tsx`

**Interfaces:**
- Orchestrator config GET/PUT persists `enabled`, API connection ID, verified model ID, system prompt, instructions and bounded limits. `enabled=true` is rejected while the configured inference runtime is unavailable.
- Executor config GET/PUT is scoped to an executor and persists its own authenticated CLI connection ID plus supported model override/instructions. Backend verifies engine, same executor, auth state, report readiness, and existing directory grants; it does not add paths or permissions.
- The executor profile API never returns credential data.

- [ ] **Step 1: Add failing isolated-PostgreSQL tests** for owner/admin authorization, valid and invalid provider references, model compatibility, same-executor CLI binding, persistence, revocation effects and disabled Orchestrator runtime.
- [ ] **Step 2: Run the tests against the project’s isolated `_test` database and confirm expected failures.**
- [ ] **Step 3: Add migration 0012 and authenticated GET/PUT endpoints** with constraints, audited changes and safe validation.
- [ ] **Step 4: Add React forms and real transport calls** for Orchestrator and executor settings; no controls for unsupported APIs/engines.
- [ ] **Step 5: Run targeted Central and frontend tests.**
- [ ] **Step 6: Commit `feat(central): persist orchestrator and executor profiles`.**

### Task 4: Bind configured CLI profile to Codex jobs

**Files:**
- Modify: `crates/one-central/src/executor.rs`
- Modify: `crates/one-executor/src/central.rs`
- Modify: `crates/one-executor/src/job.rs`
- Modify: `crates/one-executor/src/codex.rs`
- Modify: `crates/one-executor/src/agent.rs`
- Modify: `crates/one-adapters/src/host.rs` only if a testable profile-bound host needs an interface change.
- Test: `crates/one-central/tests/central.rs`
- Test: `crates/one-executor/tests/*`

**Interfaces:**
- A configured job contains only the executor’s selected CLI connection ID, model override and bounded instructions; never credentials or arbitrary paths.
- The OneExecutor resolves the ID strictly beneath its configured profile root, uses that isolated HOME, and rejects absent/mismatched profile data without falling back to the process account.
- Jobs remain Codex-only and read-only.

- [ ] **Step 1: Add failing Central/OneExecutor tests** proving the selected profile is sent and used, another account is not selected, and missing/revoked bindings fail closed.
- [ ] **Step 2: Verify expected failures.**
- [ ] **Step 3: Bind the profile to job leasing and run Codex using the existing isolated profile directory.**
- [ ] **Step 4: Verify current read-only snapshot, cancellation, OneNode grant and executor lease tests still pass.**
- [ ] **Step 5: Commit `feat(executor): run jobs with configured CLI profile`.**

### Task 5: Full validation and authorized production release

**Files:**
- Verify only: all files from Tasks 1–4, release/deploy configuration and migration 0012.

- [ ] **Step 1: Run all Rust unit/integration tests, desktop typecheck/tests/build and contract checks.**
- [ ] **Step 2: Run ignored Central integration tests only against the isolated `_test` database; verify migration 0012 and prior conversation/Workspace isolation.**
- [ ] **Step 3: Inspect the deployment procedure, back up production database and frontend/Central artifacts, and verify the backup restore.**
- [ ] **Step 4: Publish migration, Central and frontend in the compatible order; update only the OneExecutor service if required and explicitly not any client OneNodes.**
- [ ] **Step 5: Smoke-test public Central health, Web assets/routes and connection endpoints; verify service logs contain no credentials. Roll back services on critical failure; do not restore the database over new writes.**
- [ ] **Step 6: Commit any release-safe fix locally and report release, migration, validation and concrete remaining runtime limitations.**
