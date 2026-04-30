# web/src/components/topology — Agent Guidelines

## Purpose

Frontend for the multi-agent observability surface (umbrella issue
#1999). Renders the cross-session topology WebSocket stream
(`/api/v1/kernel/chat/topology/{root}`) as a main timeline of agent
turns plus a right-rail worker inbox of spawned subagents.

## Architecture

- `TimelineView.tsx` — vertical list of `TurnCard`s; filters the
  topology event buffer down to a single `viewSessionKey` (root by
  default; the worker inbox swaps in a child key when one is selected).
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
- `TapeLineageView.tsx` — collapsible panel above the timeline that
  renders the tape fork forest as a hand-drawn SVG. Default collapsed.
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
- The WebSocket plumbing lives in `@/hooks/use-topology-subscription`,
  not here. The hook also defines the `TopologyWebFrame` union — an
  extension of `WebFrame` (from `@/agent/session-ws-client`) with the
  three topology variants the backend forwards. Keep them there until
  task #8 collapses the per-session and topology clients.

Data flow:

```
backend StreamHub
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

## Dependencies

- `@/hooks/use-topology-subscription` — owns the WS lifecycle and
  exports the `TopologyWebFrame` extension union.
- `@/agent/session-ws-client` — type-only import for the base
  `WebFrame` union (kept synced with `crates/channels/src/web.rs`).
- `@/components/ui/{card,badge,button,input}` — local shadcn-style
  primitives.
