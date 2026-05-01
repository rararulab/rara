# web/src/components/topology — Agent Guidelines

## Purpose

Frontend for the multi-agent observability surface (umbrella issue
#1999). Renders the cross-session topology WebSocket stream
(`/api/v1/kernel/chat/topology/{root}`) as a main timeline of agent
turns plus a right-rail worker inbox of spawned subagents.

## Architecture

The page is a craft-style 3-pane shell hosted by `pages/Topology.tsx`:
`SessionPicker` (left, 280px) | `TimelineView` (centre, flex) |
`WorkerInbox` + `TapeLineageView` (right, 320px). The shell auto-selects
the most-recent session on first load so users never have to paste a
session UUID — task #9 of #1999 fixed that UX complaint by replacing
the old free-text "root session key" input with a clickable list.

The topology header carries a `PanelLeft` / `PanelLeftClose` icon button
(issue #2022) that toggles the left rail by conditionally rendering the
`<aside>` wrapper around `SessionPicker`. When collapsed the picker is
removed from the DOM, the centre column reclaims the freed flex width,
and the preference is mirrored to `localStorage` under
`rara.topology.sidebarCollapsed` so it survives reloads. The toggle lives
in `Topology.tsx` rather than threading through `AppShellContext` because
the topology page does not consume the AppShell — adopting it for one
button would be overreach.

- `SessionPicker.tsx` — left rail. Lists the most-recently updated
  sessions via `useQuery(['topology', 'chat-sessions'])` against
  `GET /api/v1/chat/sessions?limit=50`, polls every 30s, and exposes
  a `+ New` button that POSTs `/api/v1/chat/sessions` with a default
  title. Calls `onAutoSelect(firstKey)` once when the URL has no key
  so the shell can redirect `/topology` → `/topology/{key}`.
- `TimelineView.tsx` — vertical list of `TurnCard`s; filters the
  topology event buffer down to a single `viewSessionKey` (root by
  default; the worker inbox swaps in a child key when one is selected).
  Hosts the vendored craft `InputContainer` pinned at the bottom and
  renders user prompts via the vendored `UserMessageBubble`; the centre
  column is `flex flex-col`, turns scroll inside the upper region, the
  editor sticks to the floor. Owns `useChatSessionWs` (one per-session
  WebSocket) and pushes plain-text prompts. The editor always sends into
  the **root** session — browsing a worker child via the inbox stays
  observation-only so replies do not get written to a sandbox tape the
  user did not pick. User-message rendering is **optimistic**: the typed
  text is added to local state on submit so it appears before the
  backend round-trip; the kernel does not echo user prompts back as
  topology events today. History-on-reload fetches
  `GET /api/v1/chat/sessions/{key}/messages` via `useSessionHistory` and
  reduces persisted assistant turns through `buildTurnsFromHistory`,
  while persisted user messages render as `UserMessageBubble`s ahead of
  any optimistic prompts. Live/history dedupe uses an **arrival-time
  barrier**, not seq comparison: at the moment the history query resolves
  for `viewSessionKey`, the current length of the session-filtered topology
  buffer is snapshotted; only entries whose buffer index is `>= barrier`
  feed the live reducer. The barrier resets on session switch (keyed map)
  and on WS reconnect (detected by the session-filtered buffer length
  going backwards, which triggers `history.refetch()` and a re-snapshot).
  This avoids comparing `ChatMessage.seq` (per-tape) with
  `TopologyEventEntry.seq` (per-WS-connection, resets on reconnect) — see
  `specs/issue-2013-topology-timeline-history.spec.md` Decisions.
- `TurnCard.tsx` — one turn = one card. Owns the reducer
  `buildTurnsFromEvents` that folds a flat `WebFrame` stream into
  `TurnCardData[]` (text, reasoning, tool calls, markers, metrics,
  usage). Splits on `done`.
- `SpawnMarker.tsx` — compact inline marker for `subagent_spawned`,
  `subagent_done`, `tape_forked`.
- `WorkerInbox.tsx` — right-rail derived view. The reducer
  `deriveWorkers` folds the same event buffer into one `WorkerInfo`
  per spawned child (status, manifest name, last activity seq, event
  count). Pure; re-runs via `useMemo`.
- `WorkerCard.tsx` — clickable card per worker; click swaps the
  `Topology` page's `viewChild` state so the timeline focuses on that
  child. The back-to-root affordance lives in the timeline header, not
  the inbox.
- `TapeLineageView.tsx` — right-rail panel (under the worker inbox)
  that renders the tape fork forest as a hand-drawn SVG. Default
  collapsed. Lives in the right rail rather than above the timeline so
  the centre column is dedicated to the conversation stream — matches
  the craft "right rail = meta" pattern.
  Pure SVG (no d3 / dagre) because tape forests are tiny (≤ a few dozen
  nodes per session) and a static layout keeps the view
  snapshot-testable. Highlights nodes whose `sessionKey` matches the
  current `viewSessionKey` so the panel and timeline stay visually
  linked. Click is intentionally not a navigation action — `tape ↔
session` is many-to-one, so a click would not unambiguously map to
  one worker; use the inbox to switch focus.
- `tape-tree-layout.ts` — pure reducer + layered layout. `buildTapeForest`
  folds `tape_forked` events into `{nodes, edges}`; `layoutTapeForest`
  assigns `(x, y)` by depth (column) and a stable per-session DFS order
  (row). Constants (`NODE_WIDTH`, `COL_GAP`, …) live next to the layout,
  not in config — they tune the mechanism, not deployment behavior.
- The cross-session topology WebSocket lives in
  `@/hooks/use-topology-subscription` (read-only stream of every event
  on root + descendants). The per-session **send** WebSocket lives in
  `@/hooks/use-chat-session-ws`, which is a thin React wrapper around
  `SessionWsClient` from `@/agent/session-ws-client` — it owns one
  socket per `sessionKey` for `prompt` / `abort` traffic. The two
  sockets are intentionally distinct: one fans-in events from many
  sessions, the other fans-out user input to one. The hook also
  defines the `TopologyWebFrame` union — an extension of `WebFrame`
  with the three topology variants. Keep them there until task #8
  collapses the per-session and topology clients.
- Model + skills caches live in `@/hooks/use-chat-models` and
  `@/hooks/use-skills`. Both are react-query hooks with `staleTime`
  matching the backend cache TTL (5 min for models, 1 min for skills).

Data flow:

```
GET /api/v1/chat/sessions
  → SessionPicker (left rail) → URL navigates to /topology/{key}
                                       │
backend StreamHub                       ▼
  → /api/v1/kernel/chat/topology/{root} WS  (TopologyFrame)
    → useTopologySubscription            (TopologyEventEntry[])
      ├→ TimelineView.filter(viewSessionKey)
      │    → buildTurnsFromEvents → TurnCard[]
      ├→ WorkerInbox.deriveWorkers
      │    → WorkerCard[]
      └→ TapeLineageView (buildTapeForest → layoutTapeForest)
           → SVG nodes + edges
```

## Critical Invariants

- **Reducer purity.** `buildTurnsFromEvents` must be pure and stable for
  the same input — `TimelineView` re-derives turns on every event push.
  Stash any mutable accumulator in `useMemo` deps, never in module
  scope.
- **Single-session filter.** `TimelineView` must filter `events` by
  `sessionKey === viewSessionKey` before reducing — never interleave
  multiple sessions in one column. A child's `done` would split the
  parent's turn (and vice versa), breaking per-turn boundaries. The
  `viewSessionKey` is the root by default; the worker inbox passes a
  child key to focus on a worker. Cross-session structure is task #7's
  fork topology view, not the timeline.
- **Mechanism constants stay in the hook.** Reconnect schedule lives in
  `use-topology-subscription` next to the socket logic, mirroring
  `session-ws-client`. Do NOT pull it out into config — see
  `docs/guides/anti-patterns.md`.

## What NOT To Do

- Do NOT add the topology WebFrame variants to
  `@/agent/session-ws-client`'s `WebFrame` union — that file is the
  per-session client and `RaraAgent` does not consume topology
  variants. Task #8 will unify the two.
- Do NOT render multiple sessions in one `TimelineView` instance. Use
  the `viewSessionKey` prop and let `WorkerInbox` switch focus instead.
  Tape fork lineage lives in `TapeLineageView`, not `TimelineView`.
- Do NOT make `TapeLineageView` nodes clickable for navigation. Tapes
  and sessions are not 1:1 (one session can host many fork tapes), so a
  click would not unambiguously map to one worker. Highlight by
  `viewSessionKey` is the link; navigation goes through the inbox.
- Do NOT pull in d3 / dagre / react-flow for the lineage SVG. The data
  is tiny (≤ a few dozen nodes per session), the layout is static, and
  a hand-drawn SVG keeps the bundle slim and the layout
  snapshot-testable. If the visualisation outgrows this, the right move
  is a paginated / collapsible per-session subtree, not a layout lib.
- Do NOT drop completed / failed workers from `WorkerInbox`. The
  surface is an observation deck — historical workers stay visible so
  operators can inspect what ran. If inbox length becomes a UX problem,
  add a filter, don't garbage-collect.
- Do NOT render `phase`, `progress`, `attachment`, `approval_*`, or
  `tape_appended` frames here yet — the reducer drops them on purpose
  to keep cards focused. Wire them in only when there's a concrete UI
  use; otherwise the card becomes a JSON dump.
- Do NOT replace `TopologyEventEntry.events: TopologyEventEntry[]`
  with a Map keyed by session — order across sessions is meaningful
  for the timeline (a child spawn marker must appear in the parent's
  turn at the right point).
- Do NOT auto-create a session when the list is empty — the empty
  state shows a `Create session` button instead. Auto-creating on
  mount produces phantom sessions every time someone visits
  `/topology` cold; the user must opt in.
- Do NOT bring back the free-text "root session key" input. The whole
  point of task #9 was to stop exposing session UUIDs to the user. If
  deep linking is needed, share the `/topology/{key}` URL — the
  picker will auto-select the matching row when the URL has a key.

## Dependencies

- `@/hooks/use-topology-subscription` — owns the WS lifecycle and
  exports the `TopologyWebFrame` extension union.
- `@/agent/session-ws-client` — type-only import for the base
  `WebFrame` union (kept synced with `crates/channels/src/web.rs`).
- `@/api/client` — REST helper for the session list / create calls
  the picker drives.
- `@tanstack/react-query` — `SessionPicker` uses the standard
  `useQuery`/`useMutation` pattern; the cache key
  (`['topology', 'chat-sessions']`) is the single invalidation point if
  a future surface needs to nudge the picker.
- `@/components/ui/{card,badge,button,input}` — local shadcn-style
  primitives.
