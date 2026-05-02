spec: task
name: "issue-2039-seq-counter-divergence"
inherits: project
tags: []
---

## Intent

The chat-history endpoint and the execution-trace endpoint walk the same
tape with two independently-implemented `seq` counters that disagree on
how to count `TapEntryKind::ToolResult`. When an assistant turn issues
N parallel tool calls, the tape stores one `ToolResult` entry holding N
results. The counters then diverge:

- `tap_entries_to_chat_messages`
  (`crates/extensions/backend-admin/src/chat/service.rs:1050-1078`)
  flattens the `results` array and emits one `ChatMessage` per result,
  bumping `seq` once per result inside the inner `for` loop (line 1059).
  N parallel results → seq advances by N.
- `get_execution_trace`
  (`crates/extensions/backend-admin/src/chat/service.rs:782-787`) treats
  the same entry as a single unit, bumping `seq` by exactly 1 per
  `ToolResult` TapEntry (line 783).

`Message` and `ToolCall` entries are symmetric (both bump by 1), so the
divergence comes only from `ToolResult` and is exactly N-1 per parallel
fan-out.

The frontend in PR 2037 (issue 2032) added a friendly-404 surface in
`ExecutionTraceModal` that catches the "rara_turn_id metadata" substring
on 404 and shows "Trace data is not available for this turn yet." That
is a bandaid: the seq the user clicks on is in fact valid; it just
addresses the wrong TapEntry on the trace side.

Reproducer (the bug appears today):

1. In a session, send a prompt that triggers two parallel tool calls in
   one assistant turn — e.g. `Read foo.md` and `Bash ls`. The tape
   records: `Message(user)` → `Message(assistant, tool_calls=[a,b])` or
   `ToolCall{calls=[a,b]}` → `ToolResult{results=[ra,rb]}` →
   `Message(assistant, final)`.
2. Call `GET /api/v1/chat/sessions/{key}/messages`. The `seq` of the
   trailing assistant message is, say, 5 (1 user + 1 assistant-with-calls
   + 2 tool-results + 1 final).
3. Click the "execution trace" button on that final assistant message.
   The frontend calls `GET /api/v1/chat/sessions/{key}/execution-trace?seq=5`.
4. `get_execution_trace` walks the tape, but its `ToolResult` arm bumps
   seq by 1 (not 2). So at seq=5 it has only seen entries 1+1+1+1 = 4
   of its counter; the 5th would be the **next** turn's user message.
   In a session with no further turns, `last_user_entry` is the user
   message from the failing turn — by luck this can sometimes return a
   trace, but in any session with N≥2 turns containing parallel tool
   results, the counter walks past the right user message and either
   reads the wrong `rara_turn_id` (silent wrong-trace surfacing) or
   bails with `InvalidRequest` / `NotFound`.
5. Frontend shows the friendly-404 from PR 2037, masking that the seq
   was correct and the backend mapping is wrong.

Prior-art search (run per spec-author rules):

- `gh issue list "seq counter"` — only #2032 (the precipitating
  hotfix) appears in the open set; nothing addressing the divergence.
- `gh issue list "execution-trace"` — the long history of the trace
  feature (#1598, #1608, #1611, #1613, #1826, #1827, #2032) shows
  trace storage being moved into the kernel and the buttons being
  wired through chat-v2 and topology, but no PR has unified the seq
  derivation between the two endpoints.
- `gh pr list "execution-trace seq"` — nothing.
- `git log --grep="seq" --since=60.days` — #1751/#1752 (refactor(db)
  catchups) and #2013 ("replace seq-based dedupe with arrival-time
  barrier" on the topology stream) are the only adjacent commits;
  neither touches `tap_entries_to_chat_messages` or
  `get_execution_trace`.
- `rg "fn tap_entries_to_chat_messages|fn get_execution_trace" crates/`
  — only the two definitions in `chat/service.rs` plus the router
  call site. No third party walks this tape with its own counter.

No prior art conflicts with the proposed fix. PR 2037 is the
precipitating context: it surfaces the symptom; this spec fixes the
cause.

Root cause: two independent walks of the same tape that must agree on
seq semantics. The fix is to derive both views from a single shared
helper so the counter logic exists in one place. The chat-history view
is the contract the frontend already consumes (one `ChatMessage` per
result, each with its own seq) — that side is load-bearing and stays
as is. The trace view must adopt the same per-result seq accounting.

## Decisions

- **Single source of seq truth.** Introduce one private helper in
  `chat/service.rs` (e.g. `walk_tape_with_seq`) that emits, in order,
  `(seq, &TapEntry, ResultIndex)` for every entry that contributes to
  a `ChatMessage`. `tap_entries_to_chat_messages` and
  `get_execution_trace` both consume this iterator. Neither function
  re-implements the counter.
- **Per-result seq remains the contract.** The frontend already
  receives `seq=N` per individual tool result via `/messages`. We do
  not change that. Instead, `get_execution_trace` learns to honor it.
- **Resolve to the last user-message entry at-or-before the requested
  seq.** Same intent as today; only the counter changes. The
  `rara_turn_id` lookup path stays identical.
- **Do not delete the frontend friendly-404.** PR 2037's
  `ExecutionTraceModal` defense-in-depth stays — there are still
  legitimate 404s (legacy sessions before trace storage existed). The
  backend fix means it stops triggering for "valid seq, parallel
  tool results"; it remains a real safety net for "no trace recorded".

## Boundaries

### Allowed Changes

- `**/crates/extensions/backend-admin/src/chat/service.rs`
- `**/crates/extensions/backend-admin/src/chat/**/*.rs`
- `**/specs/issue-2039-seq-counter-divergence.spec.md`
- `**/crates/channels/tests/web_session_smoke.rs`

### Forbidden

- `**/web/**`
- `**/crates/kernel/**`
- `**/crates/rara-model/**`
- `**/crates/extensions/backend-admin/src/chat/router.rs`

The router is forbidden because the route shape, query param, and error
mapping are already correct — only the service-layer walk is wrong.
The frontend is forbidden because the `seq` contract on `/messages`
does not change; `ExecutionTraceModal`'s friendly-404 stays.

`crates/channels/tests/web_session_smoke.rs` is allowed only as a 1-line
unblock for a pre-existing workspace-compile breakage from PR #2043
(`feat(kernel,sessions,web): per-session active/archived status`) that
left `SessionEntry` with a new required `status` field but missed
updating this test fixture. Without it, `cargo test --workspace` (which
the spec lifecycle gate runs to locate `tests::execution_trace_*`)
cannot compile, so the gate cannot verify our fix. The change is
mechanical — adding `status: SessionStatus::Active` — and is surfaced
explicitly in the implementer hand-back rather than silently expanded.

## Acceptance Criteria

```gherkin
Feature: seq counter agreement between chat history and execution trace

  Scenario: parallel tool results map seq correctly across endpoints
    Given a tape with one user Message carrying rara_turn_id metadata
    And one ToolCall entry with two parallel calls
    And one ToolResult entry with two results
    And one trailing assistant Message
    When tap_entries_to_chat_messages emits ChatMessages with seq values
    And get_execution_trace is called with the seq of the trailing assistant message
    Then it resolves to the rara_turn_id of the user Message that opened the turn
    Test: crates/extensions/backend-admin/src/chat/service.rs::tests::execution_trace_resolves_after_parallel_tool_results

  Scenario: single tool result behaves the same as before
    Given a tape with one user Message carrying rara_turn_id metadata
    And one ToolCall entry with one call
    And one ToolResult entry with one result
    And one trailing assistant Message
    When get_execution_trace is called with the seq of the trailing assistant message
    Then it resolves to the rara_turn_id of the user Message that opened the turn
    Test: crates/extensions/backend-admin/src/chat/service.rs::tests::execution_trace_single_tool_result_unchanged

  Scenario: requesting seq before the first user message is rejected
    Given a tape that begins with a system Message
    When get_execution_trace is called with seq 0
    Then it returns ChatError::InvalidRequest
    Test: crates/extensions/backend-admin/src/chat/service.rs::tests::execution_trace_no_user_message_rejects
```

## Out of Scope

- Changing the `/messages` seq contract.
- Removing or altering the `ExecutionTraceModal` friendly-404.
- Persisting trace lookups by something other than `rara_turn_id`
  metadata on the user TapEntry.
- Tape format changes (splitting `ToolResult` into one entry per
  result).
