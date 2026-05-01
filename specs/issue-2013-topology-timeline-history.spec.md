spec: task
name: "issue-2013-topology-timeline-history"
inherits: project
tags: []
---

## Intent

The topology page's center pane (`web/src/components/topology/TimelineView.tsx`)
no longer loads persisted chat history. After the craft-vendor chat refactor
landed in the multi-agent observability umbrella (PR 2003 / #1999), the
center column renders only (a) optimistic local user turns from
`userTurnsBySession` state and (b) agent turns reduced from the topology
WebSocket. It never fetches the session's tape on mount. Re-opening
`/topology/<existing-key>` for a session with prior conversation shows the
empty-state placeholder until a new live event arrives — historical user
prompts and agent replies are invisible even though they are intact on
disk.

The backend already has the fetch path. `GET /api/v1/chat/sessions/{key}/messages`
(handler at `crates/extensions/backend-admin/src/chat/router.rs:544`,
service at `service.rs:630`) returns `Vec<ChatMessage>` derived from tape
entries, with monotonic `seq`, `role`, `content`, `tool_calls`,
`tool_call_id`, `tool_name`, `created_at`. Each child session has its own
tape (children get a distinct `SessionKey` via `Kernel::spawn_child` at
`crates/kernel/src/handle.rs:559`), so history is per-session-key — the
same key the topology shell is already routing on. No backend change is
needed.

The current `web/src/components/topology/AGENT.md` already documents the
gap explicitly: *"history-on-reload is deferred (no `GET /messages`
endpoint yet)"*. The endpoint exists; the frontend just never plugged
into it.

Reproducer:
1. Open `/topology` on a deployment with at least one chat session that
   has prior messages (e.g. the auto-selected most-recent session that
   `SessionPicker` lands on).
2. Center pane shows: *"Waiting for the next turn on `<key>`…"*. No
   bubbles, no `TurnCard`s.
3. `curl http://10.0.0.183:25555/api/v1/chat/sessions/<key>/messages`
   returns N>0 messages. The data is there.
4. After fix: same navigation renders historical user bubbles +
   assistant `TurnCard`s in `seq` order before any WS event arrives.

Prior art reviewed:
- PR 2003 / #1999 ("multi-agent observability UI") landed the topology
  shell. Its task #6 commit `6fadb9bf` ("feat(web): topology timeline
  view") shipped `TimelineView` without history. Subsequent commits in
  the same umbrella (`62de3b62` craft chat input, `8ec0dadb` 3-pane
  shell, `bdc45b89` tape lineage) compounded the surface but none
  wired history. The umbrella merged on 2026-04-30; this is its known
  follow-up.
- `web/src/components/topology/AGENT.md:38–39` calls out the deferred
  history fetch by name.
- No prior issue or PR re-opens this scope — confirmed via
  `gh issue list --search "topology history messages"`,
  `gh pr list --search "topology timeline"`, and
  `git log --grep topology --since=90.days`.
- The legacy chat surface (`pages/Chat.tsx`, `hooks/use-session-timeline.ts`)
  uses a different live-rendering strategy (per-session WS only) and
  does not call `/messages` either, so there is no existing fetch hook
  to reuse — a new hook is the smallest change.
- `crates/extensions/backend-admin/src/chat/service.rs:944–1060`
  (`tap_entries_to_chat_messages`) is the source of truth for
  ChatMessage shape; the frontend mapping mirrors it.

Goal alignment: advances signal 4 ("every action is inspectable") — a
topology view that hides persisted turns is a lying inspector. Indirectly
advances signal 2 ("the user stops asking") because losing visible
history forces the user to re-ask. Crosses no NOT line; this is
restoring observable behavior, not a feature-parity push.

Hermes positioning: chat history rendering is table-stakes for any agent
UI (Hermes does this trivially); rara has no engineering reason to do it
differently. The reason it regressed is purely accidental — vendor swap
landed before the fetch path was wired. Restoring the obvious behavior.

Single test that nails the lane: a vitest test mounts `<TimelineView>`
with a `viewSessionKey` whose `/messages` response is mocked to return
two messages (one user, one assistant). Before the fix, the test sees
the "Waiting for the next turn…" placeholder. After the fix, it sees
the user-bubble text and the assistant text rendered through
`TurnCard`. Fail before, pass after.

Two visual regressions in the in-flight implementation (caught by
parent-agent browser smoke test against PR 2018) extend this Intent:

1. **Chronological ordering regression.** The current code in
   `TimelineView.tsx:297–305` renders ALL user bubbles before ALL
   agent turns. That was acceptable when "user bubbles" meant 1–2
   optimistic live-session entries, but with history loaded it pulls
   every historical user message to the top of the timeline, losing
   conversation flow. Concrete repro: session
   `d6e905d9-fd62-41ca-8918-97b37276f534` has 8 user / 17 assistant /
   6 tool_result messages; in the rendered DOM all 8 user bubbles
   stack at `top: -1372px` (above all assistant content), so a user
   scrolled to the top sees only assistant content and asks "where
   did the questions go". Fix: interleave user bubbles and agent
   turns chronologically using the existing `created_at` field on
   `ChatMessage` (already on the wire — see
   `crates/extensions/backend-admin/src/chat/service.rs:996`,
   `created_at: entry.timestamp` — and already serialized as i64
   milliseconds via `#[serde(with = "ts_milliseconds")]` on the
   shared `ChatMessage` type). Live agent turns produced from
   topology events have no `created_at` but are by construction
   strictly post-barrier, so they sort to the tail.

2. **Assistant `TurnCard` height regression.** Assistant turns
   render with substantially over-tall empty card frames in the
   browser, inconsistent with the desired inline-markdown shape
   (see `vendor/craft.png`). Either the historical render path
   produces empty inner segments (e.g. an empty assistant text row
   that still occupies a full segment height) or the `TurnCard`
   container has unconditional padding / `min-height`. Implementer
   investigates; the falsifying contract is the height-bound
   scenario below.

## Decisions

- **Where the fetch lives.** New hook `web/src/hooks/use-session-history.ts`
  owns the `react-query` fetch keyed by `['topology', 'session-history',
  sessionKey]`. Pattern matches `use-chat-models` / `use-skills`
  (react-query, `staleTime` aligned with the data's mutation cadence —
  history mutates on every turn, so `staleTime: 0` and rely on the WS
  push for freshness). The hook is invoked from `TimelineView` keyed on
  `viewSessionKey`, so switching workers automatically refetches.
- **REST wrapper.** Add `listMessages(key, limit?)` to
  `web/src/api/sessions.ts` returning a typed `ChatMessage[]`. The
  TypeScript shape mirrors `crates/kernel/src/channel/types.rs`'s
  `ChatMessage` (fields `seq`, `role`, `content`, `tool_calls`,
  `tool_call_id`, `tool_name`, `created_at`). Default `limit` 200 to
  match the backend default at `router.rs:550`.
- **Mapping to existing render shapes.** Historical user messages
  (`role === "user"`) feed `userTurnsBySession[viewSessionKey]` as
  `{id, text, t, createdAt}` entries — the same shape `handleSubmit`
  already produces, plus a `createdAt` field threaded from
  `ChatMessage.created_at` so that interleaved chronological ordering
  (see next bullet) has a real key to sort on. Historical assistant
  messages (`role === "assistant"`) and tool messages (`role === "tool"`
  / `"tool_result"`) build a new `historyTurns: TurnCardData[]` array
  via a pure helper `buildTurnsFromHistory(messages)` co-located with
  the other reducer in `TurnCard.tsx`. The helper folds consecutive
  assistant + tool-result messages into one `TurnCardData` (one user
  prompt = one assistant turn, ending at the next user message or
  end-of-list), populates `text` from assistant content, `toolCalls`
  from `assistant.tool_calls` paired with the corresponding
  `tool_result` content, sets `inFlight: false`, `metrics: null`,
  `usage: null` (history does not carry the live metrics frame), and
  carries `createdAt` from the assistant message that anchors the turn
  so the chronological merge has a sort key for both bubbles and
  turns.
- **Chronological ordering of bubbles and turns.** Historical user
  bubbles and historical+live agent turns render in a **single ordered
  list** keyed on `createdAt` for history-sourced items. The earlier
  "all user bubbles first, then all agent turns" design (current
  `TimelineView.tsx:297–305`) is removed because it broke once history
  loaded more than 1–2 user messages: with 8 historical user prompts
  the entire prompt sequence floats above the agent replies, losing
  conversation flow (concrete repro on session
  `d6e905d9-fd62-41ca-8918-97b37276f534`: 8 user bubbles render at
  `top: -1372px`, above all assistant content). Live agent turns
  derived from topology events have no `createdAt`; they always sort
  to the tail because they are by construction strictly post-barrier
  (the arrival barrier ensures any live turn was produced after the
  history fetch resolved). Implementation: build a unified array of
  `{kind: "bubble" | "turn", createdAt: number | null, payload}` and
  sort stably by `createdAt` ascending with `null` last; render in
  order. Each rendered node carries `data-testid="turn-or-bubble"` for
  the chronological-ordering scenarios.
- **Assistant turn height matches content.** Visual regression
  observed in the parent agent's browser smoke test: assistant
  `TurnCard`s render with substantially over-tall empty boxes,
  inconsistent with the desired inline-markdown rendering (compare
  `vendor/craft.png`). Root cause is to be diagnosed by the
  implementer; the contract is that an assistant turn whose only
  content is a short plain-text body must not render a card frame
  with empty internal segments or unconditional `min-height` padding.
  Either (a) suppress empty inner segments in `TurnCard.tsx` so a
  text-only history turn renders only the markdown row, or
  (b) remove the unconditional `min-height` / over-large vertical
  padding from the card container, whichever is the smaller change.
  Falsified by the new height-bound scenario.
- **Live + history dedupe — arrival-barrier, not seq.** `ChatMessage.seq`
  (per-tape, persistent) and `TopologyEventEntry.seq`
  (`web/src/hooks/use-topology-subscription.ts:179,196`, per-WS-connection,
  resets to 0 on every reconnect) are NOT comparable counters. A
  `seq <= lastHistorySeq` filter is wrong-by-construction: after a
  reconnect with 50 persisted entries, the first live frame arrives with
  `seq=1` and would be incorrectly dropped, freezing chat until the WS
  frame counter eventually exceeds 50. Inspecting the wire types
  (`TopologyEventEntry` in the hook + `WebFrame` variants like
  `text_delta`) shows there is also no shared id and no per-event
  timestamp on the WS side — there is no field at parity that can serve
  as a content-level dedupe key.
  Use an **arrival-time barrier** instead: at the moment the history
  fetch for `viewSessionKey` resolves, snapshot
  `historyBarrierSeq = events.length` (the current length of the
  topology subscription's events buffer for this session). When folding
  live events for `viewSessionKey`, only consider entries whose buffer
  index is `>= historyBarrierSeq`; entries that arrived before the
  history fetch resolved are treated as already represented by the
  history payload and dropped. Re-snapshot the barrier on every
  successful refetch (including session switch and WS reconnect), keyed
  by `viewSessionKey`. This avoids any cross-counter comparison entirely
  and uses only a quantity each side actually has: history's "this is
  the truth as of the moment I resolved" and the live buffer's local
  index.
- **`agent-spec lifecycle` does not currently support `Package: web`
  selectors** (no vitest adapter). The lifecycle gate on this spec
  therefore fails by tooling, not by verification — implementer and
  reviewer verify the scenarios by running vitest directly. Tracked as
  a lane-2 chore in issue 2015 ("agent-spec: add vitest adapter for
  web specs").
- **Session switch reset.** Already handled correctly: `userTurnsBySession`
  keys by session, and `agentTurns` filters `events` by session. The
  new history hook keys on `viewSessionKey` so react-query swaps the
  cached entry on switch. No teardown needed beyond letting the hook
  re-key.
- **Loading and error UI.** While the fetch is pending, render the
  existing placeholder ("Waiting for the next turn on …") — no
  spinner, no skeleton. On fetch error, render an inline retry-able
  error band above the input editor; do NOT block the WebSocket from
  attaching, so a transient HTTP failure does not break live chat.
  This matches the project's "no noop UX" posture: the user is told
  history failed to load, but the rest of the surface keeps working.
- **No backend changes.** The endpoint, the service, and the tape
  conversion already exist and are correct. Spec scope is purely
  `web/`.
- **No fork-tree sidebar, no SessionEntry parent_id.** Both deferred
  per user direction — out of scope.

## Boundaries

### Allowed Changes
- **/web/src/components/topology/TimelineView.tsx
- **/web/src/components/topology/TurnCard.tsx
- **/web/src/components/topology/RaraTurnCard.tsx
- **/web/src/components/topology/AGENT.md
- **/web/src/test-setup.ts
- **/web/src/hooks/use-session-history.ts
- **/web/src/api/sessions.ts
- **/web/src/api/types.ts
- **/web/vitest.config.ts
- **/web/src/components/topology/__tests__/**
- **/web/src/hooks/__tests__/**
- **/specs/issue-2013-topology-timeline-history.spec.md

### Forbidden
- crates/**
- web/src/vendor/**
- web/src/pages/Chat.tsx
- web/src/hooks/use-session-timeline.ts
- web/src/hooks/use-topology-subscription.ts
- web/src/hooks/use-chat-session-ws.ts
- web/src/agent/**
- web/src/components/topology/SessionPicker.tsx
- web/src/components/topology/WorkerInbox.tsx
- web/src/components/topology/TapeLineageView.tsx
- web/src/components/topology/SpawnMarker.tsx
- web/src/components/topology/WorkerCard.tsx
- web/src/components/topology/tape-tree-layout.ts
- .github/workflows/**
- config.example.yaml

## Acceptance Criteria

Scenario: TimelineView renders historical messages on mount before any live event
  Test:
    Package: web
    Filter: TimelineView.history.renders_history_before_live_events
  Given a session key with two persisted messages (one user "hello", one assistant "hi there") returned by GET /api/v1/chat/sessions/{key}/messages
  When TimelineView mounts with that viewSessionKey and an empty topology events buffer
  Then the rendered DOM contains the user bubble text "hello"
    And contains the assistant text "hi there" rendered via TurnCard
    And does not show the "Waiting for the next turn" placeholder

Scenario: Switching viewSessionKey refetches history and resets the rendered timeline
  Test:
    Package: web
    Filter: TimelineView.history.session_switch_refetches
  Given TimelineView is mounted with viewSessionKey "A" whose history contains message "from-A"
    And the rendered DOM contains "from-A"
  When the parent re-renders with viewSessionKey "B" whose history contains message "from-B"
  Then the rendered DOM contains "from-B"
    And the rendered DOM does not contain "from-A"

Scenario: Live events that arrived before history resolved are not re-rendered after history loads
  Test:
    Package: web
    Filter: TimelineView.history.arrival_barrier_dedupe
  Given TimelineView is mounted with viewSessionKey "S" and a pending GET /api/v1/chat/sessions/S/messages
    And the topology subscription delivers a text_delta event for session "S" with delta "boundary-text" while the history fetch is still pending
    And the rendered DOM (pre-history) contains "boundary-text" exactly once via the live path
  When the history fetch resolves with one assistant message whose content is "boundary-text"
  Then the rendered DOM contains "boundary-text" exactly once
    And the assistant content is sourced from the history payload (rendered through the history TurnCard path), not duplicated by the pre-history live event

Scenario: WS reconnect re-snapshots the barrier even when history payload is structurally unchanged
  Test:
    Package: web
    Filter: TimelineView.history.reconnect_resnapshots_barrier
  Given TimelineView is mounted with viewSessionKey "S" and a history fetch that resolves with one assistant message "X"
    And the rendered DOM contains "X" exactly once
    And a live text_delta event for "Y" has rendered post-barrier
  When the topology subscription rebuilds its events buffer from [] (mimicking a WS reconnect)
    And the history refetch resolves with a payload structurally identical to the previous one (same array reference under react-query structural sharing)
    And a fresh live text_delta event for "X" arrives (kernel re-streaming the in-progress turn after reconnect)
  Then the rendered DOM contains "X" exactly once
    And the post-reconnect live frame is gated by a freshly-snapshotted arrival barrier rather than rendering on top of history

Scenario: Historical user bubbles and assistant turns render in chronological order
  Test:
    Package: web
    Filter: TimelineView.history.chronological_ordering_history_only
  Given a session whose history payload returns four messages in tape order: M1 role=user content="q1" created_at=t1, M2 role=assistant content="a1" created_at=t2, M3 role=user content="q2" created_at=t3, M4 role=assistant content="a2" created_at=t4 with t1<t2<t3<t4
  When TimelineView mounts with that viewSessionKey and an empty topology events buffer
  Then the rendered DOM contains exactly four nodes with attribute data-testid="turn-or-bubble"
    And the document order of those nodes is: bubble("q1"), turn("a1"), bubble("q2"), turn("a2")
    And no node containing "q2" appears in the DOM before any node containing "a1"

Scenario: Live agent turn renders strictly after historical entries
  Test:
    Package: web
    Filter: TimelineView.history.chronological_ordering_history_then_live
  Given a session whose history payload returns two messages: M1 role=user content="hist-q" created_at=t1, M2 role=assistant content="hist-a" created_at=t2 with t1<t2
    And TimelineView is mounted with that viewSessionKey and the rendered DOM contains bubble("hist-q") then turn("hist-a") in that order
  When the topology subscription delivers a post-barrier text_delta event for the same session that produces an agent turn with text "live-a"
  Then the document order of the data-testid="turn-or-bubble" nodes is: bubble("hist-q"), turn("hist-a"), turn("live-a")
    And no node containing "live-a" appears in the DOM before any node containing "hist-a"

Scenario: Assistant turn with short text content renders without an over-tall empty card
  Test:
    Package: web
    Filter: TimelineView.history.assistant_turn_height_matches_content
  Given a session whose history payload returns one assistant message of role=assistant content="ok" created_at=t1 with no tool_calls
  When TimelineView mounts with that viewSessionKey
  Then the rendered TurnCard for that message contains the text "ok"
    And the rendered TurnCard's bounding height is at most 96 pixels (single short markdown line plus padding)
    And the rendered TurnCard contains no descendant element with computed style min-height greater than 0 other than the content row itself

Scenario: Assistant message body is rendered as markdown by the vendor TurnCard surface
  Test:
    Package: web
    Filter: TimelineView.history.markdown_renders_inline_formatting
  Given a session whose history payload returns one assistant message with content "Here is **bold** and `code`"
  When TimelineView mounts with that viewSessionKey
  Then the rendered DOM contains a <strong> element whose text is "bold"
    And the rendered DOM contains a <code> element whose text is "code"
    And no literal "**" or backtick characters appear adjacent to those tokens (proving the markdown was parsed, not displayed verbatim)

Scenario: History fetch failure still allows live chat to function
  Test:
    Package: web
    Filter: TimelineView.history.fetch_error_does_not_block_live
  Given GET /api/v1/chat/sessions/{key}/messages responds with HTTP 500
  When TimelineView mounts with that viewSessionKey
  Then an inline error band is visible
    And the InputContainer is rendered and not disabled solely because of the history fetch error

## Out of Scope

- Adding `parent_id` (or any other field) to `SessionEntry`.
- Building the cross-session fork-tree sidebar (deferred per user).
- Unifying `ChatMessage.seq` (per-tape counter) with
  `TopologyEventEntry.seq` (per-WS-connection counter). The
  arrival-barrier dedupe above sidesteps the gap entirely; any
  cross-counter unification work is a separate concern and not needed
  for this spec to pass.
- Building the vitest adapter for `agent-spec lifecycle`. Tracked
  separately in issue 2015.
- Pagination of history beyond `limit=200`. The backend already
  supports `?limit=N`; UI-side scroll-to-load-more is a follow-up.
- Touching the legacy `pages/Chat.tsx` surface — it has its own
  rendering strategy and is not what regressed.
- Sub-second tie-breaking between bubbles and turns that share the
  same `created_at` timestamp. Backend timestamps are millisecond
  resolution; if two messages collide on `created_at` the renderer
  falls back to tape insertion order from the history payload, which
  is already monotonic by `seq`. No additional contract is required.
- Backend changes of any kind. Endpoint and service are already
  correct.
