# Provider AI Information Architecture Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Provider AI, Executors, and Orchestrator responsibilities clear in the OneCoding Settings experience while preserving all current connection flows.

**Architecture:** Keep Provider AI as the provider connection catalog and credential management page. Keep Executors focused on paired execution workers, with a contextual link to Provider AI. Keep Orchestrator controls honest about unavailable backend support.

**Tech Stack:** React, TypeScript, Vitest, Testing Library, existing OneHub transport.

**Spec:** `docs/superpowers/specs/2026-11-19-provider-ai-executors-boundaries-design.md`

## Global Constraints

- Preserve existing OneHub endpoints, authentication, permissions, and connection states.
- Do not represent saved API keys as connected without a real provider check.
- Do not claim remote CLI installation or Orchestrator prompt/model configuration is implemented.
- Preserve `/configuracoes/provider-ai` compatibility.
- No backend, database, migration, deployment, or dependency changes.

## Review Focus

- A non-admin must not see administrative connection controls; retain existing authorization boundaries.
- API credentials must remain encrypted and absent from UI state after submission; retain existing form behavior.
- A saved but untested API key must remain “Configurada”, never “Conectada”.
- Opening Provider AI from Executors must keep the existing route and provider connection data.
- Orchestrator engine/model/prompt settings must be described as unavailable until real backend support exists.

---

### Task 1: Clarify the three configuration areas

**Files:**
- Modify: `apps/desktop/src/central/SettingsHome.tsx`
- Modify: `apps/desktop/src/central/AdminPanel.tsx`
- Modify: `apps/desktop/src/central/CentralApp.tsx`
- Modify: `apps/desktop/src/central/ProviderPage.tsx`
- Test: `apps/desktop/src/__tests__/central-web.test.tsx`
- Test: `apps/desktop/src/__tests__/provider-page.test.tsx`

**Interfaces:**
- `AdminPanel` receives `onProvider(): void` from `CentralApp` to open the existing Provider AI route.
- Existing `OneHubTransport` methods and Provider AI page behavior remain unchanged.

- [x] **Step 1: Write failing behavior tests**
  - Add a Settings navigation test proving Provider AI remains a separate entry and its description is about CLI/API connections.
  - Add an Executors test proving the page explains its worker role and the contextual action opens `/configuracoes/provider-ai`.
  - Add an Orchestrator test proving motor, modelo, and prompts are explicitly unavailable (no fake configuration controls).
  - Add a Provider AI test proving its page explains executor-hosted CLI authentication and OneHub-encrypted API keys; retain the existing “Conectada” CLI / “Configurada” API status assertions.
- [x] **Step 2: Run the focused tests and confirm they fail for missing behavior**
  - Run: `pnpm --filter @oneagent/desktop test -- --run src/__tests__/central-web.test.tsx src/__tests__/provider-page.test.tsx`
  - Expected: failures only for the new copy/navigation behaviors.
- [x] **Step 3: Implement the minimal UI and navigation changes**
  - Keep separate Settings entries; clarify their descriptions in `SettingsHome.tsx`.
  - Add a contextual Provider AI link in the OneExecutors section through `AdminPanel` and `CentralApp`.
  - Explain the unsupported Orchestrator motor/model/prompt configuration without adding controls.
  - Add concise Provider AI explanatory copy; do not claim that CLIs are remotely installed or API keys have been tested.
- [x] **Step 4: Re-run focused tests**
  - Run: `pnpm --filter @oneagent/desktop test -- --run src/__tests__/central-web.test.tsx src/__tests__/provider-page.test.tsx`
  - Expected: both files pass.
- [x] **Step 5: Run full frontend verification**
  - Run: `pnpm --filter @oneagent/desktop typecheck && pnpm --filter @oneagent/desktop test && pnpm --filter @oneagent/desktop build:web`
  - Expected: typecheck and build succeed; all frontend tests pass.
- [x] **Step 6: Commit the validated change**
  - Commit: `feat(web): clarify provider and executor settings`
