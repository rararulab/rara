# web/src/pages — Agent Guidelines

## Purpose

Top-level routed pages mounted by `App.tsx`. Each file is the root of a
single route under `DashboardLayout`.

## Architecture

- `Topology.tsx` — default landing route (`/`) and `/topology[/:rootSessionKey]`.
  Renders the multi-agent observability view: timeline, worker inbox, and
  fork lineage panels driven by `useTopologySubscription`.
- `KernelTop.tsx` — kernel sessions overview; mounts `SessionList` +
  `SessionDetail` and the approvals drawer.
- `Subscriptions.tsx`, `Docs.tsx`, `Login.tsx` — single-purpose admin
  pages.
- `Agents.tsx`, `CodingTasks.tsx`, `McpServers.tsx`, `Scheduler.tsx`,
  `Skills.tsx` — settings-adjacent pages not currently wired into the
  router; consult before deleting.

## Critical Invariants

- The index route MUST resolve to a real component, not a redirect — the
  router treats `index` as a sibling of named routes, and a `Navigate`
  inside the layout would re-mount on every navigation.
- Pages are layout consumers. They MUST NOT render their own top-bar /
  sidebar; that work belongs to `DashboardLayout`.

## What NOT To Do

- Do NOT re-introduce a chat-style fullscreen page that bypasses
  `DashboardLayout` — the multi-agent observability view (Topology) is
  the chat replacement (#1999). A second fullscreen route would split
  the navigation surface again.
- Do NOT add page-local data fetching for resources already exposed by a
  shared hook in `@/hooks` — the hook is the cache key boundary.

## Dependencies

- `@/api/*` — backend client per resource.
- `@/hooks/use-topology-subscription` — WebSocket-backed event stream
  consumed by `Topology.tsx`.
- `@/components/topology/*` — view modules used only by `Topology.tsx`.
- `@/components/kernel/*` — view modules used only by `KernelTop.tsx`.
