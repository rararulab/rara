# rara-codex-oauth — Agent Guidelines

## Purpose

OpenAI Codex OAuth integration — handles the full PKCE OAuth flow for obtaining and refreshing Codex API tokens, with keyring-backed persistence and an ephemeral local callback server.

## Architecture

### Key module

- `src/lib.rs` — The entire crate. Contains:
  - OAuth URL construction (`build_auth_url`) with PKCE challenge.
  - Authorization code exchange and token refresh (`exchange_authorization_code`, `refresh_tokens`).
  - Token persistence via `KeyringStore` (`load_tokens`, `save_tokens`, `clear_tokens`).
  - In-process pending OAuth state (`PendingCodexOAuth`) stored in a `LazyLock<Mutex>`.
  - Ephemeral callback server on `localhost:1455` (`start_callback_server`) that captures the OAuth callback, exchanges tokens, and redirects to the frontend.

### OAuth flow

1. Frontend calls `/start` endpoint (in `rara-backend-admin`).
2. Backend generates PKCE verifier/challenge, saves pending state, starts callback server, returns auth URL.
3. User authorizes in browser, redirected to `http://localhost:1455/auth/callback`.
4. Callback server validates state, exchanges code for tokens, saves to keyring, redirects to frontend settings page.

### Constants

- Client ID: `app_EMoamEEZ73f0CkXaXp7hrann` (overridable via `RARA_CODEX_CLIENT_ID` env var).
- Redirect URI: `http://localhost:1455/auth/callback` (fixed, registered with OpenAI).
- Keyring service: `rara-ai-codex`, account: `tokens`.

## Critical Invariants

- The redirect URI is hardcoded and must match the OAuth client registration — do not change it.
- Only one callback server can run at a time — starting a new one cancels the previous one.
- Refresh token may be omitted by the token endpoint — always preserve the previous value when missing.
- Token expiry includes a 60-second skew to trigger refresh before actual expiration.

## What NOT To Do

- Do NOT change the callback port (1455) — it is registered with the OAuth provider.
- Do NOT store tokens outside the keyring — they contain sensitive credentials.
- Do NOT skip PKCE — the Codex OAuth client requires S256 code challenge.

## Dependencies

**Upstream:** `rara-keyring-store` (token persistence), `reqwest` (HTTP token exchange), `axum` (callback server).

**Downstream:** `rara-backend-admin` (exposes OAuth start/status/clear endpoints), `rara-app` (provides keyring store).
