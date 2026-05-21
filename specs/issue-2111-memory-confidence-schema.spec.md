spec: task
name: "issue-2111-memory-confidence-schema"
inherits: project
tags: []
---

## Intent

Today every row in `memory_items` is treated as equally trustworthy.
There is no on-disk place to express "this fact has been contradicted
three times" vs "this fact has been confirmed by the user out loud".
Extraction writes a row, retrieval reads it back, end of story.

That makes rara's recall layer monotonically worse over time as the
corpus grows: contradicted or stale facts (preferences that changed,
plans that were abandoned) compete on equal footing with current ones,
and the only repair tool is hard-deletion via the LLM agent's
extractor — a blunt instrument that loses the lineage of *why* the
deletion happened.

Reproducer for the failure mode this schema enables fixing (the actual
fix lands in issue 2112 / 2113):

1. At T0 the user says "I prefer dark mode". Extractor writes a row.
2. At T+30d the user says "actually light mode now". Extractor writes
   a second row.
3. Retrieval surfaces both rows with no signal that the first is
   superseded. The agent occasionally reaches for the older one in
   contexts where embedding similarity favors it.
4. There is nowhere on disk to record "row 1's last outcome was
   `Contradicted` at T+30d" without inventing freeform JSON in
   `content` — which the extractor would then ingest as text.

This spec lands **only** the storage primitives required for the
confidence + outcome-feedback loop borrowed from AMFS
(<https://github.com/raia-live/amfs>) — a `confidence REAL` column on
`memory_items` and a new append-only `memory_outcomes` table that
records `(item_id, outcome_kind, optional tape_entry_id, created_at)`.
The update logic, the feedback API (`commit_outcome`), and the
retrieval re-ranking all land in issue 2112. The `explain()` trace API
lands in issue 2113. Splitting at this seam keeps each PR's BDD
binding clean: the schema PR's tests are "row defaults to 1.0", "table
exists", "FK enforced"; the service PR's tests are "confidence moves
monotonically under feedback"; the explain PR's tests are "turn → items
mapping round-trips".

Goal alignment: signals 2 ("the user stops asking" — stale facts stop
polluting recall), 4 ("every action is inspectable" — `memory_outcomes`
is the audit log), and 5 ("memory survives time" — the corpus stops
degrading as it grows). Crosses no `NOT` line: still single-user
(`memory_items.username` unchanged), still inspectable (in fact more
so — outcomes table is the explainability substrate), still not a
framework.

Hermes parity: Hermes's memory layer reportedly has feedback signals
but is closed/hosted. rara needs to own this primitive locally because
the inspectability requirement (signal 4) is incompatible with a
black-box recall layer. The engineering reason to do it ourselves is
already in the north star.

Prior art reviewed (raw):

- `gh issue list --search "confidence outcome memory"` — 0 issues.
- `gh pr list --search "confidence memory_items"` — 0 PRs.
- `git log --all --grep=confidence --since=180.days` — only unrelated
  hits (boxlite, agent harness commits). No prior `confidence` work
  on `memory_items`.
- `git log --all --grep=outcome` — only unrelated hits (StepOutcome
  wire enum tests, gateway lifecycle). No prior `memory_outcomes`
  attempt.
- `rg confidence crates/kernel/src/memory/ crates/rara-model/` — 0
  matches. The memory layer has no existing notion of confidence.
- Migrations under `crates/rara-model/migrations/`: most recent is
  `2026-05-01-132410-0000_session_status`. Baseline init lives at
  `20260304000000_init` and currently defines `memory_items` with
  username/content/memory_type/category/source_tape/source_entry_id/
  embedding/created_at/updated_at — confirmed against
  `crates/rara-model/src/schema.rs:142..154`.
- No prior decision to revert. This is greenfield.

## Decisions

- **New diesel migration**, named
  `2026-05-21-000000_memory_confidence_outcomes/{up.sql,down.sql}`.
  Never modify the already-applied `20260304000000_init` — project
  spec rule, also a `__diesel_schema_migrations` checksum issue on
  every deployed instance.
- **`memory_items.confidence`**: `REAL NOT NULL DEFAULT 1.0`. SQLite
  permits adding a `NOT NULL` column with a `DEFAULT`, which
  backfills existing rows to `1.0` — every pre-existing memory item
  starts fully trusted, which matches today's implicit behavior.
- **`memory_outcomes` table** (append-only, no `updated_at`):
  - `id INTEGER PRIMARY KEY AUTOINCREMENT`
  - `item_id INTEGER NOT NULL REFERENCES memory_items(id) ON DELETE CASCADE`
  - `outcome_kind TEXT NOT NULL` — the enum is wire-typed; values
    serialized by issue 2112 are `success`, `failure`, `confirmed`,
    `contradicted`. This spec does not constrain the value space at
    the SQL layer (no `CHECK`); the kernel's `OutcomeKind` enum is
    the single source of truth and is enforced in Rust before
    insert. Rationale: adding `CHECK` here would force a migration
    every time we add a new outcome kind, and `OutcomeKind` is
    deliberately a closed enum in Rust.
  - `tape_entry_id INTEGER NULL` — nullable because not every
    outcome originates from a tape entry (e.g. periodic background
    re-validation).
  - `created_at TEXT NOT NULL DEFAULT (datetime('now'))` — matches
    the existing tables' ISO 8601 convention.
- **Indexes**: `CREATE INDEX idx_memory_outcomes_item_id ON
  memory_outcomes(item_id);` — every read pattern queries by
  `item_id` (for `explain()` and for debugging "what happened to this
  fact"). No composite index — the table is append-only and stays
  small for a long time.
- **`MemoryItem` field**: add `pub confidence: f32` after
  `updated_at`. Wire-format compatibility: it is a public struct
  consumed downstream as JSON via `MemoryTool::exec_search`; adding
  a field is backwards-compatible for serde-with-deny-unknown-fields
  callers (we have none) and forwards-compatible for any frontend
  that ignores unknown keys.
- **`MemoryItemRow` projection**: add the column. The existing
  `Option<i32>` coercion pattern at the id boundary stays; the
  confidence column is `f32` (SQLite REAL maps natively, diesel
  exposes it as `Float`).
- **`NewMemoryItem`**: do NOT add a `confidence` field. New items
  use the SQL DEFAULT (1.0). Rationale: the extractor at
  `crates/kernel/src/memory/knowledge/extractor.rs:141` has no signal
  to set a non-default confidence at insert time. Forcing every
  caller to pass `1.0` is noise; the SQL default is the right
  mechanism.
- **`insert_item`** stays signature-stable. The diesel
  `insert_into(...).values((...))` call lists explicit columns and
  does not mention confidence, so the SQL DEFAULT applies. Verified
  against `crates/kernel/src/memory/knowledge/items.rs:101..114`.
- **`schema.rs` regeneration**: regenerate via
  `diesel print-schema` after the migration applies locally, commit
  the diff in this same PR. `schema.rs` is `@generated`; the diff
  should be limited to the `memory_items` block (new `confidence`
  column) plus a new `diesel::table!` block for `memory_outcomes`
  and the `allow_tables_to_appear_in_same_query!` macro entry.
- **No new Rust type for `OutcomeKind` in this PR.** That enum is
  the natural fit for issue 2112's service layer; defining it here
  would be unused dead code. The SQL column accepts any TEXT; issue
  2112 is the gate that constrains it.
- **`down.sql`** drops the `memory_outcomes` table and recreates
  `memory_items` without the `confidence` column (SQLite cannot
  `DROP COLUMN` until 3.35; we follow the
  rebuild-table-and-copy idiom used elsewhere in the project where
  needed). If down-migration is genuinely irreversible in practice
  (e.g. existing rows have non-default confidence), document that
  the `down` is best-effort.

## Boundaries

### Allowed Changes
- crates/rara-model/migrations/2026-05-21-000000_memory_confidence_outcomes/**
- **/crates/rara-model/migrations/2026-05-21-000000_memory_confidence_outcomes/**
- crates/rara-model/src/schema.rs
- **/crates/rara-model/src/schema.rs
- crates/kernel/src/memory/knowledge/items.rs
- **/crates/kernel/src/memory/knowledge/items.rs
- specs/issue-2111-memory-confidence-schema.spec.md
- **/specs/issue-2111-memory-confidence-schema.spec.md

### Forbidden
- crates/rara-model/migrations/20260304000000_init/**
- crates/rara-model/migrations/2026-04-24-165308-0000_datafeed_drop_feed_read_cursors/**
- crates/rara-model/migrations/2026-05-01-000000_session_index/**
- crates/rara-model/migrations/2026-05-01-132410-0000_session_status/**
- crates/kernel/src/memory/knowledge/service.rs
- crates/kernel/src/memory/knowledge/extractor.rs
- crates/kernel/src/memory/knowledge/embedding.rs
- crates/kernel/src/memory/knowledge/tool.rs
- crates/kernel/src/memory/context.rs
- crates/kernel/src/memory/store.rs
- config.example.yaml
- .github/workflows/**
- docs/**
- web/**

## Completion Criteria

Scenario: new memory item defaults confidence to 1.0
  Test:
    Package: rara-kernel
    Filter: memory::knowledge::items::tests::insert_item_defaults_confidence_to_one
  Given a fresh test DB with the new migration applied
  When insert_item runs against a NewMemoryItem with no confidence field
  Then the row loaded via get_items_by_ids has confidence equal to 1.0

Scenario: pre-existing memory rows backfill to confidence 1.0
  Test:
    Package: rara-kernel
    Filter: memory::knowledge::items::tests::backfill_existing_rows_to_one
  Given a row inserted via raw SQL before the new migration applies
  When the new migration runs and the row is loaded back
  Then its confidence column equals 1.0

Scenario: memory_outcomes table exists after migration
  Test:
    Package: rara-kernel
    Filter: memory::knowledge::items::tests::memory_outcomes_table_accepts_rows
  Given a memory_items row with id N
  When a raw INSERT into memory_outcomes (item_id, outcome_kind, created_at) runs with item_id = N
  Then the row is persisted and queryable by item_id

Scenario: memory_outcomes foreign key cascades on item delete
  Test:
    Package: rara-kernel
    Filter: memory::knowledge::items::tests::memory_outcomes_cascades_on_item_delete
  Given a memory_items row with one memory_outcomes child
  When the parent memory_items row is deleted
  Then the child memory_outcomes row is also gone

## Out of Scope

- `OutcomeKind` enum, `commit_outcome` API, confidence update
  formula — all in issue 2112.
- Re-ranking the embedding search by confidence — in issue 2112.
- Recording which items shaped a given LLM turn (`explain()` API,
  context-sources tape entry) — in issue 2113.
- Exposing confidence or outcome history to the LLM via MCP tool —
  deliberately deferred; we want the feedback loop running internally
  before letting the model self-tune.
- Migrating the extractor at
  `crates/kernel/src/memory/knowledge/extractor.rs` to write
  confidence at insert time. The default is 1.0 and that is correct
  for newly extracted items.
- Backfilling outcomes for historical mistakes. The feedback loop is
  forward-only from the moment issue 2112 lands.
