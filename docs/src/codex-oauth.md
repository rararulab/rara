# Codex OAuth Integration

This document describes how Codex OAuth is layered in the codebase and why.

## Layering

- `crates/integrations/codex-oauth`
  - Owns Codex provider-specific behavior.
  - Builds OAuth URLs and PKCE values.
  - Exchanges authorization code and refresh token at OpenAI token endpoint.
  - Stores/loads tokens using keyring.
  - Stores pending OAuth state in short-lived in-process memory.
  - Implements token refresh policy (`should_refresh_token`).
  - **Runs ephemeral callback server on `localhost:1455`** — the only
    redirect URI accepted by the Codex public OAuth client.

- `crates/extensions/backend-admin/src/settings/codex_oauth.rs`
  - Owns HTTP route wiring only (`/start`, `/status`, `/disconnect`).
  - Translates API requests to integration calls.
  - Must not re-implement token exchange/refresh rules.
  - Does NOT handle the OAuth callback — that lives on port 1455.

- `crates/workers/src/worker_state.rs`
  - Owns runtime orchestration only.
  - Loads tokens, checks refresh window, refreshes through integration crate, saves refreshed tokens, then builds provider client.
  - Must not embed OAuth endpoint details.

## OAuth Flow

1. Frontend calls `POST /api/v1/ai/codex/oauth/start`.
2. Backend creates `state` + PKCE verifier/challenge via integration crate.
3. Backend starts ephemeral callback server on `localhost:1455`.
4. Backend returns `auth_url` — frontend opens it in a new browser tab.
5. User authenticates at OpenAI and is redirected to
   `http://localhost:1455/auth/callback` (pre-registered redirect URI).
6. Ephemeral server validates `state`, exchanges `code` for tokens via
   integration crate, persists tokens in keyring, then redirects the
   browser to the frontend settings page (`/settings?codex_oauth=success`).
7. Ephemeral server shuts itself down.

## Token Refresh Flow

1. Worker selects provider `codex` from runtime settings.
2. Worker loads persisted tokens via integration crate.
3. If `should_refresh_token(expires_at_unix)` is true, worker acquires refresh lock.
4. Worker re-loads token (double-check) and refreshes via integration crate.
5. Worker persists refreshed token and constructs OpenAI provider with latest access token.

## Why localhost:1455?

The Codex public OAuth client (`app_EMoamEEZ73f0CkXaXp7hrann`) only
accepts `http://localhost:1455/auth/callback` as its redirect URI.
This is the same approach used by the official Codex CLI and other
third-party tools (Roo Code, OpenCode, etc.).

## Rationale

- Avoids boundary leakage from integration concerns into `backend-admin` and worker internals.
- Keeps provider-specific behavior in one place for easier maintenance and testing.
- Prevents duplicated OAuth logic and constant drift.

## Environment Variables

- `RARA_FRONTEND_URL`
  - Frontend base URL used for post-OAuth redirects.
  - Defaults to `http://localhost:5173`.
  - In production (shared domain), set to your public URL.
- `RARA_CODEX_CLIENT_ID`
  - Optional override for Codex OAuth client id.
  - Use this when default client id is not accepted in your environment/account.
