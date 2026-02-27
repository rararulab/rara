# Codex OAuth Provider Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a `codex` AI provider with OAuth connect/disconnect/status flow in Settings UI, and make runtime LLM calls work with token refresh.

**Architecture:** Extend runtime AI settings with Codex OAuth credential fields, add backend OAuth endpoints under settings module, then update LLM provider loader to build provider from Codex access token with refresh-on-expiry. Frontend settings page gets Codex connect/disconnect/status actions and callback toast handling.

**Tech Stack:** Rust (axum, reqwest, async-openai, serde), React + TanStack Query, existing settings KV store.

---

### Task 1: Add settings fields and normalization support

**Files:**
- Modify: `crates/domain/shared/src/settings/model.rs`
- Test: `crates/domain/shared/src/settings/model.rs` (existing tests module)

**Step 1: Write failing tests**
- Add tests that assert:
  - `is_configured()` returns true for provider `codex` only when access token exists.
  - `apply_patch()` and `normalize()` trim/store codex token fields correctly.

**Step 2: Run tests to verify red**
- Run: `cargo test -p rara-domain-shared model::tests:: -- --nocapture`
- Expected: failing assertions for missing codex support.

**Step 3: Minimal implementation**
- Add codex token fields to `AISettings` and `AiRuntimeSettingsPatch`.
- Extend `apply_patch()` + `normalize()` and `is_configured()` for `codex`.

**Step 4: Verify green**
- Run the same test command and ensure pass.

### Task 2: Add Codex OAuth backend endpoints

**Files:**
- Create: `crates/extensions/backend-admin/src/settings/codex_oauth.rs`
- Modify: `crates/extensions/backend-admin/src/settings/mod.rs`
- Modify: `crates/extensions/backend-admin/src/settings/ai.rs`
- Modify: `crates/extensions/backend-admin/src/settings/router.rs` (if needed for route merge)

**Step 1: Write failing tests**
- Add unit tests in `codex_oauth.rs` for:
  - state verification helper behavior
  - callback query error mapping
  - token-expiry calculation helper

**Step 2: Run tests to verify red**
- Run: `cargo test -p rara-app codex_oauth -- --nocapture`

**Step 3: Minimal implementation**
- Implement endpoints:
  - `POST /api/v1/ai/codex/oauth/start`
  - `GET /api/v1/ai/codex/oauth/callback`
  - `GET /api/v1/ai/codex/oauth/status`
  - `POST /api/v1/ai/codex/oauth/disconnect`
- Add OAuth constants and token exchange/refresh helpers.
- Store oauth transient `state/verifier` and tokens via settings patches.

**Step 4: Verify green**
- Run same tests and ensure pass.

### Task 3: Runtime provider loader support for `codex`

**Files:**
- Modify: `crates/workers/src/worker_state.rs`
- Modify: `crates/core/agent-core/src/model.rs`
- Test: `crates/core/agent-core/src/model.rs` tests module

**Step 1: Write failing tests**
- Add test that provider detection recognizes `codex` hint.

**Step 2: Run tests to verify red**
- Run: `cargo test -p agent-core model::tests:: -- --nocapture`

**Step 3: Minimal implementation**
- Add `codex` provider family in model detection.
- In `SettingsLlmProviderLoader`, add `codex` match arm:
  - read token fields
  - refresh when expired/near-expired
  - persist refreshed token via `SettingsSvc::update`
  - build `OpenAiProvider` with codex access token.

**Step 4: Verify green**
- Run target tests plus loader compile check.

### Task 4: Settings frontend integration

**Files:**
- Modify: `web/src/api/types.ts`
- Modify: `web/src/api/client.ts`
- Modify: `web/src/pages/Settings.tsx`

**Step 1: Write failing checks**
- Add TypeScript types and usage first, then run build expecting fail until wiring is complete.

**Step 2: Verify red**
- Run: `cd web && bun run build`

**Step 3: Minimal implementation**
- Add API client methods for codex oauth start/status/disconnect.
- Add providers UI card for codex connection controls.
- Handle callback query params and toast + query invalidation.

**Step 4: Verify green**
- Re-run `cd web && bun run build` successfully.

### Task 5: End-to-end verification

**Files:**
- No new files; verify workspace.

**Step 1: Run Rust verification**
- Run: `cargo test -p rara-domain-shared`
- Run: `cargo test -p agent-core`
- Run: `cargo check -p workers -p rara-app`

**Step 2: Run frontend verification**
- Run: `cd web && bun run build`

**Step 3: Confirm issue traceability**
- Ensure branch name and commits reference `#324`.
