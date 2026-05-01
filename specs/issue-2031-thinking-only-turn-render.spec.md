spec: task
name: "issue-2031-thinking-only-turn-render"
inherits: project
tags: [bug, ui, web]
---

## Intent

The topology timeline renders assistant turns through the vendor
`TurnCard` (`web/src/vendor/craft-ui/components/chat/TurnCard.tsx`,
landed via PR 2018), bridged by `RaraTurnCard.tsx`. When a turn has
reasoning ("thinking") content but no final assistant text and no tool
calls, the vendor returns `null` and the turn vanishes from the UI.

The vendor's `hasNoMeaningfulWork` check
(`TurnCard.tsx:2884-2898`) walks the activities array and asks "is any
activity meaningful?". Its branches:

- `type: 'tool'` → meaningful unless `status === 'error'`.
- `type: 'intermediate'` → meaningful when `content` is non-empty.
- `type: 'plan'` → always meaningful.
- everything else (including `type: 'thinking'`) → falls through the
  trailing `return true` ("consider as no meaningful work").

Combined with `!response`, a turn that contains only a
`type: 'thinking'` activity short-circuits to `return null` at line
2873/2897 and the entire turn card is suppressed. The bridge in
`RaraTurnCard.tsx:66-74` is what produces the `type: 'thinking'`
activity from `turn.reasoning`, and `RaraTurnCard.tsx:98-105` is what
sets `response = undefined` when `turn.text.length === 0`.

If we do not do this, the following concrete bug appears. Reproducer:

1. Run the frontend against the remote backend
   (`VITE_API_URL=http://10.0.0.183:25555 bun run dev`), open the
   topology page for any session that has a thinking-only assistant
   turn in its tape (a turn with `reasoning_content` set but `content`
   empty and no `tool_calls` — produced when a thinking model emits
   only chain-of-thought before being interrupted, hitting
   max-tokens, or returning empty content alongside non-empty
   reasoning, exactly the failure mode that PR 1622 / PR 1627 / issue
   1979 patched on the kernel write side).
2. Or, in vitest: render `RaraTurnCard` with a `TurnCardData` whose
   `reasoning = "let me think about this"`, `text = ""`,
   `toolCalls = []`, `inFlight = false`.
3. Observed bad outcome: the rendered output is empty — `container`
   contains no `data-turn-id` element, no thinking pill, no card
   chrome. The user sees a gap in the timeline where reasoning
   happened, with no way to tell whether the turn was lost, the
   model errored, or rara silently swallowed work. There is no path
   to the execution trace because there is no card to click.

This crosses `goal.md` signal 4 ("every action is inspectable …
no 'I don't know why it did that'"). The reasoning trace exists in
the tape and in the persisted `Message.reasoning_content`; the UI
silently drops it on the floor. It also wastes the work that PR 1622
went to the trouble of doing (persisting `reasoning_content`) and
that PR 1979 / PR 1979 follow-ups did to clean up adjacent empty-row
suppression — those fixes assume the reasoning will surface
somewhere downstream.

Why prior art does not already handle this:

- PR 1979 (`f66d9e48 fix(kernel): suppress whitespace-only assistant
  tape entries`) is the **inverse**: it suppresses tape rows whose
  content is `""` AND whose reasoning is also empty. A
  thinking-only turn with non-empty reasoning is explicitly **kept**
  by PR 1979 — the suppression only fires when both fields are empty.
  So the tape side is correct; the bug is purely in the
  `RaraTurnCard` → vendor adapter layer.
- PR 1622 / PR 1627 (`fix(kernel): persist reasoning_content in
  Message for thinking mode`) ensured reasoning round-trips through
  the tape. They did not touch any frontend.
- PR 2018 (vendor TurnCard swap, `0c538f01`) introduced the bridge
  `RaraTurnCard.tsx` and self-flagged this exact issue as a known P2
  follow-up in the implementation summary. No subsequent commit has
  addressed it.

## Decisions

1. **Fix lives in the adapter, not the vendor.** `TurnCard.tsx` is
   `// @ts-nocheck`-ed vendored code from craft-agents-oss; we
   re-pull it. Patching `hasNoMeaningfulWork` would be erased on the
   next vendor refresh. The adapter is rara-owned and is the only
   place the bridge between rara turn shape and vendor activity shape
   is encoded — it is also where the bug originates.

2. **Promote thinking-only activities to `type: 'intermediate'` in
   the adapter.** When the turn has non-empty `reasoning` AND empty
   `text` AND zero `toolCalls`, the adapter emits the activity with
   `type: 'intermediate'` (carrying the same `content` and `status`),
   instead of `type: 'thinking'`. The vendor's existing
   `hasNoMeaningfulWork` branch
   (`if (a.type === 'intermediate') return !a.content?.trim()`)
   then evaluates non-empty content as meaningful and the turn
   renders.

   Rejected alternative: synthesize a placeholder `response` with
   `text: '(thinking only)'`. This pollutes the response slot with
   fabricated user-facing text and breaks the "what the model said"
   read of the response field. The `intermediate` reroute is
   structurally honest — reasoning IS an intermediate signal, and
   the vendor renders intermediate activities as a content row.

   Rejected alternative: always emit `type: 'intermediate'` for all
   reasoning. This loses the existing distinct visual treatment for
   normal turns where reasoning accompanies a final response. The
   reroute is gated on the thinking-only condition.

3. **No new BDD scenarios for the "reasoning + text" case.** That
   case already renders correctly today and is covered implicitly by
   the existing `RaraTurnCard.test.tsx` cases. The new scenarios
   target only the bug.

4. **Test selectors bind to `data-turn-id`.** The vendor card sets a
   `data-turn-id={turnId}` attribute on its outer wrapper (used by
   existing tests). The new scenarios assert presence/absence of that
   selector.

## Boundaries

### Allowed Changes

- **/web/src/components/topology/RaraTurnCard.tsx
- **/web/src/components/topology/__tests__/RaraTurnCard.test.tsx
- **/specs/issue-2031-thinking-only-turn-render.spec.md

### Forbidden

- web/src/vendor/**
- crates/**
- web/src/components/topology/TurnCard.tsx

## Acceptance Criteria

```gherkin
Feature: Thinking-only assistant turns are visible in the topology timeline

  Scenario: thinking-only turn renders a card
    Given a TurnCardData with reasoning="let me think about this"
      and text=""
      and toolCalls=[]
      and inFlight=false
    When RaraTurnCard renders the turn
    Then the rendered output contains an element with data-turn-id matching the turn id
    And the reasoning content is present in the DOM

    Test: web/src/components/topology/__tests__/RaraTurnCard.test.tsx
    Filter: thinking_only_turn_renders_card

  Scenario: live thinking-only turn renders a card while streaming
    Given a TurnCardData with reasoning="working it out"
      and text=""
      and toolCalls=[]
      and inFlight=true
    When RaraTurnCard renders the turn
    Then the rendered output contains an element with data-turn-id matching the turn id

    Test: web/src/components/topology/__tests__/RaraTurnCard.test.tsx
    Filter: thinking_only_live_turn_renders_card

  Scenario: turn with reasoning plus final text still renders both
    Given a TurnCardData with reasoning="step 1: ..."
      and text="here is the answer"
      and toolCalls=[]
      and inFlight=false
    When RaraTurnCard renders the turn
    Then the rendered output contains an element with data-turn-id matching the turn id
    And the final assistant text is present in the DOM
    And the reasoning content is present in the DOM

    Test: web/src/components/topology/__tests__/RaraTurnCard.test.tsx
    Filter: turn_with_reasoning_and_text_renders_both

  Scenario: empty turn with no reasoning, text, or tools is still suppressed
    Given a TurnCardData with reasoning=""
      and text=""
      and toolCalls=[]
      and inFlight=false
    When RaraTurnCard renders the turn
    Then no element with data-turn-id matching the turn id is present in the DOM

    Test: web/src/components/topology/__tests__/RaraTurnCard.test.tsx
    Filter: empty_turn_remains_suppressed
```

## Out of Scope

- Visual polish of the thinking-only card (spacing, iconography). The
  fix is "render something inspectable"; styling iteration is a
  follow-up if the user finds the default `intermediate` rendering
  inadequate.
- Backend or tape-side changes. Reasoning persistence is already
  correct (PR 1622 / PR 1627).
- The other PR-2018 P2 follow-up about `SpawnMarker` living outside
  the vendor card. Separate concern, separate spec if it ships.
