# rara-keyring-store — Agent Guidelines

## Purpose

Credential store abstraction backed by the OS keyring (macOS Keychain, Linux Secret Service) — provides a `KeyringStore` trait and a default implementation using the `keyring` crate.

## Architecture

### Key module

- `src/lib.rs` — The entire crate. Defines:
  - `KeyringStore` trait — async `load`/`save`/`delete` operations keyed by `(service, account)` pairs.
  - `DefaultKeyringStore` — delegates to OS keyring via the `keyring` crate.
  - `Error` enum — `Keyring` (OS keyring errors) and `Pg` (database errors, used by `rara-pg-credential-store`).
  - `KeyringStoreRef` = `Arc<dyn KeyringStore>`.

### Design

The trait is intentionally simple (3 methods) so alternative backends (e.g. SQLite-backed `PgKeyringStore`) can implement it. `NoEntry` from the keyring crate is translated to `Ok(None)` rather than an error.

## Critical Invariants

- `NoEntry` is not an error — it means the credential does not exist. Always returns `Ok(None)`.
- The `Pg` error variant is defined here (not in `pg-credential-store`) because the trait's `Result` type is shared across implementations.

## What NOT To Do

- Do NOT log credential values — only log metadata (service, account, value length).
- Do NOT add business logic — this is a pure storage abstraction.

## Dependencies

**Upstream:** `keyring` (OS keyring access), `snafu`, `async-trait`.

**Downstream:** `rara-codex-oauth` (token persistence), `rara-pg-credential-store` (SQLite backend), `rara-app` (selects backend at boot).
