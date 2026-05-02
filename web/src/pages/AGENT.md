# web/src/pages — Agent Guidelines

## Purpose

Top-level routed pages mounted by `App.tsx`. Each file is the root of a
single route under `DashboardLayout`.

## Architecture

- `Chat.tsx` — default landing route (`/`) and `/chat[/:rootSessionKey]`.
  Renders the multi-agent observability view: timeline, worker inbox, and
  fork lineage panels driven by `useTopologySubscription`. The product
  surface is "Chat"; the underlying WS subscription / data model is still
  named "topology" because it carries the parent-child session tree
  (see `web/src/components/topology/`). Old `/topology[/:rootSessionKey]`
  links 302-redirect to the corresponding `/chat` route — see #2041.
- `Docs.tsx`, `Login.tsx` — single-purpose admin pages.
- `Agents.tsx`, `CodingTasks.tsx`, `McpServers.tsx`, `Scheduler.tsx`,
  `Skills.tsx` — settings-adjacent pages not currently wired into the
  router; consult before deleting.

## Critical Invariants

- The index route MUST resolve to a real component, not a redirect — the
  router treats `index` as a sibling of named routes, and a `Navigate`
  inside the layout would re-mount on every navigation.
- Pages are layout consumers. They MUST NOT render their own top-bar or
  global nav rail; that work belongs to `DashboardLayout` and
  `@/components/shell/NavRail`. The page title is surfaced to the slim
  top bar via the route's `handle: { title, showLiveIndicator? }`
  metadata read by `DashboardLayout` through `useMatches()` (#2059); a
  page that owns a long-lived subscription publishes its live state via
  `usePublishPageStatus` (see `@/components/shell/PageStatusContext`)
  instead of opening a second WebSocket from the layout. Per-page
  internal columns (e.g. `Chat.tsx`'s sessions sidebar) are still the
  page's own concern — the rule only forbids re-rendering app chrome.

## What NOT To Do

- Do NOT re-introduce a chat-style fullscreen page that bypasses
  `DashboardLayout` — `Chat.tsx` is the multi-agent observability view
  (#1999) and the chat replacement. A second fullscreen route would split
  the navigation surface again.
- Do NOT add page-local data fetching for resources already exposed by a
  shared hook in `@/hooks` — the hook is the cache key boundary.
- Do NOT re-introduce `KernelTop.tsx` or `Subscriptions.tsx` admin pages
  (removed in #2041) without surfacing the IA decision first — they were
  leftovers from #1476 / #1743 that no current flow depends on.

## Dependencies

- `@/api/*` — backend client per resource.
- `@/hooks/use-topology-subscription` — WebSocket-backed event stream
  consumed by `Chat.tsx`.
- `@/components/topology/*` — view modules used only by `Chat.tsx`.
