# rara-domain-shared — Agent Guidelines

## Purpose

Shared domain interfaces, traits, and types for cross-crate contracts — defines the `SettingsProvider` trait and settings key constants used across the application.

## Architecture

### Key modules

- `src/settings/mod.rs` — `SettingsProvider` trait (async `get`/`set`/`delete`/`subscribe` for runtime settings), and `keys` module with string constants for all known settings keys (e.g. `TELEGRAM_BOT_TOKEN`, `LLM_API_KEY`).
- `src/convert.rs` — Cross-domain type conversion utilities.

### `SettingsProvider` trait

The central abstraction for runtime-mutable configuration. Implementations:
- `SettingsSvc` in `rara-backend-admin` (primary implementation, KV-store backed).
- Change notifications via `tokio::sync::watch`.

## Critical Invariants

- Settings keys are string constants defined in `settings::keys` — never use raw strings for key lookup.
- `SettingsProvider` must be `Send + Sync` — it is wrapped in `Arc` and shared across tasks.
- New settings keys must be added to the `keys` module — not scattered across crates.

## What NOT To Do

- Do NOT implement `SettingsProvider` outside of `rara-backend-admin` without good reason — there should be one authoritative implementation.
- Do NOT use raw key strings instead of constants from `settings::keys`.
- Do NOT put concrete implementations here — this crate defines interfaces only.

## Dependencies

**Upstream:** `async-trait`, `tokio` (for watch channels), `serde`.

**Downstream:** `rara-app`, `rara-kernel`, `rara-backend-admin`, `rara-channels` — any crate that reads or writes runtime settings.
