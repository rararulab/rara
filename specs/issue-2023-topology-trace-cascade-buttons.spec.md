spec: task
name: "issue-2023-topology-trace-cascade-buttons"
inherits: project
tags: [enhancement, ui, web]
---

## Intent

The topology page renders historical and live assistant turns through the
vendor `TurnCard` (`web/src/vendor/craft-ui/components/chat/TurnCard.tsx`,
landed in main via PR 2018), bridged by `RaraTurnCard.tsx`. The vendor
already exposes the chrome we need: an actions dropdown
(`TurnCardActionsMenu`, opened by clicking a three-dot trigger in the
turn header) that surfaces an `onOpenDetails?: () => void` slot, plus an
`onOpenActivityDetails?: (activity: ActivityItem) => void` slot wired
into every completed activity row. There is no rara-side wiring on
either slot today, so neither the per-turn execution trace nor the
cascade context is reachable from the UI.

The backend still serves the data: `crates/extensions/backend-admin/src/chat/router.rs`
registers `get_execution_trace` and `get_cascade_trace` (`GET /api/v1/chat/sessions/{key}/trace?seq=<n>`
and the cascade sibling). `web/src/api/kernel-types.ts` retains
`ExecutionTrace` and `CascadeTrace` TypeScript types. PR 1826 / commit
356dd74e shipped the equivalent affordance on the retired `/chat-v2`
page (the "📊 详情" / "🔍 Cascade" buttons + `ExecutionTraceModal` +
`CascadeModal`); PR 1833 retired pi-web-ui and removed the modals, the
client wrappers, and the hooks. This spec rewires the now-existing
vendor slots to the still-existing backend.

If we do not do this, the following concrete bug appears. Reproducer:

1. Run frontend against the remote backend (`VITE_API_URL=http://10.0.0.183:25555 bun run dev`).
2. Open the topology page; let history load. Each completed assistant
   turn renders as a vendor `TurnCard` with text, tool calls, and a
   header — but the three-dot actions menu does not render (the vendor
   short-circuits when both `onOpenDetails` and `onOpenMultiFileDiff`
   are undefined, see `TurnCardActionsMenu.tsx` lines 39-42), and
   activity rows are not click-through to detail.
3. The user wants to know which intermediate iterations / token usage /
   raw tool args produced this reply. There is no path from the UI to
   the `/api/v1/chat/sessions/{key}/trace?seq=<n>` endpoint that the
   kernel already serves. The user has to `curl` the API by hand or
   read remote logs to inspect a turn — exactly the regression
   `goal.md` signal 4 ("every action inspectable through native eval
   interfaces") forbids.

This advances goal.md signal 4. It is a legitimate restoration scoped
to wiring: PR 1826 shipped the buttons, PR 1833 removed them as
collateral when the entire chat surface was replaced, PR 2018 brought
in vendor surfaces that already model the same affordance. The risk
PR 1672 / issue 1672 caught (buttons leaking onto every persisted row
including intermediate iterations) is structurally gone — the vendor's
`onOpenDetails` is per-turn and only fires when the turn is complete,
so per-message leakage cannot recur as long as the rara adapter
threads `finalSeq` at turn granularity (one seq per turn, not per
event).

`TurnCardData` (`TurnCard.tsx` lines 29-55) currently has no `seq`
field. The id is `turn-${seq}` of the first event but the seq itself
is not threaded through. Wiring the actions menu requires adding a
`finalSeq: number | null` field to `TurnCardData`, populated by
`buildTurnsFromHistory` from the persisted assistant row's seq for
history-sourced turns and from the kernel-assigned seq on the `done`
frame for live turns. `null` means "no inspect affordance" (live frame
mid-stream, or any case where the seq is not yet known); the adapter
leaves `onOpenDetails` undefined in that case and the vendor hides the
chrome (`TurnCardActionsMenu` lines 39-42).

Prior art surveyed:

- PR 2018 — landed vendor `TurnCard` + `RaraTurnCard` adapter. The
  adapter (`web/src/components/topology/RaraTurnCard.tsx`) currently
  leaves `onOpenDetails` and `onOpenActivityDetails` undefined and
  comments that "rara doesn't yet expose those surfaces" — this spec
  is the surface.
- PR 1826 / commit 356dd74e — original buttons on `/chat-v2`. Diff is
  the modal-content blueprint (what the trace modal showed: iteration
  count, model, token usage, raw tool args; what the cascade modal
  showed: the cascade chain). Do not import — files have been
  deleted; write fresh React modal components inspired by the
  original layout.
- PR 1672 — gating bug on chat-v2: buttons leaked onto every persisted
  assistant row including intermediate iterations. Mitigated
  structurally by per-turn `finalSeq` threading (vendor's
  `onOpenDetails` fires once per turn).
- PR 1798 — consolidated trace pills. Visual reference, not reusable.
- PR 1611 — alignment fix on the old detail button. Irrelevant given
  the vendor's three-dot menu owns placement.
- `web/src/api/kernel-types.ts` retains `ExecutionTrace` /
  `CascadeTrace` types. Reuse as-is.
- Backend endpoints exist: `crates/extensions/backend-admin/src/chat/router.rs`
  registers `get_cascade_trace` and `get_execution_trace`. No backend
  work needed.

## Decisions

- Add `finalSeq: number | null` to `TurnCardData`. History path
  populates it from the last persisted assistant row of the turn. Live
  path leaves it `null` until the kernel emits the `done` frame with
  the assigned seq, then sets it.
- The adapter `RaraTurnCard` wires `onOpenDetails` (opens the
  execution trace modal) only when `turn.finalSeq !== null` and
  `turn.inFlight === false`. When either is unmet, leave the prop
  undefined; the vendor hides the three-dot menu entirely
  (`TurnCardActionsMenu.tsx` lines 39-42). This is the structural
  equivalent of PR 1826's `metadata.seq !== undefined` gate, lifted
  to the turn level so PR 1672's per-message footgun cannot recur.
- The cascade affordance binds to a different surface than the trace.
  Trace is per-turn (the actions menu's "view turn details" item).
  Cascade is contextual to a specific tool call — wire it through
  `onOpenActivityDetails(activity)` so clicking a tool row opens
  the cascade modal scoped to that turn's `finalSeq` (the vendor
  already routes activity-row clicks to this prop, see
  `TurnCard.tsx` lines 3020 and 3043). If the implementer finds
  `onOpenActivityDetails` does not match the desired UX, they may
  instead use `renderActionsMenu` to inject a custom dropdown with
  both items; that decision is deferred to implementation, with
  rationale recorded in the PR.
- Modal components (`ExecutionTraceModal.tsx`, `CascadeModal.tsx`)
  are fresh React under `web/src/components/topology/`. PR 1826 is
  the layout reference, not an import source.
- API client wrappers live in `web/src/api/sessions.ts` as
  `fetchExecutionTrace(sessionKey, seq)` and
  `fetchCascadeTrace(sessionKey, seq)`. Both return the existing
  types from `kernel-types.ts`; no new types.
- One `useTraceFetch` hook keyed on `(sessionKey, seq, kind)`,
  fetching only on modal open (lazy). No pre-fetch on card render.
- The vendor `TurnCard.tsx` is **not edited** — all rara-side
  customisation flows through `RaraTurnCard.tsx`'s prop wiring.
- **Lifecycle gate**: `agent-spec lifecycle` cannot exercise these
  scenarios end-to-end yet — issue 2015 (open) tracks the missing
  vitest adapter for `agent-spec`. The implementer runs the vitest
  suite directly (`cd web && bun run test`) as the verification
  signal until 2015 lands. Reviewer may APPROVE on green vitest +
  manual smoke against the remote backend.

## Boundaries

### Allowed Changes
- **/web/src/components/topology/RaraTurnCard.tsx
- **/web/src/components/topology/TurnCard.tsx
- **/web/src/components/topology/TimelineView.tsx
- **/web/src/components/topology/ExecutionTraceModal.tsx
- **/web/src/components/topology/CascadeModal.tsx
- **/web/src/hooks/use-session-timeline.ts
- **/web/src/hooks/use-trace-fetch.ts
- **/web/src/api/sessions.ts
- **/web/src/components/topology/__tests__/**
- **/specs/issue-2023-topology-trace-cascade-buttons.spec.md

### Forbidden
- **/crates/**
- **/web/src/vendor/**
- **/web/src/api/kernel-types.ts
- **/web/src/components/topology/AGENT.md

The crates path is forbidden because the backend endpoints already
exist and were verified before drafting; if the implementer thinks
the backend needs changes, that is grounds to escalate to spec-author,
not silently expand scope. The vendor path is forbidden because
craft-ui is a vendored upstream — fork-edits make future syncs
painful and the public prop surface is sufficient for this work.
`kernel-types.ts` is forbidden because the existing `ExecutionTrace`
/ `CascadeTrace` types are sufficient — re-deriving them from the
wire shape would split the contract. `AGENT.md` updates are deferred
until the modals settle in their final home.

## Completion Criteria

Scenario: Actions menu is wired only when finalSeq is present and the turn has completed
  Test:
    Package: web
    Filter: RaraTurnCard__actions_menu_wired_when_finalSeq_present_and_not_inflight
  Given a RaraTurnCard rendered with `finalSeq = 42` and `inFlight = false`
  When the card is rendered
  Then the vendor's three-dot actions trigger is in the DOM
    And clicking it reveals the "view turn details" item

Scenario: Actions menu is suppressed on in-flight turns and on turns without a known seq
  Test:
    Package: web
    Filter: RaraTurnCard__actions_menu_suppressed_when_inflight_or_seq_null
  Given a RaraTurnCard rendered with `inFlight = true` and `finalSeq = null`
  When the card is rendered
  Then no actions trigger is in the DOM
    And no "view turn details" item is reachable

Scenario: Selecting "view turn details" opens a modal that fetches and displays the trace
  Test:
    Package: web
    Filter: RaraTurnCard__trace_modal_opens_with_fetched_content
  Given a RaraTurnCard with `finalSeq = 42` for session key "sess-abc"
    And the trace API returns a known ExecutionTrace payload (mocked at the fetch layer)
  When the user opens the actions menu and selects "view turn details"
  Then a modal becomes visible
    And the modal shows content derived from the mocked ExecutionTrace payload (e.g. an iteration count or model name from the fixture)

Scenario: Activating an activity row on a completed turn opens the cascade modal scoped to the turn
  Test:
    Package: web
    Filter: RaraTurnCard__cascade_modal_opens_from_activity_row
  Given a RaraTurnCard with `finalSeq = 42` for session key "sess-abc" and at least one completed tool activity
    And the cascade API returns a known CascadeTrace payload (mocked at the fetch layer)
  When the user clicks the tool activity row
  Then a modal becomes visible
    And the modal shows content derived from the mocked CascadeTrace payload

Scenario: Trace fetch failure surfaces an error in the modal instead of crashing the card
  Test:
    Package: web
    Filter: RaraTurnCard__trace_modal_shows_error_on_fetch_failure
  Given a RaraTurnCard with `finalSeq = 42` for session key "sess-abc"
    And the trace API rejects with a network error (mocked at the fetch layer)
  When the user opens the actions menu and selects "view turn details"
  Then the modal becomes visible
    And the modal shows an error indicator (not blank, not a thrown exception)
    And the surrounding TurnCard is still rendered

## Out of Scope

- Backend changes. The trace and cascade endpoints already exist
  (`crates/extensions/backend-admin/src/chat/router.rs`).
- Vendor `TurnCard` edits. All rara-side wiring flows through
  `RaraTurnCard.tsx` props.
- Modifying `kernel-types.ts` — the existing `ExecutionTrace` /
  `CascadeTrace` types are reused as-is.
- Adding inspect affordances to non-turn surfaces (user bubbles, spawn
  markers, worker cards). Only `RaraTurnCard` is in scope.
- Pre-fetching trace / cascade on card render. Fetch on modal open only.
- Visual redesign of the modals beyond what is needed to render the
  fetched payload legibly. Polish iteration is a follow-up if needed.
- Building the `agent-spec` vitest adapter — issue 2015 owns that.
  Verification here is direct vitest + manual smoke.
- Changing where `finalSeq` is sourced for live turns beyond reading
  the kernel-assigned seq from the `done` frame. If the live path
  turns out not to surface seq at all, the implementer should leave
  live-turn wiring disabled (consistent with PR 1826's "live-only
  frames stay button-less") and surface that observation, not invent
  a workaround.
