# domain — Agent Guidelines

## Purpose

Workspace group containing cross-domain contracts: shared traits, type conversions, and well-known constants used across crate boundaries.

## Sub-Crates

| Crate | Purpose |
|-------|---------|
| `shared` | Cross-domain contracts: `SettingsProvider` trait, time conversion helpers (`chrono` ↔ `jiff`), well-known setting keys |

## Architecture

`domain/shared` defines **interfaces only** — no implementations live here. Implementations belong in the crates that own the data (e.g., `yunara-store` for settings storage).

## Critical Invariants

- `SettingsProvider` trait is the single interface for key-value settings across the system — all settings access must go through this trait.
- Time conversion between `chrono` (DB layer) and `jiff` (domain layer) must use the helpers in `convert.rs` — do NOT convert manually to avoid timezone bugs.
- Setting key constants in `settings/keys.rs` must be kept in sync with the YAML config schema.

## What NOT To Do

- Do NOT add implementations in `domain/shared` — it defines contracts only.
- Do NOT add crate-specific types here — only types that cross 2+ crate boundaries belong in domain.
- Do NOT depend on kernel, integration, or extension crates — domain is a low-level dependency.

## Dependencies

- **Downstream**: Kernel, integrations, extensions consume these contracts.
- **Upstream**: `chrono`, `jiff`, `async_trait`, `tokio` (watch channels for settings).
