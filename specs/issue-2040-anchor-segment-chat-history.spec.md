spec: task
name: "issue-2040-anchor-segment-chat-history"
inherits: project
tags: []
---

## Intent

Long-running sessions accumulate hundreds to thousands of tape entries.
Today the chat-history read path
(`crates/extensions/backend-admin/src/chat/service.rs::list_messages`,
exposed at `GET /api/v1/chat/sessions/{key}/messages`) calls
`TapeService::entries(&tape_name)` which reads and parses the entire
JSONL tape, converts every entry to `ChatMessage`, then keeps only the
trailing `limit`. Every "open this session" click pays O(full tape) on
both disk read and JSON decode. There is also no way to address
"messages between two checkpoints" — the operator can only ask for
"the last N".

Issue 2025 (PR 2038, merged 2026-05-01) just persisted
`AnchorRef.byte_offset` on every `Anchor`-kind tape entry and exposed
the per-session `anchors[]` array on `GET /api/v1/chat/sessions`. Each
`AnchorRef` carries `anchor_id`, `byte_offset` (file position of the
JSONL line where the anchor was written), `name`, `timestamp`, and
`entry_count_in_segment`. That infrastructure is currently unused at
the read end — this spec cashes it in.

The change has two halves that ship together:

1. **Backend.** Extend `GET /api/v1/chat/sessions/{key}/messages` with
   two optional query params, `from_anchor` and `to_anchor`, that
   resolve to byte offsets via the session's persisted `anchors[]` and
   are passed to a new `TapeService::entries_in_byte_range(start, end)`
   that `seek`s into the JSONL file and parses only that segment.
   Existing `?limit=N` behaviour (no anchor params) is preserved
   verbatim for backward compat.
2. **Frontend.** Add a chapter-marker strip alongside the existing
   `web/src/components/topology/TimelineView.tsx` (the chat-history
   pane on the topology page). One marker per anchor, ordered by
   timestamp, showing the anchor `name` truncated and
   `entry_count_in_segment` as a badge. Clicking a marker fetches the
   segment between that anchor and the next one via the new endpoint
   and scrolls the timeline to it.

Reproducer for the negative case (today, with no fix):

1. Open `/topology/<key>` for a session that has been running for a
   week with ~5 anchors and ~800 tape entries.
2. The frontend issues `GET /api/v1/chat/sessions/<key>/messages?limit=200`.
   The backend reads the full ~800-entry tape, decodes every line,
   converts to `ChatMessage`, then keeps the last 200. Server-side
   timing scales O(full tape).
3. The user wants to jump to "the conversation around the anchor named
   `daily-summary-2026-04-28`". There is no UI affordance and no API
   shape that supports this — they must scroll up through 200+ bubbles
   manually, or refresh with a larger `limit` and scroll further.
4. After fix: the timeline pane shows a strip of clickable chapter
   markers, one per `AnchorRef`. Clicking
   `daily-summary-2026-04-28` issues
   `GET /api/v1/chat/sessions/<key>/messages?from_anchor=<id>&to_anchor=<next_id>`,
   the backend `seek`s to `byte_offset` and parses only the segment
   bytes (typically <50 entries), and the timeline scrolls to that
   segment.

### Goal alignment

- **Signal 4 — every action is inspectable.** Anchors are the kernel's
  own checkpoints (session/start, daily summary, fold points). Today
  they are recorded but invisible; this surface lets the operator
  navigate by them, which is the inspectability story.
- **Signal 1 — process runs for months without intervention.** The
  byte-range read makes per-segment fetch cost O(segment), independent
  of total tape length. Without this, the read cost grows with corpus
  size — exactly the decay the engineering bet is supposed to avoid.

Crosses no `goal.md` "What rara is NOT" line. This is the read-side
counterpart to the index work in #2025; it is single-surface (topology
chat history) depth, not multi-surface breadth.

### Decision: param shape — `from_anchor` + `to_anchor`, both optional

Considered three shapes:

- A: `?from_anchor=<id>&to_anchor=<id>` — closed range, both bounds
  explicit. Either bound is optional; absent `from_anchor` means
  "start of tape", absent `to_anchor` means "end of tape".
- B: `?since_anchor=<id>&limit=<n>` — open-ended forward read.
- C: Both, picked at request time.

Pick **A**. The timeline UI knows both bounds when the user clicks a
marker (the clicked anchor and the next one in `anchors[]`). The
"earliest segment" case (no `from_anchor`) and the "most-recent
segment" case (no `to_anchor`) both fall out naturally as one-bound
omissions, no second param shape needed. Shape B is a follow-up if a
future use case (e.g. infinite-scroll forward) actually surfaces it;
adding it now is speculative per `KARPATHY.md` rule 2.

When **both** params are absent, behaviour is byte-for-byte identical
to today's "last `limit` messages" path — the same code path runs, no
new branch. This is the regression-pin we encode in scenario 4.

### Decision: boundary semantics — `[from_anchor.byte_offset, to_anchor.byte_offset)`

Half-open. The line at `to_anchor.byte_offset` IS the to-anchor's own
JSONL line, which the user does NOT want included when they click
"jump to the segment **between** A and B" — that line belongs to
segment B. Including it would cause the to-anchor to render twice if
the user then clicked B. Half-open avoids that.

When `to_anchor` is absent, read from `from_anchor.byte_offset` to
EOF. When `from_anchor` is absent, read from offset 0 to
`to_anchor.byte_offset` (exclusive).

### Decision: error shape

- Unknown `from_anchor` or `to_anchor` ID → 404 with
  `ChatError::SessionError { message: "anchor <id> not found in session <key>" }`.
- `from_anchor.byte_offset > to_anchor.byte_offset` (anchors out of
  order in the request) → 400 with
  `ChatError::InvalidRequest { message: "from_anchor must precede to_anchor" }`.
- All other errors propagate as today.

### Decision: store-layer primitive

Add `FileTapeStore::read_byte_range(tape_name, start: u64, end: Option<u64>) -> TapResult<Vec<TapEntry>>`.
Implementation: open the JSONL file, `seek(SeekFrom::Start(start))`,
read lines until EOF or until the file cursor reaches `end`. Each line
is JSON-decoded into `TapEntry`. The existing `byte_offset` invariant
(captured at append time as the position **before** the write) means
seeking to `byte_offset` lands the reader exactly at the start of that
entry's JSONL line. Surface on `TapeService` as
`entries_in_byte_range(tape_name, start, end)`.

This is the actual primitive that makes the cost story true. The
existing `entries_after(tape_name, after_entry_id)` filters in memory
after a full tape parse — it does NOT seek. We are not reusing it.

### Decision: frontend marker placement

Add `TimelineChapterStrip.tsx` as a sibling of
`TimelineView.tsx` rather than nesting it inside. The two components
share a parent on the topology page and are coordinated via props
(`anchors`, `currentSegment`, `onSelectAnchor`). Avoids growing
`TimelineView`'s already-busy responsibilities. Naming uses "chapter"
to avoid colliding with the existing `TimelineView` name and with the
fork-tree "lineage" concept in `TapeLineageView.tsx` — both are
already established in this codebase under "timeline".

### Prior art

- **PR 2038 / issue 2025.** Persisted `byte_offset` on
  `AnchorRef`, exposed `anchors[]` on the session list. This spec is
  the named follow-up — issue 2025's body explicitly lists "Frontend
  chapter-timeline UI consuming the new `anchors` array" and "New
  `GET /chat/sessions/{key}/messages?between=anchor_a..anchor_b`
  segment-fetch endpoint" as out-of-scope items deferred to a
  separate task.
- **PR 2018 / issue 2013** ("restore topology timeline chat history").
  Wired `TimelineView` to call `GET .../messages?limit=200` on mount
  after the craft-vendor refactor dropped the call. That PR is the
  immediate caller of `list_messages` from the topology page; this
  spec is its natural extension. Not a regression of #2013 — the
  no-params behaviour stays identical.
- **PR 2029 / issue 2022** ("collapsible topology sidebar"). Adjacent
  topology-page work. Touched `Topology.tsx` only; no overlap with
  the timeline pane this spec touches.
- **PR 2003 / issue 1999** ("multi-agent observability UI"). Landed
  the topology shell and `TapeLineageView` (which renders fork-tree
  anchors as labels on edges — a different surface from chat-history
  chapter navigation). Confirms anchors are already a first-class
  visual concept on this page; this spec extends them to the
  chat-history pane.
- `gh issue list --search "anchor timeline"` — issue 2025 (already
  cited), issue 1999 (umbrella), issue 396 (closed dock sidebar
  proposal from 2024, not a re-introduction risk). No conflicting
  prior decision to surface.
- `git log --grep "list_messages|byte_offset" --since=180.days` — only
  PR 2038. No prior implementation of byte-range reads to be aware of.

## Decisions

- **HTTP shape.** `GET /api/v1/chat/sessions/{key}/messages` accepts
  three optional query params: `limit` (existing), `from_anchor` (new,
  `u64` anchor id), `to_anchor` (new, `u64` anchor id). When both
  anchor params are absent, response is identical to today.
- **Boundary semantics.** Half-open
  `[from_anchor.byte_offset, to_anchor.byte_offset)`. Empty bound on
  either side extends to file start / file end respectively.
- **`limit` interaction.** When anchor params are present, `limit` is
  ignored — the segment is bounded by anchors, not by count. Spec'd
  explicitly so an implementer does not invent a "min(segment,
  limit)" rule.
- **Errors.** Unknown anchor id → 404. `from_anchor` ordered after
  `to_anchor` → 400. Missing `from_anchor` with present `to_anchor`
  is valid (read from start). Missing `to_anchor` with present
  `from_anchor` is valid (read to EOF).
- **Store primitive.** New
  `FileTapeStore::read_byte_range(tape_name, start: u64, end: Option<u64>)`.
  Surfaced on `TapeService` as
  `entries_in_byte_range(tape_name, start, end)`. Both methods take
  the byte offsets directly; anchor-id resolution is the HTTP
  handler's job (it has the `SessionEntry.anchors[]` already from the
  session row).
- **Frontend component.** New
  `web/src/components/topology/TimelineChapterStrip.tsx`. Mounted
  alongside `TimelineView` on the topology page. Reads `anchors[]`
  from the existing `chat-sessions` query (already loaded — no extra
  fetch). On click, calls a new `fetchSessionMessagesBetweenAnchors`
  in `web/src/api/sessions.ts` and passes the result to
  `TimelineView` via a `segmentMessages` prop (plus a sentinel that
  tells `TimelineView` to replace its current message list rather
  than merge — keeps the WS-derived live tail clean from
  history-replacement state).
- **No new dependencies.** Use existing `lucide-react` icons,
  existing react-query patterns, existing `cn` helper.
- **No anchor creation / editing UI.** Out of scope; anchors are
  kernel-emitted.

## Boundaries

### Allowed Changes

- **/crates/extensions/backend-admin/src/chat/router.rs
- **/crates/extensions/backend-admin/src/chat/service.rs
- **/crates/extensions/backend-admin/tests/anchor_segment.rs
- **/crates/kernel/src/memory/store.rs
- **/crates/kernel/src/memory/service.rs
- **/web/src/api/sessions.ts
- **/web/src/api/types.ts
- **/web/src/api/__tests__/sessions.test.ts
- **/web/src/components/topology/TimelineChapterStrip.tsx
- **/web/src/components/topology/TimelineView.tsx
- **/web/src/components/topology/__tests__/TimelineChapterStrip.test.tsx
- **/web/src/components/topology/AGENT.md
- **/web/src/pages/Topology.tsx
- **/web/src/pages/__tests__/Topology.test.tsx
- **/specs/issue-2040-anchor-segment-chat-history.spec.md

### Forbidden

- crates/kernel/src/session/**
- crates/rara-model/migrations/**
- crates/extensions/backend-admin/src/chat/error.rs
- crates/extensions/backend-admin/src/chat/snippet.rs
- crates/extensions/backend-admin/src/chat/model_catalog.rs
- web/src/vendor/**
- web/src/components/topology/TapeLineageView.tsx
- web/src/components/topology/SessionPicker.tsx
- web/src/components/topology/WorkerInbox.tsx
- web/src/components/topology/SpawnMarker.tsx
- web/src/components/topology/TurnCard.tsx
- web/src/components/topology/RaraTurnCard.tsx
- config.example.yaml

## Acceptance Criteria

```gherkin
Feature: Anchor-segment chat history endpoint and timeline navigation

  Scenario: Segment read returns exactly the entries between two anchors
    Given a tape with three anchors A1 A2 A3 and entries interleaved between them
      And the session's anchors array carries the persisted byte_offset for each anchor
    When the client requests GET /api/v1/chat/sessions/{key}/messages?from_anchor=A2&to_anchor=A3
    Then the response contains the entries whose tape file position lies in [A2.byte_offset, A3.byte_offset)
      And the JSONL line at A3.byte_offset is NOT included in the response
      And the JSONL line at A2.byte_offset IS included in the response
    Test:
      Package: rara-backend-admin
      Filter: segment_between_two_anchors

  Scenario: Segment read with only from_anchor extends to end of tape
    Given a tape with anchors A1 A2 and entries appended after A2
    When the client requests GET /api/v1/chat/sessions/{key}/messages?from_anchor=A2
    Then the response contains the entries from A2.byte_offset to EOF
      And no upper bound is applied
    Test:
      Package: rara-backend-admin
      Filter: segment_from_anchor_to_eof

  Scenario: Segment read with only to_anchor extends from start of tape
    Given a tape with anchors A1 A2 and entries before A1
    When the client requests GET /api/v1/chat/sessions/{key}/messages?to_anchor=A1
    Then the response contains the entries from byte offset 0 up to A1.byte_offset (exclusive)
    Test:
      Package: rara-backend-admin
      Filter: segment_from_start_to_anchor

  Scenario: No anchor params preserves existing last-N behavior
    Given a tape with N tape entries and any number of anchors
    When the client requests GET /api/v1/chat/sessions/{key}/messages?limit=50
    Then the response is byte-for-byte identical to the response produced before this change
      And no byte-range seek is performed
    Test:
      Package: rara-backend-admin
      Filter: no_anchor_params_preserves_legacy_behavior

  Scenario: Unknown anchor id returns 404
    Given a session whose anchors array does not contain the id 99999
    When the client requests GET /api/v1/chat/sessions/{key}/messages?from_anchor=99999
    Then the response status is 404
      And the error message names both the anchor id and the session key
    Test:
      Package: rara-backend-admin
      Filter: unknown_anchor_returns_404

  Scenario: from_anchor ordered after to_anchor returns 400
    Given a tape with anchors A1 A2 where A1.byte_offset < A2.byte_offset
    When the client requests GET /api/v1/chat/sessions/{key}/messages?from_anchor=A2&to_anchor=A1
    Then the response status is 400
      And the error message states from_anchor must precede to_anchor
    Test:
      Package: rara-backend-admin
      Filter: reversed_anchors_returns_400

  Scenario: Store-layer byte-range read does not parse outside the range
    Given a JSONL tape file whose total size is much larger than the segment between two anchors
    When the kernel calls FileTapeStore::read_byte_range(name, start, Some(end))
    Then the file is seeked to start
      And lines are parsed only until the file cursor reaches end
      And no entry whose JSONL line begins at or after end is returned
    Test:
      Package: rara-kernel
      Filter: read_byte_range_seeks_and_stops_at_end

  Scenario: TimelineChapterStrip renders one marker per anchor
    Given a session whose anchors array has three entries with names N1 N2 N3
    When the topology page renders for that session
    Then the chapter strip contains three markers in the order the anchors appear
      And each marker shows the anchor name (truncated if long) and its entry_count_in_segment as a badge
    Test:
      Package: web
      Filter: renders_marker_per_anchor

  Scenario: Clicking a marker fetches the segment and scrolls TimelineView to it
    Given the topology page is rendered with three anchors A1 A2 A3
    When the user clicks the marker for A2
    Then the frontend issues GET /api/v1/chat/sessions/{key}/messages with from_anchor=A2 and to_anchor=A3
      And the returned messages replace the current TimelineView message list
      And the TimelineView scrolls to the first message of the segment
    Test:
      Package: web
      Filter: click_marker_fetches_and_scrolls

  Scenario: Most-recent anchor click omits to_anchor
    Given the topology page is rendered with anchors A1 A2 A3 where A3 is the most recent
    When the user clicks the marker for A3
    Then the frontend issues GET /api/v1/chat/sessions/{key}/messages with from_anchor=A3 and no to_anchor param
    Test:
      Package: web
      Filter: most_recent_marker_omits_to_anchor
```

## Constraints

- **`agent-spec lifecycle` does not currently support `Package: web`
  selectors** (no vitest adapter; same caveat as issue 2013). The three
  frontend scenarios therefore appear as `uncertain` in the lifecycle
  report; implementer and reviewer verify them by running
  `cd web && bun run test -- TimelineChapterStrip` directly. Tracked as
  a follow-up under issue 2015 ("agent-spec: add vitest adapter for web
  specs").
- All source comments and identifiers in English.
- No new YAML config knobs. The byte-range read is a mechanism
  primitive, not a deployment concern (see
  `docs/guides/anti-patterns.md` "mechanism constants are not config").
- The new store method must not load the full tape into memory before
  filtering. The whole point is bounded I/O.
- No changes to `AnchorRef` schema, no new migrations, no changes to
  `SessionEntry` shape — those landed in #2025 and this spec only
  reads them.
- Backward compat: the no-params response of
  `GET /api/v1/chat/sessions/{key}/messages` is byte-identical to
  pre-change behaviour. Verified by scenario 4.
- New `pub` items on `TapeService` and `FileTapeStore` require `///`
  doc comments.
- Anchor read locking: the byte-range read holds whatever lock
  `TapeService::entries` holds today (no new concurrency primitive).
  Append-during-read remains safe because anchor offsets are persisted
  only after the line is fully written.
