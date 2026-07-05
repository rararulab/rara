spec: task
name: "issue-2197-fleet-inbox-ui"
inherits: project
tags: []
---

## Intent

After issues 2194 and 2195, rara can dispatch a fleet task and record its
result contract — but the only way to see any of it is `curl` against the
read API or `sqlite3` against the DB. The existing worker inbox
(`web/src/components/topology/WorkerInbox.tsx`, landed in PR 2003) renders
only in-process child sessions; out-of-process fleet workers are invisible
in the UI.

Reproducer for the failure mode:

1. The user dispatches a coding task via chat and switches to the rara web
   UI to watch it.
2. The worker inbox right rail shows nothing — the fleet task is not a
   child session, so no card appears while it runs.
3. Observed bad outcome: the task completes with a branch, PR URL, and
   cost, and the UI never shows any of it; the user falls back to asking
   rara "how is that task going?" — the exact regression of goal signal 2
   — or to reading the DB by hand.

This issue renders fleet tasks in the existing worker-inbox surface (the
hybrid main-timeline + right-rail-inbox direction already decided for
multi-agent UI): a fleet section in the inbox listing dispatched tasks
with live status, and, for terminal tasks, the result contract — branch,
PR link, token usage, cost. Data comes from polling issue 2195's read API.

Goal alignment: signal 4 ("every action is inspectable" — dispatched work
becomes visible, not a black box) and signal 2 ("the user stops asking").
NOT lines: display-only — no new task producers, no multi-user surface,
rara stays the orchestrator. Hermes parity: not applicable — UI for a
rara-specific subsystem.

Prior art reviewed (raw):

- Issue 1999 / PR 2003 — multi-agent observability UI: `WorkerInbox.tsx`,
  `WorkerCard.tsx`, `TimelineView.tsx` under `web/src/components/topology/`
  (see its AGENT.md), hooks `use-session-timeline.ts` /
  `use-topology-subscription.ts`. The fleet section extends this surface
  instead of adding a new page — the hybrid timeline+inbox direction is a
  recorded decision.
- Issue 2022 (`specs/issue-2022-topology-collapsible-sidebar.spec.md`) —
  the lane-1 FE precedent: vitest selectors under
  `web/src/pages/__tests__/` binding BDD scenarios to component tests.
- `web/package.json` — vitest is already wired (`bun run test`); no new
  dependencies are needed or allowed.
- `gh issue list --search "inbox fleet ui"` — nothing beyond the fleet
  series (issues 2194, 2195, 2196). No prior decision reversed.

## Decisions

- **Placement**: a fleet-tasks section inside the existing worker inbox
  right rail — a sibling list to the child-session workers, not a new
  page. New components live in `web/src/components/topology/` (e.g.
  `FleetTaskCard.tsx`), following `WorkerCard.tsx` conventions (status
  badge styling, card layout).
- **Data**: a `use-fleet-tasks` hook in `web/src/hooks/` polling
  `GET /api/v1/fleet/tasks` on an interval, following the data-fetch
  conventions of the existing hooks. Polling only — no WebSocket plumbing
  in this issue (recorded as out of scope in issue 2195; revisit if
  polling proves laggy).
- **Card content**: agent type, truncated prompt (CJK-safe truncation —
  use code-point/grapheme-aware truncation, never byte slicing; issue 2138
  is the standing reminder), status badge for
  queued/running/succeeded/failed/lost, relative created/completed time.
  Terminal cards additionally render: branch name, PR URL as an external
  link, token usage, and cost. Failed/lost cards surface the error text.
- **Empty state**: when the API returns no tasks, the fleet section
  renders nothing (no empty-shell noise in the rail).
- **API client**: typed fetch added beside the existing API client code in
  `web/src/api/`, matching issue 2195's response shape (which never
  includes `callback_token`).
- **Tests**: vitest component tests under
  `web/src/components/topology/__tests__/FleetTasks.test.tsx`, mocking the
  HTTP layer — so this issue's BDD binding runs without a live backend,
  keeping the PR independently verifiable while the runtime path depends
  on issue 2195 being merged.
- **Quality gate**: frontend lane — `bun run build` + ESLint + vitest;
  before/after screenshots against the local stack once 2195 is available.

## Boundaries

### Allowed Changes
- web/src/components/topology/**
- **/web/src/components/topology/**
- web/src/hooks/**
- **/web/src/hooks/**
- web/src/api/**
- **/web/src/api/**
- specs/issue-2197-fleet-inbox-ui.spec.md
- **/specs/issue-2197-fleet-inbox-ui.spec.md

### Forbidden
- web/package.json
- web/bun.lock
- web/vite.config.ts
- crates/**
- config.example.yaml
- .github/workflows/**

## Completion Criteria

Scenario: fleet tasks from the API render in the worker inbox
  Test: web/src/components/topology/__tests__/FleetTasks.test.tsx::renders_fleet_tasks_from_api
  Given the fleet tasks API is mocked to return one running and one succeeded task
  When the worker inbox renders
  Then both tasks appear as cards with their agent type and status badge

Scenario: a terminal task shows the result contract
  Test: web/src/components/topology/__tests__/FleetTasks.test.tsx::terminal_task_shows_pr_link_and_cost
  Given a succeeded task with branch, pr_url, token usage, and cost_usd
  When its card renders
  Then the branch name, a link pointing at pr_url, and the formatted token/cost figures are visible

Scenario: a failed task surfaces its error
  Test: web/src/components/topology/__tests__/FleetTasks.test.tsx::failed_task_shows_error
  Given a failed task with a non-empty error field
  When its card renders
  Then the failed badge and the error text are visible

Scenario: the fleet section is absent when there are no tasks
  Test: web/src/components/topology/__tests__/FleetTasks.test.tsx::hides_fleet_section_when_empty
  Given the fleet tasks API is mocked to return an empty list
  When the worker inbox renders
  Then no fleet section heading or card is rendered

Scenario: a CJK prompt is truncated without breaking characters
  Test: web/src/components/topology/__tests__/FleetTasks.test.tsx::cjk_prompt_truncates_safely
  Given a task whose prompt is a long CJK string
  When its card renders
  Then the truncated prompt renders valid characters with no replacement glyphs

## Out of Scope

- Live push updates over the per-session WebSocket — polling first;
  follow-up issue if needed.
- Dispatching or cancelling tasks from the UI — the only producer is the
  agent's `fleet_dispatch` tool.
- Main-timeline rendering of the completion summary — that arrives
  through the normal chat stream via issue 2195's session event and needs
  no dedicated UI work here.
- Backend changes of any kind — if the read API shape needs adjustment,
  that is a follow-up on the backend, not a `crates/**` edit in this PR.
- Filtering, pagination, or a dedicated fleet history page.
