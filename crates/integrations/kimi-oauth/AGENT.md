# rara-kimi-oauth — Agent Guidelines

## Purpose
Read OAuth tokens from kimi-cli's `~/.kimi/credentials/kimi-code.json` and provide
a `LlmCredentialResolver` for the Kimi Code platform API.

## Architecture
- `StoredKimiTokens` — serde struct matching kimi-cli's token JSON format
- `load_tokens()` / `should_refresh_token()` — file I/O + expiry check
- `refresh_tokens()` — POST to `auth.kimi.com/api/oauth/token`
- `KimiCredentialResolver` — implements `LlmCredentialResolver`, returns `LlmCredential` with `X-Msh-*` headers
- `kimi_common_headers()` — builds the 6 required metadata headers

## Critical Invariants
- Token file format MUST match kimi-cli's `OAuthToken.to_dict()` — field names are `expires_at` (float), not `expires_at_unix` (u64).
- `X-Msh-Platform` MUST be `"kimi_cli"` (not `"rara"`) — the server validates this.
- Device ID is read from `~/.kimi/device_id`, NOT generated — must share with kimi-cli.
- Refreshed tokens MUST be written back to the same file so kimi-cli stays in sync.

## What NOT To Do
- Do NOT generate a new device ID — must read kimi-cli's existing one.
- Do NOT change `X-Msh-Platform` to `"rara"` — Kimi server may reject unknown platforms.
- Do NOT use `rara_paths::config_dir()` for token storage — tokens live in `~/.kimi/`.

## Dependencies
- Upstream: `rara-kernel` (for `LlmCredentialResolver` trait)
- Downstream: `rara-app` (registers the driver in `boot.rs`)
- External: kimi-cli must be installed and `kimi auth login` completed
