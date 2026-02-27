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

- `crates/extensions/backend-admin/src/settings/codex_oauth.rs`
  - Owns HTTP route wiring only.
  - Translates API requests to integration calls.
  - Returns redirect/status/disconnect responses.
  - Must not re-implement token exchange/refresh rules.

- `crates/workers/src/worker_state.rs`
  - Owns runtime orchestration only.
  - Loads tokens, checks refresh window, refreshes through integration crate, saves refreshed tokens, then builds provider client.
  - Must not embed OAuth endpoint details.

## OAuth Flow

1. Frontend calls `POST /api/v1/ai/codex/oauth/start`.
2. Backend creates `state` + PKCE verifier/challenge via integration crate.
3. Backend persists pending OAuth state in keyring and returns `auth_url`.
4. User authenticates at OpenAI and is redirected to `/api/v1/ai/codex/oauth/callback`.
5. Backend validates `state`, exchanges `code` through integration crate, persists tokens, then redirects to UI success/error page.

## Token Refresh Flow

1. Worker selects provider `codex` from runtime settings.
2. Worker loads persisted tokens via integration crate.
3. If `should_refresh_token(expires_at_unix)` is true, worker acquires refresh lock.
4. Worker re-loads token (double-check) and refreshes via integration crate.
5. Worker persists refreshed token and constructs OpenAI provider with latest access token.

## Rationale

- Avoids boundary leakage from integration concerns into `backend-admin` and worker internals.
- Keeps provider-specific behavior in one place for easier maintenance and testing.
- Prevents duplicated OAuth logic and constant drift.

## Environment Variables

- `RARA_PUBLIC_BASE_URL`
  - Base URL used to build OAuth callback URI.
  - Example: `http://localhost:8000` or your public URL in production.
- `RARA_CODEX_CLIENT_ID`
  - Optional override for Codex OAuth client id.
  - Use this when default client id is not accepted in your environment/account.
