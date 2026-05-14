spec: task
name: "issue-2087-response-footer-trace-cascade"
inherits: project
tags: [enhancement, ui, web]
---

## Intent

The web chat surface currently has no visible Trace or Cascade affordance
on pure-text LLM responses. PR 2028 / spec
`specs/issue-2023-topology-trace-cascade-buttons.spec.md` wired both
affordances through the vendor `TurnCard`'s **activity area** —
`onOpenDetails` flows through the three-dot actions menu rendered in the
turn header (only present when `onOpenDetails` or `onOpenMultiFileDiff`
is wired, and visually attached to the activity / tool-call section),
and `onOpenActivityDetails` is bound to clicking a completed tool row.

Both bindings break for a class of turns the user routinely produces:

1. A pure-text LLM response (no tool calls, no reasoning trace) renders
   no activity rows at all. The three-dot menu trigger lives next to the
   (now-empty) activity header and is either hidden or visually
   unreachable, and there is no tool row to click for cascade.
2. Cascade is conceptually **turn-level** in the user's mental model
   ("show me everything that happened to produce this reply"), not
   per-tool. Hanging it off a tool row mis-teaches what cascade is.

The parallel UX on the Telegram channel already does this correctly:
`crates/channels/src/telegram/adapter.rs:2312-2315` attaches a
`📊 详情` button and a `🔍 Cascade` button to the assistant message
itself as a turn-level inline keyboard — independent of whether tool
calls happened. PR 703 (issue 702 "missing trace buttons for pure text
replies") is the prior fix for the exact same bug shape on Telegram;
this issue is the web equivalent.

The vendor `TurnCard` already renders a per-response footer
(`web/src/vendor/craft-ui/components/chat/TurnCard.tsx:2508-2572`,
the `border-t border-border/30` action bar) that holds Copy / Markdown
on the left and BranchDropdown on the right. That footer is the right
home for two turn-level inspect buttons. PR 2054 (commit 413c2b09)
suppresses this footer in `responseOverflowMode === 'page'` mode via
`isPageFlowResponse`, and `RaraTurnCard.tsx:167` passes exactly that
mode — so any new slot must either lift that gate for the actions row
or render its own thinner actions strip in page-flow mode. The vendor
was copied into the rara source tree wholesale; the user has confirmed
it is acceptable to modify vendor files for this change.

If we do not do this, the following concrete bug appears. Reproducer:

1. Start the local stack: `just run` + (in another terminal)
   `cd web && bun run dev`. Open `http://localhost:5173`.
2. Send the assistant a question whose answer is pure text and triggers
   no tool calls — e.g. "what is 2+2?".
3. The assistant turn renders. The vendor's three-dot actions menu is
   either invisible (no activity section to anchor it next to) or only
   appears on hover of an empty header, and there is no tool row to
   click for cascade.
4. Even when activity exists, clicking a tool row to "open cascade" is
   the wrong affordance — cascade is turn-level. The user has no way
   to scope cascade to "the whole turn" rather than "this one tool".

The current state directly regresses `goal.md` signal 4 ("every action
inspectable through native eval interfaces") for the most common turn
shape (pure text).

Prior art surveyed:

- PR 2018 — landed the vendor `TurnCard` and the first `RaraTurnCard`
  adapter. Adapter left `onOpenDetails` and `onOpenActivityDetails`
  unwired.
- PR 2028 (spec `specs/issue-2023-topology-trace-cascade-buttons.spec.md`)
  — wired Trace via `onOpenDetails` (three-dot menu) and Cascade via
  `onOpenActivityDetails` (tool row click). This issue **supersedes**
  both wiring choices. The spec itself stays as historical record of
  the rejected design.
- PR 2032 / spec `specs/issue-2032-trace-cascade-hotfix.spec.md` —
  introduced `RaraTurnCardActionsMenu.tsx` and the `renderActionsMenu`
  override to work around a vendor `SimpleDropdown` bug
  (`setHighlightedId` during a child's mount render). With the
  three-dot menu retired in favor of turn-level footer buttons, both
  the override and the file lose their sole consumer and are removed.
- PR 2054 (commit 413c2b09) — added `isPageFlowResponse` gate that
  suppresses the response footer in page-flow mode. The new buttons
  must remain visible under the current rara configuration
  (`responseOverflowMode="page"`), so the implementer either lifts
  that gate just for the actions row or renders a slimmer dedicated
  actions row in page-flow mode.
- PR 8edf3c50 / spec
  `specs/issue-2031-thinking-only-turn-render.spec.md` — reroutes
  thinking-only turns to `type: 'intermediate'` so the card renders.
  Orthogonal to this change but constrains the test fixtures: the
  pure-text scenario must use `toolCalls: []` and `reasoning: ''`,
  not `reasoning: '...'`, otherwise the thinking-rerouting kicks in
  and adds an activity row.
- PR 703 / issue 702 — Telegram-side fix for the exact same bug shape
  ("missing trace buttons for pure text replies"). The web side has
  been out of parity with the Telegram side since PR 2028 landed; this
  spec brings them back in line.
- PR 1672 — gating bug on the retired chat-v2 page (buttons leaked
  onto every persisted assistant row including intermediate
  iterations). The structural mitigation — `finalSeq` threaded at
  turn granularity, single render per turn — is preserved: the new
  buttons render once per turn in `RaraTurnCard`, gated on the same
  `inspectable = turn.finalSeq !== null && !turn.inFlight` rule that
  PR 2028 introduced.
- Backend endpoints (`/api/v1/chat/sessions/{key}/trace?seq=` and
  `/api/v1/chat/sessions/{key}/cascade?seq=`) exist and are unchanged
  by this work — see `crates/extensions/backend-admin/src/chat/router.rs`.

## Decisions

- Add a new render slot to the vendor `TurnCard` —
  `renderResponseActions?: () => React.ReactNode` — rendered inside the
  response footer action bar (the `border-t border-border/30` block
  around `TurnCard.tsx:2508-2572`), on the right-hand side alongside
  `BranchDropdown`. This is the single approved vendor edit.
- The new slot must render in the rara configuration
  (`responseOverflowMode="page"`, i.e. `isPageFlowResponse === true`).
  The implementer's choice: either lift the `!isPageFlowResponse`
  gate so the footer renders in page-flow mode with the same chrome,
  or render a thinner dedicated actions row scoped to page-flow mode.
  Whichever path keeps the rest of the footer's existing behavior
  intact for non-rara consumers is acceptable.
- `RaraTurnCard.tsx` passes
  `renderResponseActions={() => <TraceButton /><CascadeButton />}`
  when `inspectable === true` (i.e. `turn.finalSeq !== null &&
  !turn.inFlight`). When `!inspectable`, the prop is left undefined
  and the slot renders nothing.
- Button labels are plain English: `Trace` and `Cascade`. No emoji,
  no localization key (consistent with how the Telegram buttons read
  on a single-user product — the Telegram side uses emoji + CJK
  because of Telegram-native UX conventions; web matches the
  surrounding `Copy` / `Markdown` plain-English actions in the same
  footer row).
- The buttons keep the existing modal wiring untouched: `Trace`
  opens `ExecutionTraceModal` with `seq = turn.finalSeq`; `Cascade`
  opens `CascadeModal` with `seq = turn.finalSeq`.
- The vendor's `onOpenDetails` and `onOpenActivityDetails` props
  stay wired on the vendor side but become unused by rara. The
  rara adapter stops passing them. Cascade is no longer reachable
  from a tool row click — that is intentional, recorded here so a
  future reviewer does not "restore" it.
- `RaraTurnCardActionsMenu.tsx` is deleted. Its sole purpose was to
  inject a "View turn details" item into the three-dot menu, which
  the new Trace button replaces. The implementer verifies no other
  consumer imports the file before deleting.
- The `renderActionsMenu` override in `RaraTurnCard.tsx` is removed.
  With no rara-side `onOpenDetails`, the vendor's three-dot trigger
  is suppressed at its own gate (`TurnCardActionsMenu.tsx` lines
  39-42), so the `SimpleDropdown` mount-render bug from issue 2032
  is no longer reachable and the workaround is no longer needed.
- Gating rule (`inspectable`) is unchanged from PR 2028: only render
  the buttons when `finalSeq !== null && !inFlight`. PR 1672's
  per-message footgun is structurally prevented by `RaraTurnCard`'s
  one-render-per-turn discipline; the gate is what keeps live frames
  and seq-less rows button-less.
- Backend is out of scope. The trace and cascade endpoints already
  exist; no contract changes here.
- `agent-spec lifecycle` cannot exercise these scenarios end-to-end
  (issue 2015 tracks the missing vitest adapter). The implementer
  runs `cd web && bun run test
  src/components/topology/__tests__/RaraTurnCard.test.tsx` directly
  as the verification signal, plus a manual smoke against the local
  stack against the pure-text reproducer. Reviewer may APPROVE on
  green vitest + manual smoke evidence in the PR body.

## Boundaries

### Allowed Changes
- **/web/src/vendor/craft-ui/components/chat/TurnCard.tsx
- **/web/src/components/topology/RaraTurnCard.tsx
- **/web/src/components/topology/RaraTurnCardActionsMenu.tsx
- **/web/src/components/topology/__tests__/RaraTurnCard.test.tsx
- **/specs/issue-2087-response-footer-trace-cascade.spec.md

### Forbidden
- **/crates/**
- **/web/src/api/**
- **/web/src/components/topology/ExecutionTraceModal.tsx
- **/web/src/components/topology/CascadeModal.tsx
- **/web/src/vendor/craft-ui/components/chat/TurnCardActionsMenu.tsx
- **/web/src/components/chat/SessionViewer.tsx
- **/web/src/i18n/**

The crates path is forbidden because the trace and cascade endpoints
already exist; backend changes are out of scope. The api path is
forbidden because `fetchExecutionTrace` and `fetchCascadeTrace` work
unchanged. The two modal files are forbidden because their public
shape (`sessionKey`, `seq`, `open`, `onOpenChange`) is unchanged —
re-shaping them is out of scope. `TurnCardActionsMenu.tsx` is
forbidden because the three-dot menu is being abandoned, not edited;
edits there would imply re-investing in the old design.
`SessionViewer.tsx` (the other vendor consumer of
`onOpenActivityDetails`) stays untouched because it serves a
different host UI. `i18n` is forbidden because the button labels are
plain English literals scoped to a single-user product, by Decision.

## Acceptance Criteria

Scenario: Pure-text turn shows both Trace and Cascade buttons in the response footer
  Test:
    Package: web
    Filter: RaraTurnCard__pure_text_turn_shows_trace_and_cascade_buttons
  Given a RaraTurnCard rendered with `finalSeq = 42`, `inFlight = false`, `text = "2 + 2 = 4"`, `reasoning = ""`, and `toolCalls = []`
  When the card is rendered
  Then a button labeled "Trace" is in the DOM
    And a button labeled "Cascade" is in the DOM

Scenario: Turn with tool calls also shows both buttons in the response footer
  Test:
    Package: web
    Filter: RaraTurnCard__tool_call_turn_shows_trace_and_cascade_buttons
  Given a RaraTurnCard rendered with `finalSeq = 42`, `inFlight = false`, `text = "done"`, and one completed tool call
  When the card is rendered
  Then a button labeled "Trace" is in the DOM
    And a button labeled "Cascade" is in the DOM

Scenario: Buttons are suppressed on in-flight turns and on turns without a known seq
  Test:
    Package: web
    Filter: RaraTurnCard__trace_and_cascade_buttons_suppressed_when_inflight_or_seq_null
  Given a RaraTurnCard rendered with `inFlight = true` and `finalSeq = null`
  When the card is rendered
  Then no button labeled "Trace" is in the DOM
    And no button labeled "Cascade" is in the DOM

Scenario: Clicking the Trace button opens the execution trace modal scoped to the turn's seq
  Test:
    Package: web
    Filter: RaraTurnCard__trace_button_opens_trace_modal
  Given a RaraTurnCard with `finalSeq = 42` for session key "sess-abc"
    And the trace API returns a known ExecutionTrace payload (mocked at the fetch layer)
  When the user clicks the "Trace" button
  Then a modal becomes visible
    And the modal shows content derived from the mocked ExecutionTrace payload
    And the trace API was called with `("sess-abc", 42)`

Scenario: Clicking the Cascade button opens the cascade modal scoped to the turn's seq
  Test:
    Package: web
    Filter: RaraTurnCard__cascade_button_opens_cascade_modal
  Given a RaraTurnCard with `finalSeq = 42` for session key "sess-abc"
    And the cascade API returns a known CascadeTrace payload (mocked at the fetch layer)
  When the user clicks the "Cascade" button
  Then a modal becomes visible
    And the modal shows content derived from the mocked CascadeTrace payload
    And the cascade API was called with `("sess-abc", 42)`

Scenario: Clicking a tool activity row no longer opens any modal
  Test:
    Package: web
    Filter: RaraTurnCard__tool_row_click_does_not_open_cascade
  Given a RaraTurnCard with `finalSeq = 42`, `inFlight = false`, and at least one completed tool activity
  When the user expands the activity section and clicks the tool row
  Then no modal becomes visible

## Out of Scope

- Backend changes. `/api/v1/chat/sessions/{key}/trace` and the cascade
  sibling already exist (`crates/extensions/backend-admin/src/chat/router.rs`).
- Reshaping `ExecutionTraceModal` or `CascadeModal`. Their public
  contract (`sessionKey`, `seq`, `open`, `onOpenChange`) stays
  identical.
- Restoring the three-dot actions menu after this change. The menu is
  intentionally retired for the rara topology surface.
- Editing `SessionViewer.tsx`, the other vendor consumer of the old
  `onOpenActivityDetails` slot. That surface is not part of rara's
  topology page.
- i18n / localization of the new labels. rara is single-user and the
  surrounding footer copy is already plain English.
- Visual redesign of the modals or the response footer beyond what is
  needed to host the two new buttons legibly. Polish is a follow-up
  if needed.
- Bringing the `agent-spec` vitest adapter online — issue 2015 owns
  that. Verification here is direct vitest + manual smoke against the
  pure-text reproducer.
