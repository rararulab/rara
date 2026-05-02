spec: task
name: "issue-2043-session-status"
inherits: project
tags: ["web", "kernel", "sessions", "backend"]
---

## Intent

The Chat-page left sidebar (`web/src/components/topology/SessionPicker.tsx`)
lists every session that has ever been created on this rara instance,
ordered by `updated_at DESC`, capped at 50. There is no archive, no
filter, no "this conversation is done" affordance. After a few months
of normal use the rail is dominated by one-off scratch sessions, half-
finished conversations, and worker sessions spawned by past agent runs
that the user has no reason to ever revisit. The active conversations
the user actually returns to are buried in the same list as the
graveyard.

Reproducer for "what bug appears if we don't do this":

1. Open the rara web UI against the remote backend
   (`VITE_API_URL=http://10.0.0.183:25555 bun run dev`, then
   `http://localhost:5173`).
2. Click the Chat tab. The sidebar renders the response of
   `GET /api/v1/chat/sessions?limit=50&offset=0`.
3. Scroll the rail. Every session ever created is there: tests from
   prior weeks, scratch sessions opened to reproduce a bug, worker
   sessions spawned by `spawn_agent` calls, sessions whose tapes have
   three lines and were abandoned. The user has no in-product way to
   say "I am done with this one, hide it."
4. Bad outcome: the user creates a new session to escape the clutter
   rather than returning to an existing live conversation. This
   defeats the "long-running memory accumulated across years" bet
   (`goal.md` signal 5: "Memory survives time"). The product surface
   for that memory — the sidebar — becomes adversarial against it.
   Signal 2 ("the user stops asking ... they expect rara to surface
   the right thing at the right time") fails twice over: rara cannot
   surface the right session when every session is shown with equal
   weight, and the user is forced to do the curation work that rara
   was supposed to absorb.

Goal alignment:

- Signal 2 (the user stops asking, rara surfaces the right thing) —
  hiding archived sessions by default lets the live-conversation
  short-list rise to the top without any further work.
- Signal 5 (memory survives time) — the longer rara runs, the more
  this matters. A status field is the smallest change that keeps the
  sidebar usable on year-old instances.

Does not cross any "What rara is NOT" line: this is one user, one
process, no new integration, no framework generalization. The
inspectability rule (signal 4) is upheld — `status` is a plain column
on the same `sessions` row served by the same HTTP endpoint, with
identical cache and query semantics.

Hermes positioning: Hermes Agent's UI does have a notion of session
state, but its session model is more conversation-scoped than rara's
(rara sessions also include spawned worker sessions). The "two-state
manual archive" decision below is therefore not a Hermes copy — it is
the smallest cut that handles both rara session lifecycles. We have an
engineering reason to ship it independently.

### Prior-art search summary

- `gh issue list --search "session status"` — no prior issue proposes
  a sidebar archive / status field. Closest is issue 1259
  ("agent session stuck in active state") which is about a kernel-level
  run state during a turn, orthogonal to this user-level archive
  concept; the two must not be conflated (Decision 8 below).
- `gh issue list --search "archive session"` — no prior art.
- `gh issue list --search "session sidebar"` — surfaced issue 1785
  ("enrich sidebar session rows with model + messages"), issue 1623
  ("surface TaskRunHistory in ChatSidebar"), issue 2022 ("collapsible
  left sidebar on topology page"), issue 1885 ("hover menu on session
  items with copy id"), issue 1655 ("Cmd+K session search modal"). All
  of these add visible information or actions to the sidebar; none
  proposes hiding sessions. They are compatible with this work
  (status filter operates orthogonally) and are explicitly out of
  scope here.
- `git log --since=60.days --grep="session.*status|archive|stale"` —
  the only recent matches are about kernel run-state ("session stuck
  in active state") and tape persistence ("persist runtime status to
  DB"), neither related.
- `rg "SessionStatus|session_status|archived" crates/ web/src/` —
  no existing definition. The field does not yet exist anywhere in
  the codebase, so this is a green-field add, not a re-introduction.
- `rg "ChatSession" web/src/api/types.ts` — confirms the wire type is
  the place to add the new field. The `SessionEntry` Rust struct
  (`crates/kernel/src/session/mod.rs:152-215`) is the matching
  backend type.

Relationship to PR 2038 / issue 2025: issue 2025 just landed the
SQLite-backed `sessions` table with derived state. That PR is the
schema layer this work plugs into — `status` becomes one more
column on the same row, served by the same `SessionIndex` trait,
written through the existing `update_session` path (Decision 4).
Without 2025 the change would have meant a new JSON file format
or a sidecar table; 2025 makes it a one-column migration.

## Decisions

1. **Taxonomy: two states, `active` and `archived`.** No `done`,
   no `idle`. Reasoning:
   - `done` belongs to task-bound session models (craft, Linear).
     rara's sessions include long-running chats with no terminal
     state and worker sessions whose lifetime is not the user's
     to mark "done".
   - `idle` is an attribute of `updated_at`, not a status the user
     controls. Computing "idle" from a timestamp is a render-time
     concern; making it a column duplicates the source of truth and
     drifts.
   - Two states keep the migration, the API, and the UI cuts each
     to one decision boundary. Adding a third state later is cheap
     because the column is `TEXT` with a `CHECK` constraint that
     can be widened.

2. **Manual only in v1; no auto-archive timer.** The v1 surface is a
   user-controlled archive button per session row plus a sidebar
   toggle to show or hide archived. No background sweeper, no
   "older than N days → archived" rule. Reasoning:
   - Auto-stale has a real failure mode ("I came back to my
     conversation and it was hidden") that erodes trust in the
     surface.
   - Without a status field shipped first, an auto-rule has nothing
     to set. Manual is therefore the prerequisite, and the data
     this MVP collects is the input we would want before designing
     any auto-rule.
   - Auto-archive is filed as a follow-up issue; the boundary
     (Decision 11) keeps the v1 PR honest about not shipping it.

3. **Status lives on the existing `sessions` row, not a sidecar
   table.** New column `status TEXT NOT NULL DEFAULT 'active'`
   with a CHECK constraint `status IN ('active','archived')`.
   Reasoning:
   - The only query is "give me sessions with `status='active'`,
     ordered by `updated_at DESC`, paginated." A sidecar table
     would force a join on every list call.
   - The existing index on `updated_at DESC` is reused; we add a
     partial index `idx_sessions_status_updated_at ON sessions
     (status, updated_at DESC)` so the default-filtered list path
     does not scan archived rows.
   - The `SessionEntry` struct gains one field; serde wire format
     gains one field. No write-path fan-out.

4. **The status update reuses the existing `update_session` path,
   not a new method on the `SessionIndex` trait.** `SessionPatch`
   gains a `status: Option<SessionStatus>` field. `PATCH
   /api/v1/chat/sessions/{key}` carries the new field; the existing
   `apply_session_patch` helper updates it. Reasoning:
   - Mirrors how `title`, `system_prompt`, `model` are mutated
     today. No new endpoint, no new trait method, no fan-out.
   - Keeps the append-time `update_session_derived` path
     (issue 2025) free of user-controlled mutations — status is a
     "config field" in the sense Decision 2 of issue 2025's spec
     defined: it does not race the tape append.

5. **`SessionStatus` is a Rust enum, not a string in the kernel
   layer.** Two variants `Active` and `Archived`, derives
   `Debug + Clone + Copy + PartialEq + Eq + Serialize + Deserialize`,
   serialized lowercase via `#[serde(rename_all = "lowercase")]`.
   Stored as `TEXT` in SQLite (CHECK-constrained). The
   `SqliteSessionIndex` round-trips the enum via match on the column
   string. Default value when reading a row that pre-dates the
   migration: `Active`.

6. **`GET /api/v1/chat/sessions` gains a `status` query parameter.**
   - Default (omitted) → server returns only `status='active'`.
     This is the load-bearing change that makes the sidebar stop
     piling up *for existing clients that do not yet know about the
     parameter*. The legacy clients (telegram, web pre-deploy)
     simply stop seeing archived rows, which is the intended
     outcome.
   - `?status=archived` → returns only archived.
   - `?status=all` → returns both.
   - Invalid value → 400 with the allowed list.

   This is a behaviour change for the unparameterised call. The
   Decisions table in the PR description must call it out
   explicitly. There is no client today that depends on receiving
   archived sessions in the default list (no archived sessions
   exist yet — the field is new), so the break is forward-only.

7. **Web sidebar default is "active only" with a toggle.**
   `SessionPicker` adds:
   - A "Show archived" toggle in the rail header. Off by default;
     persisted to `localStorage` (key `rara.sidebar.showArchived`)
     so a user who turns it on stays on across reloads.
   - A row-level archive control: a hover-revealed button on each
     `SessionPickerItem`. Click sends `PATCH
     /api/v1/chat/sessions/{key}` with `status: 'archived'`. On
     success the item is removed from the active list (and added to
     the archived list when the toggle is on). The active session
     cannot be archived; the button is disabled with a tooltip
     ("Switch to another session first") to keep the post-archive
     "what now?" question off the table for v1.
   - When viewing archived (`showArchived = true`), each row has an
     unarchive button instead of an archive button.

8. **The kernel-level "session run state" (idle / running / streaming
   / cancelled, the thing issue 1259 is about) is NOT this status.**
   Run-state belongs to the live agent loop. `SessionStatus` is a
   user-facing archive bit. The two never share a column, a struct
   field, or a serde alias. If a future spec needs to surface
   run-state in the sidebar, that is a separate field on a separate
   issue.

9. **Spawned worker sessions inherit `Active` at creation.** No
   special-case for parent-child session relationships. A worker
   session can be archived through the same UI as any other session
   once its work is done. This is intentional minimalism for v1; if
   workers create archive churn, a follow-up can wire `status`
   into the spawn-completion path.

10. **Migration: `ALTER TABLE` is acceptable here.** Per
    `crates/rara-model/AGENT.md` and the project's "never modify
    already-applied migrations" rule, this is a *new* migration that
    adds the column with a default. SQLite supports `ALTER TABLE
    ADD COLUMN` for `NOT NULL DEFAULT 'active'` columns. The
    migration must not retroactively touch existing data — every
    existing row becomes `status='active'`, which is the correct
    semantics (no row was archived before this change). The CHECK
    constraint is added at column definition time, but SQLite cannot
    add a CHECK to an existing column via `ALTER`; the column-level
    constraint applies only on the freshly added column, which is
    exactly what we need.

11. **Out of scope for this spec / PR (filed as follow-up issues
    after merge):**
    - Auto-archive based on inactivity timer.
    - Bulk archive operation (multi-select + archive).
    - A separate "Archive" page / view (the toggle is the surface).
    - Search-within-archived (the existing `Cmd+K` search path
      from issue 1655 is independent).
    - Channel-binding behaviour when the bound session is archived
      (telegram / web binding routing logic). Today binding routes
      by `session_key` independent of any status; archiving does
      not break that. If the user archives a session that telegram
      is actively bound to, telegram still resolves the binding —
      that is a follow-up if it surfaces as a complaint.

12. **No new YAML config keys.** The default-filter behaviour is a
    mechanism, not a deployment knob. Project rule reaffirmed.

## Boundaries

### Allowed Changes

- **/crates/kernel/src/session/mod.rs
- **/crates/sessions/src/sqlite_index.rs
- **/crates/sessions/src/file_index.rs
- **/crates/sessions/src/lib.rs
- **/crates/rara-model/migrations/**
- **/crates/rara-model/src/schema.rs
- **/crates/extensions/backend-admin/src/chat/service.rs
- **/crates/extensions/backend-admin/src/chat/router.rs
- **/web/src/components/topology/SessionPicker.tsx
- **/web/src/components/topology/__tests__/SessionPicker.test.tsx
- **/web/src/api/types.ts
- **/web/src/api/sessions.ts
- **/crates/kernel/tests/session_status_e2e.rs
- **/crates/sessions/tests/sqlite_index_status.rs
- **/specs/issue-2043-session-status.spec.md

### Forbidden

- **/crates/kernel/src/agent/**
- **/crates/kernel/src/event_loop/**
- **/crates/kernel/src/memory/**
- **/crates/kernel/src/llm/**
- **/crates/kernel/src/tool/**
- **/crates/channels/**
- **/web/src/pages/Topology.tsx
- **/web/src/components/topology/TimelineView.tsx
- **/web/src/components/topology/WorkerInbox.tsx
- **/web/src/vendor/**
- **/config.example.yaml
- **/.github/workflows/**

The agent loop, memory subsystem, LLM driver, and tape tools must
not be touched: archive status is a metadata field, not a kernel
behaviour. Telegram/channel routing must not be touched (Decision
11). The Topology page shell, TimelineView, and WorkerInbox are
not part of this change — only the SessionPicker sidebar
component. Vendored craft-ui must not be modified (project rule).
No new YAML key (Decision 12). No CI workflow changes.

## Acceptance Criteria

Scenario: list_sessions defaults to status=active and excludes archived rows
  Given a SQLite-backed `SqliteSessionIndex` with three sessions whose
    `status` columns are `active`, `active`, `archived` respectively
  When the HTTP handler invokes `service.list_sessions(limit=50,
    offset=0, status=None)`
  Then the returned `Vec<SessionEntry>` has length 2 and every entry
    has `status == SessionStatus::Active`
  Test:
    Package: rara-sessions
    Filter: list_sessions_default_filters_to_active

Scenario: list_sessions with status=all returns both active and archived
  Given the same three-session fixture as the previous scenario
  When `service.list_sessions(50, 0, status=Some(SessionListFilter::All))`
    runs
  Then the returned vector has length 3 and contains both statuses,
    ordered by `updated_at DESC`
  Test:
    Package: rara-sessions
    Filter: list_sessions_status_all_returns_both

Scenario: PATCH /api/v1/chat/sessions/{key} archives a session via status field
  Given an active session with `key=K`
  When the service receives a `SessionPatch` with
    `status: Some(SessionStatus::Archived)`
  Then `SessionIndex::get_session(K)` returns an entry whose
    `status == SessionStatus::Archived` and `updated_at` advanced to
    the patch timestamp
  And a subsequent `list_sessions(50, 0, status=None)` does not
    include `K`
  And a subsequent `list_sessions(50, 0,
    status=Some(SessionListFilter::Archived))` does include `K`
  Test:
    Package: rara-kernel
    Filter: patch_session_archive_round_trip

Scenario: appending to an archived session does not unarchive it
  Given a session whose `status` is `Archived` and whose tape has 5
    entries
  When `TapeService::append_message` lands a new message on that
    session's tape and the index's `update_session_derived` runs
  Then `SessionIndex::get_session(key)` returns an entry whose
    `status` is still `Archived` and whose `total_entries` is 6
  Test:
    Package: rara-kernel
    Filter: tape_append_preserves_archived_status

Scenario: SessionPicker hides archived rows by default
  Given the `SessionPicker` component is rendered with a fetch result
    of three `ChatSession` rows, two with `status: 'active'` and one
    with `status: 'archived'`, and `localStorage` does not contain
    `rara.sidebar.showArchived`
  When the component mounts
  Then the rendered list contains exactly two `SessionPickerItem`
    nodes (the two active rows) and the archived row is absent from
    the DOM
  Test:
    Package: web
    Filter: SessionPicker hides archived rows by default

Scenario: SessionPicker "Show archived" toggle persists to localStorage and shows archived rows
  Given the same three-session fixture as the previous scenario
  When the user clicks the "Show archived" toggle in the rail header
  Then the rendered list contains all three rows (two active + one
    archived)
  And `localStorage.getItem('rara.sidebar.showArchived')` returns
    the string `'true'`
  And reloading the component (re-mounting with the same fixture)
    keeps the toggle on and the archived row visible
  Test:
    Package: web
    Filter: SessionPicker show-archived toggle persists across remount

Scenario: SessionPicker archive button on a non-active row issues PATCH and removes the row
  Given the `SessionPicker` rendered with three active sessions, none
    selected, and a mocked `PATCH /api/v1/chat/sessions/{key}`
    handler that responds with the same row carrying
    `status: 'archived'`
  When the user hovers a non-active row and clicks its archive button
  Then the `PATCH` request body equals `{ "status": "archived" }`
  And after the response resolves the row is removed from the
    rendered list (only the other two rows remain)
  And the button on the active session row is disabled with the
    tooltip "Switch to another session first"
  Test:
    Package: web
    Filter: SessionPicker archive button removes row and disables on active

Scenario: invalid status query parameter returns 400 with the allowed list
  Given a running backend
  When a client issues `GET /api/v1/chat/sessions?status=banana`
  Then the response status is 400 and the error body lists the
    allowed values `active`, `archived`, `all`
  Test:
    Package: rara-app
    Filter: list_sessions_rejects_unknown_status

## Constraints

- The new column is added via a fresh migration (never modify an
  existing one). Naming follows `crates/rara-model/AGENT.md`: use
  `just migrate-add session_status` to scaffold the pair.
  `schema.rs` is regenerated via `diesel print-schema` and committed
  in the same PR.
- No new YAML config keys (project rule, Decision 12).
- Async trait additions follow the existing project pattern
  (`#[async_trait]` + `Send + Sync`). The `SessionIndex::list_sessions`
  signature gains a `status: SessionListFilter` parameter; default
  trait impl maps to the existing behaviour for in-memory test
  indices (filter applied in-impl).
- All new code, comments, and doc-comments in English.
- Frontend tests use the existing vitest + Testing Library harness
  (see `web/src/components/topology/__tests__/`). No Playwright
  changes.
- Backend e2e tests use the in-memory `TapeService` test harness;
  no `testcontainers`, no real LLM.
- Status is a "config field" in the issue 2025 sense: it goes
  through `update_session`, never through `update_session_derived`.
  The append-time hot path does not touch `status`.
