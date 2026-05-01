spec: task
name: "issue-2022-topology-collapsible-sidebar"
inherits: project
tags: []
---

## Intent

The topology page (`web/src/pages/Chat.tsx`) renders a fixed
3-pane shell: `SessionPicker` (left, 280px) | `TimelineView` (centre,
flex) | `WorkerInbox` + `TapeLineageView` (right, 320px). The left rail
has no toggle — once `md:` breakpoint is hit, the 280px column is
permanently parked and the operator cannot reclaim that horizontal
space for the timeline.

This is a UX affordance for the inspectability surface. On a typical
1366–1440px laptop, after both rails take 600px and the centre pane
reserves padding, the timeline ends up with ~700px of usable width.
Long tool-call output (JSON args, code blocks, reasoning text) wraps
aggressively or scrolls horizontally inside `TurnCard`. Operators
reading a trace want to widen the centre column without resizing the
window.

The desired behaviour mirrors the vendored craft pattern (see
`vendor/craft.png` and existing `web/src/vendor/craft-ui/components/icons/PanelLeftRounded.tsx`):
a small icon button in the topology header toggles `SessionPicker`
visibility; when collapsed the centre + right panes redistribute and
fill the freed width. The collapsed state should persist across page
reloads via `localStorage` so the operator does not have to re-toggle
every time they revisit `/topology`.

Reproducer:
1. Open `/topology/<key>` at a viewport ≥ 768px (so the `md:` rail is
   visible).
2. Inspect the DOM — `SessionPicker` is an `<aside class="hidden w-[280px] shrink-0 border-r border-border md:block">`
   in `Topology.tsx`. There is no toggle anywhere in the header or
   surrounding chrome.
3. The timeline column (`<main class="flex flex-1 min-w-0 min-h-0 flex-col p-3">`)
   has no way to claim the 280px the picker holds.
4. After fix: a `PanelLeft` icon button in the topology header toggles
   the picker; clicking it removes the picker from the DOM (or sets
   its container to `display:none`) and the centre pane expands. The
   state survives a page reload.

Prior art reviewed:
- PR 2003 / #1999 (multi-agent observability UI) landed the 3-pane
  shell on 2026-04-30 with the picker hard-coded as visible. Its
  AGENT.md (`web/src/components/topology/AGENT.md`) does not call
  collapsibility out as deferred — it was simply not in scope.
- `gh issue list --search "sidebar collapse" --state all` returned 0
  results. No prior issue tracks this.
- `gh pr list --search "sidebar"` returned chat-sidebar work
  (PR 1770, 1626, 1589, 1873, 1886) — none of them touch the topology
  shell.
- `git log --grep "sidebar|SessionPicker" --since=180.days` shows the
  topology picker was introduced by `8ec0dadb` (PR 1999 task) and has
  not been revisited.
- The vendored craft-ui already ships `PanelLeftRounded.tsx`,
  `PanelRightRounded.tsx`, and an `AppShellContext` with `localStorage`
  helpers (`vendor/craft-ui/lib/electron/local-storage.ts`). The
  smallest concrete approach reuses the icon and a small `useState` +
  `useEffect` localStorage pattern locally in `Topology.tsx` rather
  than wiring through `AppShellContext` (the topology page does not
  consume the AppShell — adopting it just for one toggle is overreach).
- The right rail (`WorkerInbox` + `TapeLineageView`) is intentionally
  out of scope. The user asked about the left sidebar specifically;
  the right rail already auto-hides at < `lg:` breakpoint so the
  motivation is weaker.

## Decisions

- **Collapse mechanism: conditional render, not CSS-only.** When
  collapsed, the `<aside>` is removed from the DOM. The `SessionPicker`
  owns a 30s react-query refetch interval, but `useQuery` keeps the
  cache hot across mount/unmount on the same `queryKey`, so re-expanding
  is instantaneous. Removing the node also lets the flex layout
  redistribute width without the picker holding a `min-content` floor.
- **Persistence: `localStorage` key `rara.topology.sidebarCollapsed`,
  boolean.** Read once on mount via lazy `useState` initializer; write
  on every change via `useEffect`. Default to `false` (sidebar shown)
  so a first-time visitor still sees the picker.
- **Toggle placement: topology header.** A `Button size="icon" variant="ghost"`
  with `PanelLeft` (or `PanelLeftClose`) from `lucide-react` (already
  the icon library used elsewhere in the page — `Network`, `ArrowLeft`)
  sits at the start of the header, before the `Network` icon. Single
  button, two states: shows `PanelLeftClose` when expanded, `PanelLeft`
  when collapsed. `aria-label` reflects the action ("Hide sidebar" /
  "Show sidebar"). `title` matches.
- **Scope: left sidebar only.** Right rail collapsibility is NOT in
  this spec. If operators ask later, file a separate issue.
- **No new constants.** The 280px width and `md:` breakpoint stay as
  they are. The toggle just controls visibility.

## Boundaries

### Allowed Changes

- **/web/src/pages/Chat.tsx
- **/web/src/pages/__tests__/Chat.test.tsx
- **/web/src/components/topology/AGENT.md
- **/specs/issue-2022-topology-collapsible-sidebar.spec.md

### Forbidden

- `web/src/components/topology/SessionPicker.tsx`
- `web/src/components/topology/WorkerInbox.tsx`
- `web/src/components/topology/TapeLineageView.tsx`
- `web/src/components/topology/TimelineView.tsx`
- `web/src/vendor/**`
- `crates/**`
- `web/src/hooks/**`

## Acceptance Criteria

```gherkin
Feature: Collapsible left sidebar on the topology page

  Scenario: Toggle hides the SessionPicker from the DOM
    Given the topology page is rendered at a viewport that shows the left rail
      And the sidebar is in the default expanded state
    When the user clicks the sidebar toggle button in the topology header
    Then the SessionPicker is no longer present in the DOM
      And the centre timeline column expands to fill the freed width
    Test: web/src/pages/__tests__/Chat.test.tsx::toggle_hides_session_picker

  Scenario: Toggle restores the SessionPicker
    Given the topology page is rendered with the sidebar collapsed
    When the user clicks the sidebar toggle button
    Then the SessionPicker is mounted and visible again
    Test: web/src/pages/__tests__/Chat.test.tsx::toggle_restores_session_picker

  Scenario: Collapsed state persists across reloads
    Given the user has collapsed the sidebar
    When the topology page is unmounted and remounted
       (simulating a full page reload backed by the same localStorage)
    Then the SessionPicker is not rendered on the next mount
      And the toggle button reflects the collapsed state
    Test: web/src/pages/__tests__/Chat.test.tsx::collapsed_state_persists

  Scenario: Default state for a first-time visitor is expanded
    Given localStorage has no entry for the sidebar collapse key
    When the topology page mounts
    Then the SessionPicker is rendered
    Test: web/src/pages/__tests__/Chat.test.tsx::default_state_is_expanded

  Scenario: localStorage access failure falls back to default
    Given localStorage throws on read (private browsing or disabled storage)
    When the topology page mounts
    Then the SessionPicker is rendered (default expanded state)
      And the page does not crash or surface the error to the user
    Test: web/src/pages/__tests__/Chat.test.tsx::localstorage_failure_falls_back
```

## Constraints

- All code comments and identifiers in English.
- No new dependencies (icon comes from `lucide-react` which the page
  already imports).
- The `localStorage` read must be resilient to access errors (private
  browsing, disabled storage) — fall back to the default `false`
  rather than throwing.
- Do not introduce a YAML config knob for the default state — this is
  a per-user UX preference, not a deployment concern (see
  `docs/guides/anti-patterns.md` "mechanism constants are not config").
