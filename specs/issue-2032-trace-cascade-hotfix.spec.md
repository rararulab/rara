spec: task
name: "issue-2032-trace-cascade-hotfix"
inherits: project
tags: [bug, ui, web]
---

## Intent

PR 2028 (commit `bdda47e2`, "feat(web): wire trace + cascade buttons via
vendor TurnCard slots") landed without a manual browser smoke pass —
reviewer round was skipped on a subscription-quota failure mode and CI's
vitest scenarios alone did not falsify three runtime regressions that
appear only against a real backend session. This spec is the hotfix
contract for those three bugs in the wiring layer
(`web/src/components/topology/RaraTurnCard.tsx`, the modals, the trace
hook, and the api wrapper).

The browser smoke evidence comes from a real session
`d6e905d9-fd62-41ca-8918-97b37276f534` (8 user / 17 assistant / 6
tool_result messages) loaded against the remote backend at
`10.0.0.183:25555`.

If we do not do this, the following concrete bug appears. Reproducer:

1. `VITE_API_URL=http://10.0.0.183:25555 bun run dev` and open the
   topology page on session `d6e905d9-...`. History loads.
2. Bug A — three-dot actions menu never renders. Console shows the
   React warning "Cannot update a component (TurnCardActionsMenu)
   while rendering a different component (SimpleDropdown)". DOM
   probe `document.querySelectorAll('svg.lucide-more-horizontal').length`
   is 0 even on hover of every assistant turn header. The
   "view turn details" affordance — the entire trace-modal entry
   point — is permanently unreachable.
3. Bug B — clicking a "thinking" activity row opens the cascade
   modal. Per `RaraTurnCard.tsx` line 121 (current main) the
   `onOpenActivityDetails` callback ignores its `activity` argument
   and unconditionally opens cascade. Vendor passes the activity
   for both `tool` and `intermediate` rows
   (`vendor/.../TurnCard.tsx` lines 917 and 1317). Reviewer flagged
   this as a P2 on PR 2028; merge happened anyway.
4. Bug C — trace modal returns 404 on every assistant turn. The
   trace fetch is `GET /api/v1/chat/sessions/<key>/execution-trace?seq=22`
   and backend responds `404 {"message":"user message at seq 22 has
   no rara_turn_id metadata"}`. Root cause is a backend seq-space
   divergence (see Decisions). Frontend cannot fix this without
   re-implementing backend tape counting, which is outside the
   wiring layer.

Bad outcome: the affordance shipped by PR 2028 (whose entire purpose
was to advance `goal.md` signal 4 — "every action is inspectable
through native eval interfaces") is non-functional for every real
user. Signal 4 is structurally violated again until the hotfix lands.

Goal alignment: same as PR 2028 — advances `goal.md` signal 4 by
restoring the inspectability primitive for the existing backend
endpoints. Crosses no `What rara is NOT` line.

## Decisions

- Seq decision (b): frontend `finalSeq` is the right seq from the
  data we have; the backend has two endpoints in the same router
  with inconsistent seq counters. Backend evidence
  (`crates/extensions/backend-admin/src/chat/service.rs`):
  `tap_entries_to_chat_messages` (lines 966-1084, used by
  `list_messages` — the source of every `ChatMessageData.seq` the
  frontend sees) increments `seq += 1` per result inside a
  `ToolResult` entry (line 1059, inside
  `for (i, result) in results.iter().enumerate()`).
  `get_execution_trace` (lines 766-790) increments `seq += 1` once
  for the whole `ToolResult` entry (line 783). A turn with N>1
  parallel tool results drifts the `list_messages` seq forward by
  N-1 past `get_execution_trace`'s view of the same turn. The 404
  message is misleading — `get_execution_trace`'s walk lands on the
  wrong (one earlier) user-message TapEntry, which legitimately
  lacks `rara_turn_id` because trace metadata is written on the
  current turn's user entry, not the prior one. This hotfix does
  NOT fix the backend; a sibling backend issue must align the two
  seq counters and is escalated separately. Until that lands, the
  trace modal surfaces a clean error UI; the degradation is
  explicit and inspectable, not silent.

- Vendor menu workaround: pass `renderActionsMenu` to the vendor
  `TurnCard` and own the dropdown rendering on the rara side.
  Vendor evidence: `vendor/.../TurnCard.tsx` line 2956 reads
  `renderActionsMenu ? renderActionsMenu() : <TurnCardActionsMenu .../>`
  — the override slot is callable from the rara adapter without
  editing vendor files. The "setState during render" warning
  originates inside vendor `SimpleDropdown.setItemRef` (lines
  159-170 of `vendor/.../ui/SimpleDropdown.tsx`): the callback ref
  synchronously calls `setHighlightedId(id)` during a child
  `SimpleDropdownItem`'s mount render. This is a vendor bug
  (upstream `craft-agents-oss@d9c585b8`), not a rara-side
  controlled-state cycle. Editing vendor files is forbidden;
  `renderActionsMenu` lets us skip `SimpleDropdown` entirely. The
  custom dropdown is a small purpose-built popover (button +
  `useState` + click-outside handler) rendered as
  `RaraTurnCardActionsMenu`. It must render a `MoreHorizontal`
  icon (lucide-react) so the existing browser smoke probe
  `svg.lucide-more-horizontal` passes.

- Cascade modal gating: `RaraTurnCard.tsx` checks
  `activity.type === 'tool'` before opening the cascade modal.
  `intermediate` (thinking) and any other future activity types
  are ignored. Vendor already passes the activity to the
  callback (`vendor/.../TurnCard.tsx` lines 1323 and 1328).

- Trace modal failure UI: when the trace fetch returns 404 with
  the specific backend message about `rara_turn_id` metadata, the
  modal shows a human-readable line ("Trace data is not available
  for this turn yet") rather than the raw backend string. Other
  HTTP errors stay surfaced verbatim — only the seq-divergence 404
  gets the friendlier copy.

## Boundaries

### Allowed Changes

- `web/src/components/topology/RaraTurnCard.tsx`
- `web/src/components/topology/RaraTurnCardActionsMenu.tsx`
- `web/src/components/topology/ExecutionTraceModal.tsx`
- `web/src/components/topology/CascadeModal.tsx`
- `web/src/components/topology/__tests__/RaraTurnCard.test.tsx`
- `web/src/hooks/use-trace-fetch.ts`
- `web/src/api/sessions.ts`

### Forbidden

- `web/src/vendor/**`
- `crates/**`
- `web/src/components/topology/TurnCard.tsx`

## Completion Criteria

Scenario: actions menu renders in the DOM on a complete assistant turn
  Test:
    Package: web
    Filter: actions_menu_renders_in_dom
  Given a TurnCardData with finalSeq 22 and inFlight false
  When the RaraTurnCard is mounted and the user hovers the header
  Then the document contains at least one svg.lucide-more-horizontal element
  And no "Cannot update a component" warning is emitted

Scenario: clicking a thinking activity does not open cascade
  Test:
    Package: web
    Filter: cascade_modal_only_on_tool_rows
  Given a RaraTurnCard with one thinking activity and one completed tool activity
  When the user clicks the thinking activity row
  Then the cascade modal is not present in the DOM
  When the user clicks the tool activity row
  Then the cascade modal is present in the DOM

Scenario: trace modal shows friendly copy on the seq-divergence 404
  Test:
    Package: web
    Filter: trace_modal_friendly_404
  Given the trace endpoint returns 404 with the rara_turn_id-metadata message
  When the user opens the trace modal on the affected turn
  Then the modal renders "Trace data is not available for this turn yet"
  And the raw backend error string is not rendered

Scenario: trace modal surfaces non-404 errors distinctly
  Test:
    Package: web
    Filter: trace_modal_non_404_error_distinct
  Given the trace endpoint returns 500 with body "internal error"
  When the user opens the trace modal
  Then the modal renders an error UI distinct from the friendly 404 copy

## Out of Scope

- Backend seq-counter alignment between `tap_entries_to_chat_messages`,
  `get_execution_trace`, and `get_cascade_trace`. Escalated to a
  sibling backend issue.
- Any changes to `TurnCard.tsx`'s `buildTurnsFromHistory` reducer —
  `finalSeq` semantics there are correct given the data the frontend
  has. The bug is on the backend side.
- Replacing or upgrading the vendor `craft-ui` package.
