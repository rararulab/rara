spec: task
name: "issue-1979-suppress-whitespace-assistant-tape"
inherits: project
tags: ["kernel", "memory", "tape"]
---

## Intent

When a reasoning-capable model (MiniMax-M2, gpt-5.4-thinking, etc.) routes
all of its output tokens to `reasoning_content` and emits only a stray
whitespace token (`"\n"`, `"\n\n"`, `"\n\n让我查一下…\n"`-with-mostly-whitespace)
as the visible `content`, the kernel still writes that turn to tape as
`{role: "assistant", content: "\n"}`. The UI loads the session, sees an
assistant message with whitespace-only content, and renders an empty
bubble — the user reads "session 坏了" and reports the agent as broken.

Concrete evidence on the running remote (raratekiAir, verified 2026-04-28):

- Tape file
  `/Users/rara/Library/Application Support/rara/memory/tapes/315bad61f8c34b137221bd6ec086597c__d6e905d9-fd62-41ca-8918-97b37276f534.jsonl`
- Affected entries:
  - id 9: `content: "\n"`, 55 completion tokens all in `reasoning_content`
  - id 38: `content: "\n\n让我查一下…\n"` (mostly whitespace + truncated)
  - id 42: `content: "\n"`
- User-reported turn id: `27df73bb-9656-4e23-bbb9-e724e37162c7`

Reproducer for "what bug appears if we don't do this":

1. Open a session against a reasoning model that splits output between
   `reasoning_content` and `content` (any MiniMax-M2 deployment, or
   gpt-5.4-thinking on a turn that calls a tool after long internal
   reasoning).
2. Trigger a turn that ends an iteration with a tool call, where the
   visible-content stream finalizes as a single newline / whitespace-only
   string (the driver's stream-close salvage from PR 1639 succeeded in
   producing *some* text, but it was whitespace).
3. Inspect the tape: a `Message` row with `role=assistant` and
   `content="\n"` appears between the previous user turn and the
   `ToolCall` row.
4. Reload the session in the web UI: the affected turn renders an empty
   assistant bubble (web's `pi-chat-messages` filter from PR 1727 is
   render-side only and does not propagate back into the tape).
5. The next turn's context rebuild feeds the whitespace `assistant`
   message back into the LLM, so model self-context now contains a row
   it would never have written deliberately.

Why the existing protection does not catch this: PR 1633
(`d0dec0a4 feat(kernel): reject empty turns, route failures to error bus`)
gates `accumulated_text.trim().is_empty()` only on the **terminal**
`!has_tool_calls` branch at `crates/kernel/src/agent/mod.rs:2175`. The
**intermediate-iteration write** at `crates/kernel/src/agent/mod.rs:2425-2458`
— added by PR 608 (`0cb95291 fix(kernel): persist intermediate assistant
messages to tape for cascade tick detection`) so `build_cascade` can see
tick boundaries between iterations — has no trim check and runs
unconditionally before the tool-call write. That is the line that
produces the polluted rows above.

Goal alignment: signal 4 ("every action is inspectable") — a tape row
that says `content="\n"` while the model actually emitted 55 reasoning
tokens is a lie about what happened, and breaks every downstream eval
trace, replay, and cascade view that reads the tape as ground truth.
Signal 1 ("the process runs for months without intervention") — the
whole point of the persistence layer is that it survives untouched; the
user reporting "session is broken" is exactly the human-intervention
signal we are trying to drive to zero. Does not cross any "What rara
is NOT" line: this is internal correctness of rara's own memory layer.

Hermes positioning: not applicable — Hermes does not expose its tape
format and we have an engineering reason regardless (the cascade tick
detection rationale from PR 608 still has to hold after the fix).

Prior art search summary:

- `gh issue list --search "reasoning_content tape empty assistant" --state all` —
  surfaced issue 1627 (MiniMax-M2 empty-content driver bug), issue 1630
  (epic), issue 1633 (kernel rejection of empty turns); issue 1633
  explicitly said "Do not write empty assistant records to tape" but only
  delivered terminal-turn protection.
- `gh pr list --search "reasoning_content" --state all` — surfaced PR 1639
  (driver salvage), PR 1641 (kernel rejection, the merge of issue 1633),
  PR 1447 / 1452 / 1453 (reasoning_content persistence), PR 608
  (introduced the unconditional intermediate-write at line 2425 that is
  the root of this bug).
- `git log --grep reasoning_content` since 180 days — confirms the same
  set; no commit has revisited the intermediate-write since PR 608 /
  PR 1641.
- `git log --since=60.days -- crates/kernel/src/agent/mod.rs` — no recent
  edit removed a trim/empty check from this area; the gap has been
  present continuously since issue 1633 shipped.

The fix is **additive** (gate one existing write site), not a reversal
of any prior decision. PR 608's cascade-tick rationale must still hold:
when the iteration has tool calls AND `accumulated_text.trim().is_empty()`,
suppressing the assistant Message row would lose the cascade tick
boundary. The contract below preserves that boundary by writing the row
only when there is non-whitespace content OR non-empty `reasoning_content`
to record (the row carries real signal in the latter case — the tick
boundary survives, and the UI/cascade can still see "the model thought
here, then called a tool").

## Decisions

1. The gate lives in `crates/kernel/src/agent/mod.rs` at the
   intermediate-iteration write site (currently lines 2425–2458). No new
   crate, no new module, no trait reshuffle.
2. Suppression rule: skip the `append_message` call when
   `accumulated_text.trim().is_empty() && accumulated_reasoning.is_empty()`.
   When `accumulated_text` is whitespace but `accumulated_reasoning` is
   non-empty, persist the row but with `content` set to the empty string
   `""` (canonical) and `reasoning_content` populated — preserves cascade
   tick + carries real signal, and stops the UI from rendering "an
   assistant said \n".
3. Same rule on the laziness-nudge intermediate write
   (`crates/kernel/src/agent/mod.rs:2041-2057`) and the
   continuation-pending intermediate write (lines 2131–2147). Both
   currently use `&accumulated_text` directly with no trim guard.
4. The terminal-turn rejection at line 2175 stays as-is — it already
   handles the `!has_tool_calls` case correctly via `TurnError`. We do
   not move it; we only close the intermediate-write gap.
5. Canonicalization is `accumulated_text = accumulated_text.trim().to_string()`
   *only* when persisting (not on the in-memory variable used for the
   downstream stream / cascade), so tool-call branches still see exactly
   what the model emitted.

## Boundaries

### Allowed Changes

- `crates/kernel/src/agent/mod.rs`: gate the three intermediate-write
  sites identified above (line 2041, 2131, 2425).
- `crates/kernel/src/agent/mod.rs` test module (or a new sibling
  `agent/tape_persist_tests.rs` if the existing module is too crowded):
  add unit tests covering the four scenarios in Acceptance Criteria.
- One-line update to `crates/kernel/src/agent/turn_error.rs` only if a
  new `TurnFailureKind` variant is needed — likely not; the intermediate
  case is silent suppression, not a `TurnError`.
- `crates/kernel/tests/whitespace_intermediate_tape_e2e.rs` (new):
  scripted-LLM lane-2 e2e test required by `docs/guides/workflow.md` for
  any `crates/kernel/src/` change. Drives one agent turn whose iteration
  0 has whitespace content + non-empty reasoning + a tool call, then
  asserts the tape contains no whitespace-only assistant Message rows
  and that the cascade-tick boundary is preserved by the ToolCall row.
- `crates/kernel/src/llm/scripted.rs`: extend
  `ScriptedLlmDriver::stream` to emit `StreamDelta::ReasoningDelta` from
  `CompletionResponse.reasoning_content`. Without this, the e2e above
  cannot reproduce the reasoning-model shape that produced the bug —
  the driver previously dropped reasoning during streaming. The change
  is additive: existing callers that leave `reasoning_content = None`
  see no behavioral change.
- `specs/issue-1979-suppress-whitespace-assistant-tape.spec.md` (this
  file): present in the change set because it ships in the same PR.
- **/crates/kernel/src/agent/mod.rs
- **/crates/kernel/src/llm/scripted.rs
- **/crates/kernel/tests/whitespace_intermediate_tape_e2e.rs
- **/specs/issue-1979-suppress-whitespace-assistant-tape.spec.md

### Forbidden

- Do not change `crates/kernel/src/memory/tape/*` — the tape store stays
  agnostic about what the agent decides to write; this is an agent-loop
  policy fix.
- Do not touch `crates/kernel/src/llm/openai.rs` or `llm/stream.rs` —
  driver-level salvage already shipped in PR 1639 and is not the layer
  with the gap.
- Do not touch the web filter (`web/src/pages/__tests__/pi-chat-messages.test.ts`
  and the production code it covers) — render-side filtering stays as
  defense-in-depth, but this PR fixes the source so the filter never
  has to fire.
- Do not relax the terminal-turn rejection at line 2175 in either
  direction (do not loosen the trim check, do not extend it to fire on
  intermediate iterations — those have tool calls, which is real
  signal even when text is empty).
- Do not change the cascade-tick boundary semantics: a multi-iteration
  turn that previously produced N ticks must still produce N ticks
  after this change.

## Acceptance Criteria

Scenario: whitespace-only content with no reasoning is dropped from tape
  Given an agent iteration where the LLM driver finalizes
    `accumulated_text = "\n"`, `accumulated_reasoning = ""`,
    and emits one valid tool call
  When the agent loop reaches the intermediate-message-persist block
    (`crates/kernel/src/agent/mod.rs` line 2425 today)
  Then no `Message` row is appended to the tape for this iteration
    (the subsequent `ToolCall` row is still appended)
  And the cascade-tick count for the resulting turn equals the same
    turn run with `accumulated_text = "real text"` minus zero
    (i.e. tick boundary is preserved by the tool-call row alone)
  Test: `crates/kernel/src/agent/mod.rs` /
    `intermediate_write_drops_whitespace_only_message`

Scenario: whitespace content with non-empty reasoning persists with empty content
  Given an agent iteration where the LLM driver finalizes
    `accumulated_text = "\n\n"`, `accumulated_reasoning = "some 55-token chain of thought"`,
    and emits one valid tool call
  When the agent loop reaches the intermediate-message-persist block
  Then a `Message` row IS appended with `content == ""` (canonical empty,
    not `"\n\n"`) and `reasoning_content == "some 55-token chain of thought"`
  And reading that row back from tape yields `content == ""`
  Test: `crates/kernel/src/agent/mod.rs` /
    `intermediate_write_canonicalizes_whitespace_content_when_reasoning_present`

Scenario: laziness-nudge intermediate write respects the same gate
  Given the ack_detector laziness path fires
    (`crates/kernel/src/agent/mod.rs` line 2030 onward)
  And `accumulated_text.trim().is_empty() && accumulated_reasoning.is_empty()`
  When the laziness branch persists its intermediate assistant message
  Then that `Message` row is suppressed
  And the subsequent nudge `user` row is still appended (the nudge is
    semantic, not whitespace)
  Test: `crates/kernel/src/agent/mod.rs` /
    `laziness_nudge_suppresses_whitespace_intermediate_message`

Scenario: terminal-turn rejection at line 2175 is unchanged
  Given an agent iteration where `!has_tool_calls`
    and `accumulated_text.trim().is_empty()`
  When the agent loop reaches the terminal block
  Then a `TurnError` with `failure_kind = EmptyTurn` is published on the
    event bus, exactly as before
  And `KernelError::AgentExecution` is returned
  Test: `crates/kernel/src/agent/mod.rs` /
    `terminal_empty_turn_still_emits_turn_error`
    (regression guard — exists or is added; must keep passing)

Scenario: real assistant content is unchanged
  Given an iteration with `accumulated_text = "Here's the answer: 42"`
    and any `reasoning_content` value
  When the agent loop persists the message
  Then the tape row has `content == "Here's the answer: 42"` byte-for-byte
    (no trim, no canonicalization on non-whitespace content)
  Test: `crates/kernel/src/agent/mod.rs` /
    `non_whitespace_content_persists_byte_for_byte`

## Constraints

- All comments and identifiers in new code must be English (project
  rule).
- New tests use the existing in-process tape fake / `TapeService` test
  harness already used elsewhere in `crates/kernel/src/agent/`; do not
  introduce `testcontainers` or DB fixtures for this — it is an
  agent-loop unit test, not an integration test.
- No new YAML config keys. The trim-and-suppress rule is a mechanism
  constant (anti-pattern doc, "mechanism vs config"); there is no
  deployment-relevant reason to make it tunable.
- Do not amend or reword the cascade-tick comment block at line 2425;
  add a sibling comment explaining the suppression rule next to the new
  gate.
