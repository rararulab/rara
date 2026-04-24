# fts — Agent Guidelines

## Purpose
SQLite FTS5 full-text index for tape search. A derived index that accelerates
keyword search from O(n×m) brute-force to O(log n) indexed lookup.

## Architecture

```
fts/
├── mod.rs        # Business logic: TapeFts struct, entry filtering, query sanitization
├── repo.rs       # Pure SQL: insert, search, get_hwm, upsert_hwm, delete_by_tape, delete_all
├── tokenizer.rs  # Application-layer jieba pre-segmentation for CJK text
└── AGENT.md
```

- `TapeFts` is owned by `TapeService` as `Option<TapeFts>` — FTS is opt-in.
- `TapeService::with_fts(store, pool)` enables it; `TapeService::new(store)` disables it.
- All SQL lives in `repo.rs` as standalone async functions accepting `&mut DieselSqliteConnection`.
- `mod.rs` contains only business logic: HWM filtering, Message-kind filtering, text extraction, query sanitization.

### Data flow

**Write path**: `TapeService::append_message()` → JSONL append → `TapeFts::index_entries()` (best-effort)

**Search path**: `TapeService::search()` → try FTS candidates → re-rank with existing scorer → fallback to brute-force

**Lifecycle**: `TapeService::reset()` / `delete_tape()` → JSONL reset → `TapeFts::remove_tape()`

## Critical Invariants

- **JSONL is source of truth** — FTS is a derived index. Deleting the FTS DB is always safe; it rebuilds on next search via lazy backfill.
- **Symmetric segmentation** — indexed content and user queries MUST both pass through `tokenizer::segment`. `extract_fts_content` and `sanitize_fts_query` are the only two callers; any new write/query path must do the same or CJK results silently diverge.
- **FTS text surface is narrower than brute-force** — `extract_fts_content` collects only JSON string leaves from payload + metadata (keys and structural punctuation are dropped so jieba doesn't tokenize JSON syntax). The brute-force path keeps the richer `service::extract_searchable_text` output. This asymmetry is intentional; do not "simplify" by merging them.
- **Lifecycle cleanup is mandatory** — every code path that clears JSONL tape data MUST also call `TapeFts::remove_tape()`. Violation leaves stale FTS rows that return ghost results.
- **All FTS operations are best-effort** — errors are logged and swallowed, never propagated. The system must work identically without FTS.
- **Segmentation runs on the blocking pool** — `index_entries` wraps jieba in `spawn_blocking`. Do NOT call `tokenizer::segment` directly in async hot paths on long inputs.

## What NOT To Do

- Do NOT put diesel query calls in `mod.rs` — all SQL goes in `repo.rs`.
- Do NOT make FTS failures break the search path — always fall through to brute-force.
- Do NOT index non-Message entries — only `TapEntryKind::Message` is searchable.
- Do NOT reset the HWM to the max *indexed* ID — set it to the max *seen* ID, so skipped non-Message entries are not re-scanned.

## Dependencies

- **Upstream**: `yunara_store::DieselSqlitePool` (shared diesel-async pool), `jieba-rs` (dictionary ~7 MB, one-time load)
- **Downstream**: consumed by `TapeService` in `service.rs`
- **Schema**: `tape_fts` (FTS5 virtual table) + `tape_fts_meta`, created by migration `20260415042041_tape_fts_init` and rebuilt by `20260418182710_tape_fts_rebuild_jieba` to re-index under the jieba-segmented surface
