# rara-codex-oauth — Agent Guidelines

## Purpose

OpenAI Codex OAuth integration — handles the full PKCE OAuth flow for obtaining and refreshing Codex API tokens, with file-based persistence and an ephemeral local callback server. Also provides `CodexCredentialResolver` for automatic token refresh on every LLM request.

## Architecture

### Key module

- `src/lib.rs` — The entire crate. Contains:
  - OAuth URL construction (`build_auth_url`) with PKCE challenge.
  - Authorization code exchange and token refresh (`exchange_authorization_code`, `refresh_tokens`).
  - Token persistence via file I/O (`load_tokens`, `save_tokens`, `clear_tokens`) — tokens stored at `<config_dir>/codex_tokens.json`.
  - In-process pending OAuth state (`PendingCodexOAuth`) stored in a `LazyLock<Mutex>`.
  - Ephemeral callback server on `localhost:1455` (`start_callback_server`) that captures the OAuth callback, exchanges tokens, and redirects to the frontend.
  - `CodexCredentialResolver` — implements `LlmCredentialResolver` from `rara-kernel`. On each call, loads tokens from disk, refreshes if expired, and returns a fresh `LlmCredential`.

### OAuth flow

1. Frontend calls `/start` endpoint (in `rara-backend-admin`).
2. Backend generates PKCE verifier/challenge, saves pending state, starts callback server, returns auth URL.
3. User authorizes in browser, redirected to `http://localhost:1455/auth/callback`.
4. Callback server validates state, exchanges code for tokens, saves to file, redirects to frontend settings page.

### Constants

- Client ID: `app_EMoamEEZ73f0CkXaXp7hrann` (overridable via `RARA_CODEX_CLIENT_ID` env var).
- Redirect URI: `http://localhost:1455/auth/callback` (fixed, registered with OpenAI).
- Token file: `<config_dir>/codex_tokens.json` (via `rara_paths::data_dir()`).

## Critical Invariants

- The redirect URI is hardcoded and must match the OAuth client registration — do not change it.
- Only one callback server can run at a time — starting a new one cancels the previous one.
- Refresh token may be omitted by the token endpoint — always preserve the previous value when missing.
- Token expiry includes a 60-second skew to trigger refresh before actual expiration.
- `CodexCredentialResolver` reads tokens from disk on every `resolve()` call — do not cache tokens in memory across requests.

## What NOT To Do

- Do NOT change the callback port (1455) — it is registered with the OAuth provider.
- Do NOT skip PKCE — the Codex OAuth client requires S256 code challenge.
- Do NOT cache tokens in memory in `CodexCredentialResolver` — always read from disk to pick up fresh tokens from OAuth callback.

## Dependencies

**Upstream:** `rara-kernel` (LlmCredentialResolver trait), `rara-paths` (data directory), `reqwest` (HTTP token exchange), `axum` (callback server).

**Downstream:** `rara-backend-admin` (exposes OAuth start/status/clear endpoints), `rara-app` (registers CodexCredentialResolver in boot).
