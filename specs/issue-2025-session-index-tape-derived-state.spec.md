spec: task
name: "issue-2025-session-index-tape-derived-state"
inherits: project
tags: ["kernel", "sessions", "memory", "tape", "backend"]
---

## Intent

`GET /api/v1/chat/sessions?limit=50&offset=0` returns `SessionEntry` rows
whose tape-derived state fields are wrong on every row:

- `message_count` is always `0`
- `preview` is always `null`
- `updated_at` equals `created_at` (i.e. it never advances when new
  messages are appended to the tape)

Reproducer for "what bug appears if we don't do this":

1. On the running remote (`raratekiAir`, `10.0.0.183:25555`):
   `curl -s http://10.0.0.183:25555/api/v1/chat/sessions?limit=50&offset=0`.
2. Pick any returned `key`. Inspect the corresponding tape file under
   `/Users/rara/Library/Application Support/rara/memory/tapes/<hash>__<key>.jsonl`
   on the remote — it has hundreds to thousands of `Message`-kind lines.
3. Compare: response says `"message_count": 0`, tape file holds N>0
   messages. `updated_at` in the response equals `created_at` from the
   moment the session was created weeks ago, even though new turns landed
   five minutes ago.
4. Bad outcome: the web frontend (and any other API consumer — telegram
   commands historically hit the same bug, see the prior-art notes
   below) cannot show "this session has 47 messages, last touched 3
   minutes ago, currently estimated at 12k context tokens". The whole
   left-rail session list becomes a list of opaque, identical-looking
   names with no live state. The user cannot answer "where am I in the
   context window for this conversation?" without manually opening the
   chat — exactly the kind of "trust me" black box that the goal.md
   north star explicitly disallows (signal 4).

Why the existing protections do not catch this:

- PR 441 (`3c8b43a2 fix(channels): derive session message count from tape
  instead of stale field`) faced the same root cause — `message_count`
  initialized to `0` and never updated — and patched the read path of
  the **telegram** `/sessions` command by counting `Message` entries
  on every read via a new `TapeService::message_count()`. That fix
  never extended to the HTTP API surface, and even where it did apply
  it only addressed `message_count`, not `preview` / `updated_at` /
  the token-context fields. The chat HTTP API still returns the raw
  stored field.
- The full set of tape-derived state already exists in
  `TapeService::info()` (`crates/kernel/src/memory/service.rs:507-597`)
  and is consumed by `tape-info` tool + telegram dashboards. The
  HTTP `list_sessions` path
  (`crates/extensions/backend-admin/src/chat/service.rs:514-521` →
  `SessionIndex::list_sessions` → `FileSessionIndex` JSON read) bypasses
  it entirely.
- `FileSessionIndex::list_sessions`
  (`crates/sessions/src/file_index.rs:139-171`) does
  `read_dir + 全量读所有 session JSON + 内存排序 + 分页`, O(N) every
  request, which compounds the "wrong data, slowly" problem.

The fix this spec defines is structural: stop treating `SessionEntry`'s
derived fields as "snapshot at create" and start updating them
synchronously on every tape append, behind a SQLite-backed
`SqliteSessionIndex` so the read path becomes `SELECT … ORDER BY
updated_at DESC LIMIT ? OFFSET ?` and the write path is one `UPDATE`
adjacent to the JSONL append.

A second derived field, `anchors: Vec<AnchorRef>` with persisted
`byte_offset`, is added at the same time. The anchor `byte_offset` is
the architecturally load-bearing piece: today
`TapeFile.index.anchor_by_name` records `Vec` indices into the parsed
in-memory cache (`crates/kernel/src/memory/store.rs:105-149`), not byte
offsets into the file. Cold-start "seek to last anchor and parse only
since-anchor entries" therefore requires parsing the entire JSONL to
locate the anchor first — defeating the optimisation. Persisting
byte offsets in the session index (the only place we already update
on every append) caps the per-session cold-start parse cost at
`tail_size_in_bytes`, which is what the user explicitly called out as
"the core architecture dividend of this change".

Goal alignment:

- Signal 4 (every action is inspectable) — currently
  `GET /chat/sessions` lies about session state. After this change the
  observable signal at the API matches the on-disk tape.
- Signal 1 (process runs for months without intervention) — bounding
  cold-start tape parse cost via persisted anchor byte offsets is a
  prerequisite for "tape file grows for a year and the process still
  starts in O(seconds), not O(parse the whole year)".

Does not cross any "What rara is NOT" line: this is the same
single-user single-process system; SQLite is already the project DB
(diesel + `embed_migrations!` per `crates/rara-model/AGENT.md`).

Hermes positioning: not applicable — Hermes does not expose its
session-state model and we have a concrete engineering reason
regardless (the JSONL-only reproducer above).

### Prior-art search summary

- `gh issue list --search "session index"` / `"tape session
  message_count chat sessions"` — surfaced issue 43 (introduced
  `SessionIndex` + `FileSessionIndex` as JSON files, Mar 2026), issue 44
  (boot-layer integration), issue 441 (the telegram-side derive-on-read
  patch for `message_count`), issue 448 (follow-up to issue 441 for the
  session-detail command), issue 1399 (added SQLite FTS5 alongside JSONL
  for tape-search, shipped Apr 2026 as PRs 1405 / 1406 / 1414 / 1577 /
  1740 / 1737), and issues 1932 / 1958 (subagent session bookkeeping;
  orthogonal but touches `SessionEntry` write paths that this spec must
  not regress).
- `gh pr list --search "SessionIndex / FileSessionIndex"` — confirms
  the same set; no PR has migrated `FileSessionIndex` to a database
  before. PR 1674 (`perf(backend): single FTS query for session
  search`) is the closest read-path performance work and is consistent
  with this change's direction.
- `git log --grep "SessionIndex|SessionEntry|message_count"
  --since=365.days` — no commit has revisited the derived-state
  staleness since issues 441 / 448 patched the telegram read path; the
  HTTP API has carried this bug continuously since issue 43 introduced
  `FileSessionIndex`.
- `git log --grep "FTS5|tape_fts|sqlite"` — confirms the project has
  already accepted "SQLite alongside JSONL" as the right pattern for
  tape-derived secondary indices (FTS5 for search). This spec extends
  the same shape to session state.

Relationship to the prior decision in issues 43 / 44 (the JSON-file
`FileSessionIndex`): #43's stated rationale was "intentionally simple —
no database, no WAL, just files" for "single-node deployments where the
tape subsystem handles message storage". That rationale held while the
index carried only ~6 fields none of which needed indexed sort
(`title`, `model`, …). This spec changes the workload: derived state
must be updated on every tape append (write amplification), and the
read path must serve `ORDER BY updated_at DESC LIMIT/OFFSET`
efficiently on growing N. SQLite — already in the project, already the
chosen secondary-index for the tape (#1399's FTS5) — is the smaller
delta than building a hand-rolled index over JSON files. This is
explicit supersession of #43's choice, not amnesia about it.

## Decisions

1. **Persist tape-derived state synchronously at append time.** The
   single update site is `TapeService::append`
   (`crates/kernel/src/memory/service.rs`). After the underlying
   `FileTapeStore::append` succeeds, `TapeService` updates the owning
   session's index row in the same async task — no debounce, no
   background writer, no event-bus indirection. SQLite single-row
   `UPDATE` on a `WITHOUT ROWID` table keyed by `session_key` is
   ~tens of microseconds; correctness wins over the 100ms debounce
   sketched in the user note. Optional debounce can be added in a
   follow-up if profiling demands it; this spec rejects it for now
   because it complicates the crash-recovery contract below.

2. **`SessionIndex` trait gains one method.** Add
   `update_session_derived(&self, key: &SessionKey, derived:
   &SessionDerivedState) -> Result<(), SessionError>` where
   `SessionDerivedState` is a `bon::Builder` struct carrying the new
   field set (see Decision 4). The existing `update_session` keeps its
   "config fields" semantics (`title`, `system_prompt`,
   `thinking_level`, `model`, `model_provider`, `metadata`) — splitting
   the two halves prevents the append-time hot path from racing the
   user's PATCH-to-rename. The new method is `async fn` with a default
   implementation of `Ok(())` so tests using `InMemorySessionIndex` /
   `NoopSessionIndex` keep compiling without churn.

3. **`TapeService` reaches `SessionIndex` via `SessionIndexRef`** — no
   EventBus. The `TapeService` constructor gains a
   `session_index: SessionIndexRef` parameter. `SessionIndexRef` is
   already defined in `crates/kernel/src/session/mod.rs:216` and is
   already wired through `boot` and `Kernel`, so this is a constructor
   plumbing change, not a new dependency direction. Only tape names
   that match `session_tape_name(<SessionKey>)` shape trigger the
   derived-state update; user-tape and other named tapes are skipped
   silently.

4. **`SessionEntry` field changes:**
   - Keep: `key`, `title`, `model`, `model_provider`, `thinking_level`,
     `system_prompt`, `metadata`, `created_at`.
   - Repurpose `updated_at` semantics: now means "last tape append for
     this session", updated in-band with derived-state writes. Existing
     callers that read `updated_at` continue to compile and now see
     accurate values.
   - Replace `message_count: i64` with `total_entries: i64` (semantic
     shift: counts every `TapEntry`, not just `Message`-kind, mirroring
     `TapeInfo.entries`). The old field is removed, not deprecated;
     a downstream sweep updates the three call sites
     (`backend-admin` chat service serializer, telegram session
     commands, web TS types — the latter is out of scope for this spec
     beyond regenerating the OpenAPI schema). This is a typed-API
     break; rationale is that `message_count` was already lying so
     no consumer was relying on its value.
   - Repurpose `preview: Option<String>` semantics: now sourced from
     the first user-role `Message` entry's text, computed once at
     session creation (existing behaviour) but additionally
     re-populated by the boot-time rebuild path (Decision 7) so old
     sessions with `preview = None` get one. Append-time updates do
     **not** rewrite `preview` once set — preview is "what this
     conversation started as", not a sliding window.
   - Add `last_token_usage: Option<i64>` — last `total_tokens` seen
     in an `llm.run` event payload, mirroring `TapeInfo.last_token_usage`.
   - Add `estimated_context_tokens: i64` — mirroring
     `TapeInfo.estimated_context_tokens`.
   - Add `entries_since_last_anchor: i64` — mirroring
     `TapeInfo.entries_since_last_anchor`.
   - Add `anchors: Vec<AnchorRef>` (see Decision 5). Serialised as a
     SQLite `TEXT` column carrying a JSON array (single column over
     `session_anchors` join table — see Decision 6).

5. **`AnchorRef` schema:**
   ```rust
   pub struct AnchorRef {
       pub anchor_id:               u64,    // tape entry id
       pub byte_offset:             u64,    // file offset of the JSONL line start
       pub name:                    String, // anchor name from payload
       pub timestamp:               DateTime<Utc>,
       pub entry_count_in_segment:  i64,    // entries in [prev_anchor_id+1 .. anchor_id]
   }
   ```
   `byte_offset` is the offset at which the anchor's JSONL record
   begins (i.e. equal to `read_offset` immediately before that
   anchor's `pwrite`). `entry_count_in_segment` is the number of
   entries between the previous anchor (exclusive) and this anchor
   (inclusive), so the UI can render chapter sizes without re-querying.

6. **Anchors as JSON column, not a join table.** Stored as
   `anchors_json TEXT NOT NULL DEFAULT '[]'` on the `sessions` row.
   Rationale: the only query is "give me all anchors for this
   session, in order" — there is no cross-session anchor lookup. A
   join table buys nothing here and makes the append-time update a
   two-statement transaction. Revisit if a real cross-session anchor
   query appears.

7. **New crate / module layout:**
   - `crates/sessions/src/sqlite_index.rs` (new) — `SqliteSessionIndex`,
     impls `SessionIndex` against the existing diesel `DieselSqlitePool`
     from `rara-app`. Constructor takes the pool by `Arc`.
   - `crates/rara-model/migrations/<timestamp>_session_index/{up.sql,
     down.sql}` (new) — creates `sessions`, `session_channel_bindings`
     tables. Naming follows `crates/rara-model/AGENT.md`
     (`just migrate-add session_index_init`).
   - `crates/rara-model/src/schema.rs` — regenerated via
     `diesel print-schema` after the migration is added.
   - `crates/sessions/src/file_index.rs` — kept in-tree but **only**
     used by the boot-time migration path (Decision 9) and unit tests
     of that migration. All boot wiring switches to
     `SqliteSessionIndex`. The dual-implementation drift risk is
     contained because `FileSessionIndex` is no longer reachable from
     the kernel/HTTP write path.

8. **`session_channel_bindings` table** mirrors the existing JSON
   bindings under `{index_dir}/bindings/` (`channel_type TEXT NOT
   NULL`, `chat_id TEXT NOT NULL`, `thread_id TEXT NULL`,
   `session_key TEXT NOT NULL`, `created_at TIMESTAMP`,
   `updated_at TIMESTAMP`, `PRIMARY KEY (channel_type, chat_id,
   thread_id)`, plus an index on `session_key` for the reverse-lookup
   path used by approval-prompt routing).

9. **Boot-time JSON → SQLite migration, idempotent.** On
   `SqliteSessionIndex::ensure_migrated_from(json_index_dir)`:
   - If the SQLite `sessions` table is non-empty → no-op.
   - Otherwise: read every `*.json` in `json_index_dir/` (and
     `bindings/`), `INSERT` into the new tables. Move the legacy JSON
     files into `json_index_dir/legacy/` once the transaction commits
     so a second boot sees an empty `index_dir/*.json` glob and skips
     the migration entirely. The move is not part of the SQL
     transaction; on partial failure (transaction committed but `mv`
     failed mid-way), the next boot's "is the SQLite table empty?"
     check is still false and the `legacy/` dir is unioned with
     whatever didn't move yet, so the second pass simply moves the
     remainder. No data is read twice.
   - The migrated JSON rows carry `total_entries: 0`,
     `entries_since_last_anchor: 0`, `anchors: []`,
     `estimated_context_tokens: 0`,
     `last_token_usage: None`. The next tape append for that session
     fixes them up; `rebuild` (Decision 11) is the bulk-fix path.

10. **Crash-recovery contract.** The tape JSONL is the source of
    truth. The session-index row is a derived cache. On
    `SqliteSessionIndex::new(...).await`, after migrations apply, the
    index runs a **lightweight reconciliation**: for every row in
    `sessions`, compare `total_entries` against
    `TapeService::info(<tape_name>).entries`. If they differ for any
    session, that one session's row is rebuilt from the tape using
    the same logic as the `rebuild` command (Decision 11). The check
    itself is O(N_sessions) `info()` calls, each of which currently
    parses the whole tape — that is the same cost the current
    `list_sessions` path pays on every request, so this is no worse
    than today and only happens once at startup. The rebuild path
    must also re-derive `byte_offset` for every anchor by streaming
    the JSONL and recording the file position at the start of each
    `Anchor`-kind line.

11. **Rescue command.** Add `rara session-index rebuild [--key
    <SessionKey>]` to the existing CLI surface (the rara CLI binary
    already hosts subcommands; one new subcommand under the existing
    structure, no new binary). With `--key`, rebuilds one session;
    without, rebuilds all. Rebuild ignores the existing row's derived
    fields and recomputes from the tape. `title`, `system_prompt`,
    `model`, `model_provider`, `thinking_level`, `metadata` cannot be
    inferred from the tape — rebuild leaves them as whatever is in
    the row today (and `None` for never-seen sessions discovered
    only via tape file scan). Out-of-scope: a separate
    `--from-tapes-only` mode that scans `tapes/` for sessions with
    no index row; this spec does not require rediscovery, only
    repair.

12. **No new YAML config keys.** The SQLite path is already resolved
    via `rara_paths::database_dir()/rara.db`. The `index_dir` argument
    that `FileSessionIndex` takes today becomes the boot-migration
    source path only (still resolved through the existing
    `rara_paths::session_index_dir()`); after migration completes, it
    is unused at runtime.

13. **`anchors` array exposed in the HTTP response.** The
    `GET /api/v1/chat/sessions` and `GET /api/v1/chat/sessions/{key}`
    responses serialize the new `anchors` array verbatim. Frontend
    chapter-timeline and `between` query work is **out of scope** for
    this spec — the new endpoint
    `GET /chat/sessions/{key}/messages?between=anchor_a..anchor_b`
    is a separate follow-up issue, not delivered here.

## Boundaries

### Allowed Changes

- **/crates/kernel/src/session/mod.rs
- **/crates/kernel/src/memory/service.rs
- **/crates/kernel/src/memory/store.rs
- **/crates/sessions/src/sqlite_index.rs
- **/crates/sessions/src/file_index.rs
- **/crates/sessions/src/lib.rs
- **/crates/sessions/src/error.rs
- **/crates/sessions/Cargo.toml
- **/crates/sessions/AGENT.md
- **/crates/rara-model/migrations/**
- **/crates/rara-model/src/schema.rs
- **/crates/app/src/boot.rs
- **/crates/app/src/lib.rs
- **/crates/boot/src/state.rs
- **/crates/boot/src/kernel.rs
- **/crates/extensions/backend-admin/src/chat/service.rs
- **/crates/extensions/backend-admin/src/chat/router.rs
- **/crates/extensions/backend-admin/src/state.rs
- **/crates/cmd/src/chat/mod.rs
- **/crates/cmd/src/main.rs
- **/crates/cmd/src/session_index.rs
- **/crates/channels/src/telegram/commands/kernel_client.rs
- **/crates/kernel/src/session/test_utils.rs
- **/crates/kernel/src/testing.rs
- **/crates/kernel/tests/session_index_tape_derived_e2e.rs
- **/crates/sessions/tests/sqlite_index_migration.rs
- **/specs/issue-2025-session-index-tape-derived-state.spec.md

### Forbidden

- **/crates/kernel/src/agent/**
- **/crates/kernel/src/event_loop/**
- **/crates/kernel/src/llm/**
- **/crates/kernel/src/tool/tape/**
- **/crates/kernel/src/memory/codec.rs
- **/crates/kernel/src/memory/fts.rs
- **/web/**
- **/config.example.yaml
- **/.github/workflows/**

The agent loop, event-loop, LLM driver, and tape tools must not be
touched: the bug is at the index layer, not the writer of tape
entries. The web frontend must not be touched in this PR — exposing
the new `anchors` field in the response is sufficient, frontend work
is a follow-up. No new YAML key is permitted (Decision 12). No
workflow / CI file changes.

## Acceptance Criteria

Scenario: list_sessions returns tape-derived state matching the underlying tape
  Given a SQLite-backed `SqliteSessionIndex` with one session whose tape
    has 47 `Message` entries, 3 `Anchor` entries, and 12 other entries
    (62 entries total), and the most recent entry was appended 5 seconds
    ago
  When the HTTP handler serving `GET /api/v1/chat/sessions?limit=50&offset=0`
    invokes `service.list_sessions(50, 0)`
  Then the returned `SessionEntry` for that key has
    `total_entries == 62`, `anchors.len() == 3`,
    `entries_since_last_anchor` equal to the number of entries appended
    after the last `Anchor`, and `updated_at` within 1 second of the
    last append timestamp
  Test:
    Package: rara-kernel
    Filter: list_sessions_reflects_tape_state

Scenario: appending a message updates the index in-band
  Given a session whose `SessionEntry.total_entries == N` and
    `updated_at == T0`
  When `TapeService::append_message(session_key, msg)` returns Ok
  Then a subsequent `SessionIndex::get_session(session_key)` returns
    `total_entries == N + 1` and `updated_at >= T0` (monotonically
    advanced to the append timestamp, within clock resolution)
  Test:
    Package: rara-kernel
    Filter: append_message_updates_index_in_band

Scenario: appending an anchor records its byte offset and resets the since-anchor counter
  Given a session whose tape file currently has size F bytes and
    `entries_since_last_anchor == K` (K > 0)
  When the agent appends a new `Anchor`-kind entry named "chapter-2"
  Then `SessionIndex::get_session(session_key)` returns an `anchors`
    array whose last element has `byte_offset == F` (the offset where
    the anchor's JSONL line begins), `name == "chapter-2"`,
    `entry_count_in_segment == K`, and the row's
    `entries_since_last_anchor == 0`
  And seeking the JSONL file to that `byte_offset` and decoding one
    line yields a `TapEntry` with `kind == Anchor` and the matching
    `anchor_id`
  Test:
    Package: rara-kernel
    Filter: anchor_append_records_byte_offset_and_resets_segment

Scenario: boot migration moves JSON sessions into SQLite once
  Given an `index_dir` containing N valid `*.json` session files and
    M binding files, and an empty SQLite `sessions` table
  When the process starts and `SqliteSessionIndex::ensure_migrated_from`
    runs
  Then the SQLite `sessions` table contains N rows whose
    `key`/`title`/`created_at`/`model`/`thinking_level`/`metadata`
    fields equal the JSON values, `session_channel_bindings` contains
    M rows, and the original `*.json` files have been moved into
    `index_dir/legacy/`
  And restarting the process triggers no re-migration: the second
    pass observes a non-empty `sessions` table and the JSON files
    are absent from `index_dir/`
  Test:
    Package: rara-sessions
    Filter: boot_migration_is_idempotent

Scenario: crash-recovery rebuild repairs an out-of-sync row
  Given a session whose tape has 10 entries on disk but whose
    SQLite row says `total_entries == 7` and `anchors == []` (simulated
    by directly writing the row to the DB, mimicking a crash that lost
    the last three append-time index updates)
  When `SqliteSessionIndex::new(...)` runs its boot reconciliation
  Then the row's `total_entries` becomes 10, `anchors` matches the
    on-disk anchor sequence with correct byte offsets, and the JSONL
    file on disk is byte-for-byte unchanged
  Test:
    Package: rara-kernel
    Filter: crash_recovery_rebuild_repairs_out_of_sync_row

Scenario: list_sessions read uses a SQL ORDER BY rather than scanning all rows in memory
  Given a SQLite `sessions` table containing 1000 rows
  When `SqliteSessionIndex::list_sessions(50, 0)` runs
  Then the diesel query emitted is equivalent to `SELECT … FROM
    sessions ORDER BY updated_at DESC LIMIT 50 OFFSET 0`
  And `EXPLAIN QUERY PLAN` for that statement reports use of an
    index on `updated_at` (the migration creates `CREATE INDEX
    idx_sessions_updated_at ON sessions(updated_at DESC)`), not a
    full-table sort
  Test:
    Package: rara-sessions
    Filter: list_sessions_uses_updated_at_index

Scenario: rescue command rebuilds a single session from the tape
  Given a corrupt SQLite row for `session_key = K` (zero counts,
    empty anchors) and a healthy tape file for K with 5 anchors
  When the operator runs `rara session-index rebuild --key <K>`
  Then the row for K is replaced with derived state matching the
    tape (5 anchors, correct byte offsets, correct
    `total_entries`), and rows for other sessions are not touched
  Test:
    Package: rara-kernel
    Filter: rebuild_single_session_leaves_others_alone

## Constraints

- Async trait additions follow the existing project pattern
  (`#[async_trait]` + `Send + Sync`, default `async fn` body where
  backwards compatibility is required for in-memory test impls).
- The new `SqliteSessionIndex` must use the shared diesel pool created
  in `rara-app::init_infra()`; do not open a second SQLite connection.
- The migration follows `crates/rara-model/AGENT.md`: created via
  `just migrate-add session_index_init`, paired `up.sql` / `down.sql`,
  `schema.rs` regenerated via `diesel print-schema` and committed
  in the same PR.
- No new YAML config keys (project rule reaffirmed in Decision 12).
  The append-time update is a mechanism, not a deployment-relevant
  knob.
- All new code, comments, and doc-comments in English.
- The new e2e tests use the existing scripted-LLM / in-memory
  `TapeService` test harness; no `testcontainers`, no real LLM.
- `total_entries` replaces `message_count` as a typed-API break.
  The PR must update every call site so the workspace builds with
  no `message_count` references remaining.
