spec: task
name: "issue-2063-user-message-appended-event"
inherits: project
tags: []
---

## Intent

In the topology TimelineView the same user message renders **twice** for an
in-flight turn — once at the top of the timeline and once at the tail,
bracketing the in-flight assistant card and the per-turn `tape_forked`
marker (see `vendor/ing.png`). The duplicate is structural, not visual:
`web/src/components/topology/TimelineView.tsx` keeps two parallel,
unreconciled sources of truth for user messages.

1. **Optimistic path.** `userTurnsBySession` state is pushed in
   `handleSubmit`, never cleared, entries have `createdAt: null` (and
   therefore sort to the tail by the chronological merge introduced in
   PR 2018 / spec issue 2013).
2. **Persisted path.** `useSessionHistory`
   (`web/src/hooks/use-session-history.ts`) runs with `staleTime: 0`,
   refetches on WS reconnect (`TimelineView.tsx:205`) and on window
   focus, and yields entries with the backend `created_at` timestamp.

Backend (`crates/kernel/src/kernel.rs` Phase 4–5, around lines 2335–2400)
appends the user message to the main tape **synchronously, pre-fork,
pre-agent-loop**, so the `GET /api/v1/chat/sessions/{key}/messages`
endpoint returns the message within milliseconds of submit. Once history
refetches mid-turn, both paths render the same content: the persisted
entry sorts to the front by `createdAt`, the optimistic entry sorts to
the tail (after the in-flight assistant card), bracketing the assistant
card with two identical bubbles.

The two paths share no common id — optimistic uses a front-end-generated
`u-${Date.now()}-...`, persisted uses backend `seq`. There is no
reconciliation point. The `userTurnsBySession` mechanism is itself a
workaround for a missing kernel echo: today the topology stream carries
only assistant-side frames (`text_delta`, `tool_call_*`, `subagent_*`,
`tape_forked`, `turn_metrics`, etc.); the user prompt's persistence is
visible only via REST. The optimistic path is what filled that gap, and
it races with `useSessionHistory`.

Reproducer:
1. Open `/topology/<existing-key>` for a session with prior messages.
2. Submit a new prompt P. The optimistic bubble appears immediately.
3. While the assistant is still streaming (text deltas flowing,
   `tape_forked` already emitted), trigger a history refetch — easiest
   path is to switch tabs and back (window focus) or wait for the WS
   reconnect refetch on `TimelineView.tsx:205`.
4. Observed: bubble(P) at the top of the timeline (persisted, real
   `created_at`), then the in-flight assistant `TurnCard`, then the
   per-turn `tape_forked` marker, then bubble(P) again at the tail
   (optimistic, `createdAt: null`).
5. After fix: bubble(P) appears exactly once at its correct
   chronological slot, before, during, and after the in-flight turn.
   Mid-turn focus/reconnect refetches do not introduce duplicates.

The agreed solution is to **make the kernel echo user-prompt persistence
on the topology stream** so the FE has a real frame to react to, and
**remove the optimistic path entirely**. Concretely:

- **Backend.** In Phase 4–5 of `crates/kernel/src/kernel.rs`, right
  after `tape_service.append_message` returns Ok for the user message
  (the non-Mita-directive branch around lines 2375–2400) and before the
  agent-loop spawn, emit a new `StreamEvent::UserMessageAppended`
  variant on the session topology bus carrying the persisted tape entry
  identity (`seq` matching the `ChatMessage.seq` produced by
  `tap_entries_to_chat_messages` in
  `crates/extensions/backend-admin/src/chat/service.rs`), the
  multimodal-passthrough `content` matching the persisted shape, and
  `created_at` matching `entry.timestamp`. Forward through
  `crates/channels/src/web.rs` (`pub enum WebEvent` at line 99) to a
  corresponding `WebEvent::UserMessageAppended` so the FE WS subscriber
  receives it; update `crates/channels/src/web_topology.rs` if the
  forwarding pipe needs an explicit hand-off (the existing
  `SubagentSpawned` / `TapeForked` handling at line 288 is the model).
- **Frontend.** Remove `userTurnsBySession` and the optimistic push from
  `TimelineView.tsx` entirely. User bubbles come exclusively from
  (a) `historyUserBubbles` derived from `useSessionHistory` and (b) the
  new topology event applied to the live `events` buffer. Both keyed on
  backend `seq` so React/dedupe handles them naturally; the existing
  arrival barrier in TimelineView already separates "live tail" vs
  "history block" by event index, so the new event slots into that path
  without new dedupe machinery. `handleSubmit` no longer pushes a local
  bubble — the optimistic latency is replaced by the topology
  round-trip, which on a local kernel append is sub-frame.

Prior art reviewed:
- PR 2013 / spec `specs/issue-2013-topology-timeline-history.spec.md`
  (merged 2026 in PR 2018) introduced `userTurnsBySession`, the
  arrival-barrier dedupe, and the `useSessionHistory` hook with
  `staleTime: 0`. That spec explicitly chose the arrival-barrier
  mechanism because `ChatMessage.seq` and `TopologyEventEntry.seq`
  were not comparable — the user-prompt path had no kernel-side echo
  to ride on. This issue is the structural follow-up: by adding the
  echo, we collapse the two-source-of-truth design into one and
  retire the optimistic workaround. The arrival barrier itself stays
  (live agent turns still need it); only the optimistic user-bubble
  branch goes away.
- Commit `f5f17976 fix(memory): deduplicate user message in LLM
  context assembly (#101)` is a separate, backend-internal dedup
  inside `default_tape_context` and not related to the FE rendering
  duplicate addressed here. Confirmed by reading the message and the
  surrounding tape-context code path.
- Commit `8c16d920 feat(web): add optimistic updates for chat message
  sending` is the original introduction of the optimistic mechanism
  on the legacy `pages/Chat.tsx` surface — predates the topology
  surface and is not what we are touching. The legacy chat path is
  out of scope (forbidden) here.
- `gh issue list --search "user message duplicate timeline"` and
  `gh pr list --search "user message duplicate"` returned no other
  prior occurrences against the topology surface.

Goal alignment: advances `goal.md` signal 4 ("every action is
inspectable") — a timeline that double-renders the same user prompt
around the in-flight assistant is a lying inspector. Indirectly
advances signal 2 ("the user stops asking") because the duplicate
forces the user to second-guess what was actually sent. Crosses no NOT
line; this is fixing the inspector, not adding a feature.

Hermes positioning: round-tripping persisted user-message events back
to the UI is table-stakes for any agent UI; rara has no engineering
reason to do it differently. The reason rara reached today's state is
purely the order in which #2003 / #2013 landed (topology shell first,
history second, with optimistic-only as the bridge). The kernel-side
echo collapses the bridge into the same channel everything else
already flows through.

Lane test that nails lane 1: a vitest test mounts `<TimelineView>` with
a `viewSessionKey`, drives a user submit, has the topology subscription
deliver a `UserMessageAppended` frame, and **also** has
`useSessionHistory` refetch mid-turn returning the same persisted
message. Before the fix, the test sees two DOM nodes whose text equals
the prompt. After the fix, exactly one. Fail before, pass after.

## Decisions

- **New variant name.** `StreamEvent::UserMessageAppended` (kernel
  side) and `WebEvent::UserMessageAppended` (web channel side). Both
  carry `seq: i64`, `content: serde_json::Value` (mirroring the
  `tape_content` value already produced at
  `crates/kernel/src/kernel.rs` Phase 5 — text-only as a JSON string,
  multimodal as the structured passthrough), and `created_at:
  DateTime<Utc>`. Serialized field name on the wire is
  `user_message_appended` to match the existing snake_case convention
  on `WebEvent` (`text_delta`, `tape_forked`, `subagent_spawned`).
- **Where the kernel emits.** Inside `crates/kernel/src/kernel.rs` in
  the non-Mita-directive branch of Phase 5, immediately after
  `tape_service.append_message(...).await` returns Ok and **before**
  the agent-loop spawn. Emit-on-success only — if the tape append
  fails (the existing `warn!` branch), no event is emitted. The Mita
  directive branch is unaffected (Mita directives are not user-visible
  prompts).
- **Where the seq comes from.** The append API must surface the
  persisted entry's `seq` so the event carries the same value the
  REST endpoint will return. If `tape_service.append_message` does not
  already return the new entry's seq, extend its return type to do
  so; the alternative (re-reading the tape tail) is racy. The
  reviewer must verify the seq the event carries matches the seq
  `tap_entries_to_chat_messages` produces for the same entry.
- **Forwarding to WebEvent.** `crates/channels/src/web.rs`'s
  `From<StreamEvent>` (or the equivalent fan-out switch) maps
  `StreamEvent::UserMessageAppended` → `WebEvent::UserMessageAppended`
  with the same fields. `crates/channels/src/web_topology.rs` only
  needs an explicit branch if the topology forwarding pipe filters
  events explicitly (mirror the `TapeForked` / `SubagentSpawned`
  treatment at line 288).
- **Tape-context exclusion.** The new event is a stream-side echo of a
  tape entry that **already exists**; it must NOT cause the user
  message to be appended to the tape a second time, and it must NOT
  affect `default_tape_context` (the LLM context assembler must not
  see this event because it consumes the tape, not the stream). Test:
  the kernel runs as today against the tape; the new emit is purely
  observational on the stream side.
- **FE: remove optimistic, source bubbles from history + topology
  event.** Delete the `userTurnsBySession` state, the corresponding
  setter call in `handleSubmit`, and the merge that includes optimistic
  entries in the chronological list at `TimelineView.tsx`. Bubbles for
  the live tail come from a new derivation that scans the topology
  events buffer for `UserMessageAppended` frames whose buffer index is
  `>= historyBarrierSeq`, mapping each into the same `{kind: "bubble",
  createdAt, payload}` shape the chronological merge already accepts.
  Bubbles before the barrier come from `historyUserBubbles` exactly as
  today.
- **Dedupe key for the merge.** Both history-derived and topology-event-
  derived bubbles carry the backend `seq` as the React key. If a
  frame and a history entry collide on `seq` (possible when a
  history refetch resolves while the live frame is still in the
  buffer), the merge keeps the history entry and drops the live one
  (history is canonical). This is a refinement of the existing
  arrival-barrier policy, not a replacement.
- **No legacy chat changes.** `web/src/pages/Chat.tsx` and
  `web/src/hooks/use-chat-session-ws.ts` are out of scope — they have
  their own optimistic mechanism on a separate surface and are not
  what regressed.
- **No protobuf / gRPC changes.** The topology stream is a JSON
  WebSocket (`/api/v1/chat/sessions/{key}/topology`); the new variant
  is a `serde::Serialize`-derived enum case. No proto regeneration
  needed.
- **Test harness.** The vitest test reuses the existing `MSW`-style
  mocks under `web/src/**/__tests__` (see
  `TimelineView.history.test.tsx` for the pattern). No new test
  framework. Backend-side: a Rust unit / integration test in
  `crates/kernel/tests/` that drives a single user prompt and
  asserts the topology stream contains exactly one
  `UserMessageAppended` event with the expected `seq` / `content` /
  `created_at`, plus zero duplicates of the same `seq` across the
  turn.

## Boundaries

### Allowed Changes
- **/crates/kernel/src/kernel.rs
- **/crates/kernel/src/io.rs
- **/crates/kernel/src/memory/**
- **/crates/kernel/src/trace/builder.rs
- **/crates/kernel/tests/**
- **/crates/channels/src/web.rs
- **/crates/channels/src/web_topology.rs
- **/crates/channels/tests/**
- **/crates/cmd/src/chat/mod.rs
- **/crates/extensions/backend-admin/src/chat/service.rs
- **/web/src/components/topology/TimelineView.tsx
- **/web/src/components/topology/AGENT.md
- **/web/src/components/topology/__tests__/**
- **/web/src/hooks/use-topology-subscription.ts
- **/web/src/hooks/__tests__/**
- **/web/src/api/types.ts
- **/specs/issue-2063-user-message-appended-event.spec.md

### Forbidden
- web/src/pages/Chat.tsx
- web/src/hooks/use-chat-session-ws.ts
- web/src/hooks/use-session-timeline.ts
- web/src/vendor/**
- web/src/agent/**
- web/src/components/topology/SessionPicker.tsx
- web/src/components/topology/WorkerInbox.tsx
- web/src/components/topology/SpawnMarker.tsx
- web/src/components/topology/WorkerCard.tsx
- web/src/components/topology/tape-tree-layout.ts
- crates/kernel/src/agent/**
- crates/kernel/src/guard/**
- config.example.yaml
- .github/workflows/**

## Acceptance Criteria

Scenario: Kernel emits UserMessageAppended on the topology stream after persisting a user message
  Test:
    Package: rara-kernel
    Filter: kernel_emits_user_message_appended_after_tape_append
  Given a session with an attached topology stream subscriber
  When a non-Mita-directive user message is delivered through the kernel ingress and Phase 5 of message handling completes successfully
  Then the topology stream subscriber receives exactly one StreamEvent::UserMessageAppended for that message
    And the event's seq equals the seq returned by the REST endpoint GET /api/v1/chat/sessions/{key}/messages for the same tape entry
    And the event's content matches the persisted tape entry content (text-as-string for plain text, multimodal-passthrough JSON for multimodal)
    And the event's created_at equals the persisted tape entry timestamp

Scenario: Kernel does not emit UserMessageAppended for Mita directives
  Test:
    Package: rara-kernel
    Filter: kernel_does_not_emit_user_message_appended_for_mita_directive
  Given a session with an attached topology stream subscriber
  When an inbound message marked with metadata mita_directive=true is delivered through the kernel ingress
  Then the topology stream subscriber receives zero StreamEvent::UserMessageAppended events for that message

Scenario: Kernel does not emit UserMessageAppended when tape append fails
  Test:
    Package: rara-kernel
    Filter: kernel_does_not_emit_user_message_appended_on_tape_failure
  Given a session whose tape_service.append_message is configured to return an error
  When a non-Mita-directive user message is delivered through the kernel ingress
  Then the topology stream subscriber receives zero StreamEvent::UserMessageAppended events for that message

Scenario: WebEvent variant carries the same fields as the kernel StreamEvent
  Test:
    Package: rara-channels
    Filter: web_event_user_message_appended_round_trip
  Given a StreamEvent::UserMessageAppended with seq S, content C, and created_at T
  When the channel layer converts it for the topology WebSocket
  Then the resulting WebEvent::UserMessageAppended carries seq S, content C, and created_at T
    And the JSON-serialized field name is user_message_appended

Scenario: TimelineView renders exactly one user bubble for an in-flight turn even when history refetches mid-turn
  Test:
    Package: web
    Filter: TimelineView.user_message_appended.no_duplicate_when_history_refetches_mid_turn
  Given TimelineView is mounted with a viewSessionKey whose history initially returns []
    And the user submits prompt "P" via the InputContainer
    And the topology subscription delivers a UserMessageAppended event for "P" with seq=1
    And the topology subscription begins delivering text_delta events for the assistant turn
  When useSessionHistory refetches mid-turn and resolves with one persisted user message of seq=1 content="P"
  Then the rendered DOM contains exactly one node whose text content equals "P"
    And that node has document order before any node containing assistant text deltas

Scenario: TimelineView removes the optimistic user-bubble path entirely
  Test:
    Package: web
    Filter: TimelineView.user_message_appended.no_optimistic_state
  Given TimelineView source has been updated to consume UserMessageAppended events
  When the rendered tree is inspected after a user submit
  Then no React state named userTurnsBySession is present in the TimelineView component instance
    And no user-bubble node is rendered with createdAt=null

Scenario: TimelineView renders a user bubble from the topology event when history is empty and no REST refetch fires
  Test:
    Package: web
    Filter: TimelineView.user_message_appended.bubble_from_topology_event_alone
  Given TimelineView is mounted with a viewSessionKey whose history returns []
    And no further history refetch is triggered during the test
  When the topology subscription delivers a UserMessageAppended event for prompt "Q" with seq=1 created_at=t1
  Then the rendered DOM contains exactly one node whose text content equals "Q"
    And that node carries the React key derived from seq=1
    And that node renders before any subsequent live agent turn

Scenario: WS reconnect mid-turn does not duplicate the user bubble
  Test:
    Package: web
    Filter: TimelineView.user_message_appended.reconnect_does_not_duplicate
  Given TimelineView is mounted with a viewSessionKey and a UserMessageAppended event for "R" seq=1 has rendered exactly one bubble
    And an assistant turn is in flight (text_delta events still arriving)
  When the topology subscription rebuilds its events buffer from [] (mimicking a WS reconnect)
    And useSessionHistory refetches and resolves with one persisted user message seq=1 content="R"
    And a fresh UserMessageAppended event for "R" seq=1 arrives on the rebuilt buffer
  Then the rendered DOM contains exactly one node whose text content equals "R"
    And the bubble is sourced from the history payload (history takes precedence on seq collision), not from the post-reconnect live frame

## Out of Scope

- Touching the legacy `pages/Chat.tsx` surface or its optimistic
  message path. That surface has its own rendering strategy and is
  not where the duplicate appears.
- Re-litigating the arrival-barrier dedupe for live agent turns.
  The barrier stays; only the optimistic user-bubble branch is
  retired.
- Cross-counter unification of `ChatMessage.seq` and
  `TopologyEventEntry.seq`. The new event carries the tape `seq`
  directly; no global renumbering is required.
- Multimodal-content schema changes. The new event passes through
  whatever shape the tape entry already carries.
- Adding a new endpoint for "user message appended" notifications.
  The existing topology WS is the carrier.
- FE retry/backoff semantics for missed `UserMessageAppended` frames.
  The history refetch path remains the safety net; if a frame is
  lost (network blip), the next history refetch reconciles.
- Backend pagination, history limit changes, or any other
  REST-side change.
