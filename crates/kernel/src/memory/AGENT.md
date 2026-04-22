# memory — Agent Guidelines

## Purpose
Local file-backed tape memory (JSONL timeline per session) plus derived FTS5
index, fork/merge semantics, anchor-based context windowing, and LLM context
building. The authoritative record of every agent turn.

## Architecture

```
memory/
├── mod.rs            # Module docs + TapEntry / TapEntryKind / typed metadata
├── store.rs          # FileTapeStore: JSONL I/O on a dedicated worker thread
├── service.rs        # TapeService: append helpers, search, fork/merge, LLM context
├── context.rs        # default_tape_context(): TapEntry → llm::Message projection
├── anchors.rs        # AnchorSummary, HandoffState
├── fork_metadata.rs  # Fork linkage persisted in TapEntry.metadata
├── tree.rs           # AnchorTree for /fork visualisation
├── error.rs          # TapError (snafu)
├── fts/              # SQLite FTS5 index (see fts/AGENT.md)
└── knowledge/        # Knowledge-graph sub-subsystem
```

- `TapeService::search(tape, query, limit, all_tapes)` — ranked search, optionally
  cross-tape, returns bare `Vec<TapEntry>`. Tape attribution is dropped at the
  boundary.
- `TapeService::search_across_tapes(query, limit)` — cross-tape variant that
  preserves the originating tape. Returns `Vec<TapeSearchHit { entry, tape_name }>`.
  Exists because callers enumerating results per session (e.g. the admin
  session-search endpoint) would otherwise need N sequential `search` calls;
  this collapses to a single FTS query since `fts::FtsHit.tape_name` is already
  tracked in the index. Falls back to a brute-force scan across all tapes when
  FTS is unavailable.

## Critical Invariants

- **JSONL is source of truth** — FTS is a derived index, safe to delete at any time.
- **Tape name == session key** for chat sessions — the cross-tape search relies
  on this so consumers can pass `tape_name` straight into `SessionKey::try_from_raw`.
  User tapes use the `user:` prefix and will fail that parse (skip them).
- **Append-only JSONL** — never rewrite a tape file; fork/merge is how history
  mutates.
- **FTS errors never break search** — always fall through to brute-force.
  `search_across_tapes` follows the same contract.

## What NOT To Do

- Do NOT add fields to `TapEntry` that only make sense for one caller — pollution
  of the universal type; use a wrapper like `TapeSearchHit` instead.
- Do NOT change the signature of `TapeService::search` — it has multiple callers
  across the workspace; add a new method for new shapes.
- Do NOT call `search_brute_force` directly — route through `search` /
  `search_across_tapes` so FTS is always tried first.
- Do NOT skip FTS backfill in new cross-tape query paths — newly-written entries
  are invisible to the index until `backfill_fts` runs.

## Dependencies

- **Upstream**: `rara-paths` (config-driven tape directory), `sqlx` (shared pool
  for FTS), `jieba-rs` (CJK tokenisation).
- **Downstream**: `rara-backend-admin::chat::service::SessionService::search_sessions`
  consumes `search_across_tapes`; the agent loop uses `append_*` + context
  builders during every turn.
