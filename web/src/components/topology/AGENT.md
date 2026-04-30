# web/src/components/topology — Agent Guidelines

## Purpose

Frontend for the multi-agent observability surface (umbrella issue
#1999). Renders the cross-session topology WebSocket stream
(`/api/v1/kernel/chat/topology/{root}`) as a main timeline of agent
turns with inline spawn / done / fork markers.

## Architecture

- `TimelineView.tsx` — vertical list of `TurnCard`s; filters the
  topology event buffer down to the root session.
- `TurnCard.tsx` — one turn = one card. Owns the reducer
  `buildTurnsFromEvents` that folds a flat `WebFrame` stream into
  `TurnCardData[]` (text, reasoning, tool calls, markers, metrics,
  usage). Splits on `done`.
- `SpawnMarker.tsx` — compact inline marker for `subagent_spawned`,
  `subagent_done`, `tape_forked`.
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
      → TimelineView.filter(rootSessionKey)
        → buildTurnsFromEvents
          → TurnCard[]
```

## Critical Invariants

- **Reducer purity.** `buildTurnsFromEvents` must be pure and stable for
  the same input — `TimelineView` re-derives turns on every event push.
  Stash any mutable accumulator in `useMemo` deps, never in module
  scope.
- **Root-only filter.** `TimelineView` must filter `events` by
  `sessionKey === rootSessionKey` before reducing. Descendant events
  are tasks #6 (worker inbox) / #7 (fork topology) — rendering them in
  the main timeline would double-count and break per-turn boundaries.
- **Mechanism constants stay in the hook.** Reconnect schedule lives in
  `use-topology-subscription` next to the socket logic, mirroring
  `session-ws-client`. Do NOT pull it out into config — see
  `docs/guides/anti-patterns.md`.

## What NOT To Do

- Do NOT add the topology WebFrame variants to
  `@/agent/session-ws-client`'s `WebFrame` union — that file is the
  per-session client and `RaraAgent` does not consume topology
  variants. Task #8 will unify the two.
- Do NOT render descendant-session events in `TimelineView`. Tasks #6
  (worker inbox right rail) and #7 (fork topology) own descendant
  rendering.
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
