# rara-sessions — Agent Guidelines

## Purpose

Session metadata persistence layer — provides the SQLite-backed implementation of `rara_kernel::session::SessionIndex` plus the one-shot legacy JSON-file migrator. Stores session metadata (title, model, timestamps) and tape-derived state (`total_entries`, `last_token_usage`, `estimated_context_tokens`, `entries_since_last_anchor`, `anchors[]`). Message content lives in tape JSONL files (`rara-kernel::memory`), not here.

## Architecture

### Key modules

- `src/lib.rs` — Crate root, re-exports `file_index`, `sqlite_index`, `types`, and `error`.
- `src/sqlite_index.rs` — **Runtime impl.** `SqliteSessionIndex` implementing `rara_kernel::session::SessionIndex` against the diesel `sessions` and `session_channel_bindings` tables (migration `2026-05-01-000000_session_index`). Anchors are stored as a JSON column (`anchors_json`). Constructed in `crates/app/src/boot.rs` and shared with `TapeService` so append-time derived-state writes go straight to SQL.
- `src/file_index.rs` — **Legacy.** `FileSessionIndex` (JSON-file-per-session). Kept only as the read source for `SqliteSessionIndex::ensure_migrated_from`, which moves files into `<index_dir>/legacy/` after a successful one-shot import. Do NOT add new callers.
- `src/types.rs` — Re-exported session and message types from `rara-kernel`.
- `src/error.rs` — Re-exports `rara_kernel::session::SessionError`.

### Data flow (runtime)

1. Boot (`crates/app/src/boot.rs`):
   1. Construct `SqliteSessionIndex::new(diesel_pools)`.
   2. Call `ensure_migrated_from(json_index_dir)` — idempotent, no-ops when the SQL table is non-empty.
   3. Spawn `reconcile_all(...)` in the background to repair drift between SQL rows and on-disk tape tails (Decision 10). Boot does NOT await it.
   4. Hand `SqliteSessionIndex` to both the kernel (as `dyn SessionIndex`) and `TapeService` (`with_session_index(...)`).
2. Read path (`SessionIndex::list_sessions` / `get_session`): single SQL query, ordered by indexed `updated_at DESC`.
3. Write path (`TapeService::append`): every successful append calls `SessionIndex::update_session_derived(key, derived)` in band — see "Append-time derived-state contract" below.
4. Rescue path (`crates/cmd/src/session_index.rs`): `rara session-index rebuild [--key K]` rebuilds rows from on-disk tapes via the same `ReconcileTape` adapter the boot reconciler uses.

## Critical Invariants

- **The SQL row is the source of truth for session metadata.** UI reads, agent context-window calculations, and listing queries trust the row. The on-disk tape is the source of truth for derived state when reconciling, but any consumer that needs current counts reads the SQL row.
- **Append-time derived-state contract (Decision 1 of issue #2025).** Every `TapeService::append` for a session-keyed tape MUST update the row's `total_entries`, `last_token_usage`, `estimated_context_tokens`, `entries_since_last_anchor`, `anchors[]`, and `updated_at` in band before returning success. A failure to update is logged at warn-level and left for the boot reconciler — never silently swallowed.
- **Boot reconciler closes drift, not correctness.** `SqliteSessionIndex::reconcile_all` only repairs rows whose derived counts disagree with the on-disk tape. Append-time correctness must not depend on it.
- **Migration is one-shot and irreversible per directory.** Once `ensure_migrated_from` succeeds, the source `*.json` files are moved into `<index_dir>/legacy/` and the second invocation observes a non-empty SQL table and short-circuits. Do NOT re-import legacy files manually.
- **`update_session_derived` and `update_session` write disjoint columns.** `update_session_derived` writes the tape-derived columns + `updated_at`. `update_session` writes the user-facing config columns (title, model, etc.). The two paths must not race on the same column — splitting the schema this way is what lets the hot append path coexist with user PATCHes.
- **Anchors are stored as a JSON array column.** Decision 8 of #2025 — avoids a child table + N round-trips per read. Treat it as opaque JSON; round-trip through `serde_json::{from_str, to_string}` only.

## What NOT To Do

- Do NOT add new callers of `FileSessionIndex` — it exists solely as the migration source. New code wires `SqliteSessionIndex` from `crates/app/src/boot.rs`.
- Do NOT add columns to `sessions` without a new diesel migration in `crates/rara-model/migrations/` — modifying an applied migration breaks every deployed instance.
- Do NOT compute derived state lazily ("recompute on read"). The append path owns correctness; read paths are dumb SQL fetches.
- Do NOT block boot on `reconcile_all`. Failures are warn-only; correctness is upheld by append-time writes, not boot timing.
- Do NOT introduce a `rara-sessions → rara-kernel` cycle through `TapeService`. The `ReconcileTape` trait is inverted on purpose — the kernel implements the trait at the `crates/app` boundary.
- Do NOT flatten diesel/runtime errors into `SessionError::FileIo`. Use `SessionError::Database` so ops can distinguish a constraint violation from an actual I/O error.
- Do NOT assume sessions are on the filesystem — they live in SQLite. The `<index_dir>/` path on disk only contains `legacy/` post-migration.

## Dependencies

**Upstream:** `rara-kernel` (for `SessionIndex` trait, `SessionEntry` / `SessionDerivedState` / `SessionKey` types), `rara-model` (diesel schema), `yunara-store` (diesel pools), `diesel-async`.

**Downstream:** `rara-app` (constructs `SqliteSessionIndex` during boot, wires it into `TapeService` and the kernel), `rara-cmd` (`rara session-index rebuild` rescue command).
