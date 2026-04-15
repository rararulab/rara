# fts — Agent Guidelines

## Purpose
SQLite FTS5 full-text index for tape search. A derived index that accelerates
keyword search from O(n×m) brute-force to O(log n) indexed lookup.

## Architecture

```
fts/
├── mod.rs     # Business logic: TapeFts struct, entry filtering, query sanitization
├── repo.rs    # Pure SQL: insert, search, get_hwm, upsert_hwm, delete_by_tape, delete_all
└── AGENT.md
```

- `TapeFts` is owned by `TapeService` as `Option<TapeFts>` — FTS is opt-in.
- `TapeService::with_fts(store, pool)` enables it; `TapeService::new(store)` disables it.
- All SQL lives in `repo.rs` as standalone async functions accepting `&SqlitePool` or `&mut Transaction`.
- `mod.rs` contains only business logic: HWM filtering, Message-kind filtering, text extraction, query sanitization.

### Data flow

**Write path**: `TapeService::append_message()` → JSONL append → `TapeFts::index_entries()` (best-effort)

**Search path**: `TapeService::search()` → try FTS candidates → re-rank with existing scorer → fallback to brute-force

**Lifecycle**: `TapeService::reset()` / `delete_tape()` → JSONL reset → `TapeFts::remove_tape()`

## Critical Invariants

- **JSONL is source of truth** — FTS is a derived index. Deleting the FTS DB is always safe; it rebuilds on next search via lazy backfill.
- **Text surface must match** — `extract_fts_content()` delegates to `service::extract_searchable_text()`. If you change what text the brute-force path searches, the FTS index must match. Do NOT create a separate extraction function.
- **Lifecycle cleanup is mandatory** — every code path that clears JSONL tape data MUST also call `TapeFts::remove_tape()`. Violation leaves stale FTS rows that return ghost results.
- **All FTS operations are best-effort** — errors are logged and swallowed, never propagated. The system must work identically without FTS.

## What NOT To Do

- Do NOT put `sqlx::query` calls in `mod.rs` — all SQL goes in `repo.rs`.
- Do NOT make FTS failures break the search path — always fall through to brute-force.
- Do NOT index non-Message entries — only `TapEntryKind::Message` is searchable.
- Do NOT reset the HWM to the max *indexed* ID — set it to the max *seen* ID, so skipped non-Message entries are not re-scanned.

## Dependencies

- **Upstream**: `sqlx::SqlitePool` from `rara-model` (shared pool), `service::extract_searchable_text`
- **Downstream**: consumed by `TapeService` in `service.rs`
- **Schema**: `tape_fts` (FTS5 virtual table) + `tape_fts_meta` created by migration `20260415042041_tape_fts_init`
