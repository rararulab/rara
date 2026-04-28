spec: task
name: "issue-1973-e2e-contract"
inherits: project
tags: ["test", "docs", "app", "kernel"]
---

## Intent

rara is an HTTP-served, in-process Rust agent — its end-to-end behavior is
testable in pure Rust. Today the codebase has e2e tests (5 in
`crates/app/tests/`, 5 in `crates/kernel/tests/`) but no written contract
for what an e2e test *is* in this repo. The result: when an implementer
adds a feature touching `crates/app/src/`, `crates/kernel/src/`,
`crates/channels/src/`, etc., there is no mechanical signal telling them
"this needs an e2e", and no shared idea of what shape that e2e should take.
PR 1941 is the cautionary tale — a real-LLM e2e was added whose assertions
(`saw_anchor`, `read_file_calls >= 9`) tested the model's
instruction-following rather than rara's own code, because no written
contract distinguished "what e2e proves about rara" from "what e2e proves
about the model".

The deletion record matters. Issue 1930 (PR 1933) deleted the scripted-LLM
e2e suite (`crates/app/tests/e2e_scripted.rs`) and all `wiremock`-based
HTTP fakes. It explicitly **kept** `ScriptedLlmDriver` +
`KernelTestHarness` (`crates/kernel/src/llm/scripted.rs`,
`crates/kernel/src/testing.rs`) because kernel-internal tests use them as
*dependency injection*, not as LLM mocks. The line drawn: HTTP-level mocks
are out; an in-process scripted `LlmDriver` injected at the kernel
boundary is in. Issue 1941 (PR 1943) then added `e2e.yml` to run
real-LLM flows on `main` push only, leaving PR-time `cargo nextest run
--workspace --profile ci` (rust.yml) as the gate for everything that does
*not* call a real LLM.

This spec produces two things:

1. A written e2e contract (`docs/guides/e2e-style.md`) that defines
   the canonical shape, the lane between "kernel-injected scripted LLM"
   and "real LLM in `e2e.yml`", and the rule that wiremock / HTTP-level
   mocks are not coming back.
2. A workflow rule (one paragraph in `docs/guides/workflow.md`) that
   binds lane-2 implementer behavior: a diff that touches
   `crates/{app,kernel,channels,acp,sandbox}/src/` must add or extend a
   PR-time e2e covering the changed flow, or the implementer must state
   in the PR body which lane (1/2/3 below) makes coverage infeasible.

Reproducer for "what bug appears if we don't do this": an implementer
adds a new tool to the kernel registry, writes a unit test for the tool's
input parser, lands the PR. Two PRs later a reviewer notices the tool
fires the wrong event-bus topic during real session use. Today, neither
the implementer guide nor a checklist demands a kernel-level e2e
exercising "session receives InboundMessage → tool dispatches → expected
TapEntry written". Without the contract, this gap reproduces every PR.

Goal alignment: advances `goal.md` signal 4 ("Every action is
inspectable") — uniform e2e shape that asserts on `TurnTrace`,
`TapeService`, and event-bus side effects gives every reviewer the same
inspection vocabulary. Does not cross any "What rara is NOT" line: this
is internal testing infrastructure, not user-facing surface.

Hermes positioning: not applicable; this is a development-process
artifact specific to rara's Rust codebase.

## Decisions

### How do we expand PR-time e2e coverage without resurrecting wiremock

The decision chain (issue 1890 then 1930/PR 1933 then 1941/PR 1943)
banned two specific things: HTTP-level fakes (`wiremock`, `mockito`) and
the scripted-LLM flow-suite `e2e_scripted.rs`. It explicitly kept
`ScriptedLlmDriver` (the in-process trait impl of `LlmDriver`) and
`KernelTestHarness` as dependency injection at the kernel boundary,
because the kernel's `LlmSubsys` is an in-process trait, not an HTTP
client. The lanes available for new PR-time e2e are therefore:

1. No-LLM flows. Session routing, channel adapters, guard rejections,
   tape persistence, tool registry, event-bus topics, principal
   resolution. These are the bulk of rara's behavior and they don't need
   any LLM. Existing examples: `crates/kernel/tests/guard_integration.rs`,
   `tool_concurrency.rs`, `tool_validate.rs`, `task_report_test.rs`.
2. Kernel-DI scripted LLM. Tests that need to drive an agent loop
   inject `ScriptedLlmDriver` directly into the kernel, asserting on
   `TurnTrace` and `TapeService` output. Existing example:
   `crates/kernel/tests/anchor_checkout_e2e.rs` runs on every PR. This is
   not the deleted `crates/app/tests/e2e_scripted.rs` pattern — that one
   wired scripted LLM through the full app stack as a substitute flow
   suite; the keep-list is for narrow kernel-loop scenarios with crisp
   turn-by-turn assertions.
3. Real-LLM flows. Stay in `e2e.yml` (`main` push only), `#[ignore]`'d
   by default, never gating PRs. Per the decision in issue 1941.

`docs/guides/e2e-style.md` codifies this three-lane split. New e2e tests
default to lane 1 (no LLM); lane 2 is reserved for assertions whose only
meaningful precondition is "agent loop produced N turns of shape X" and
whose assertions are deterministic on the scripted output; lane 3 is
the existing `e2e.yml` set, not expanded by this spec.

### What canonical shape does an e2e take

Codify the existing pattern from `crates/app/tests/web_session_smoke.rs`
and `crates/kernel/tests/anchor_checkout_e2e.rs`:

- App-level: `rara_app::start_with_options()` with `StartOptions` overrides
  for paths and config; inject `InboundMessage` via the channel layer;
  assert on `TapeService` entries, `TurnTrace`, or HTTP responses.
- Kernel-level: build a `KernelTestHarness` (already in
  `crates/kernel/src/testing.rs`); drive the agent loop directly;
  assert on `TurnTrace` and event-bus topic publications.
- `#[ignore]` only when the test depends on an external resource the
  PR-time runner cannot provide (real LLM provider, `boxlite` runtime
  files — see `crates/app/tests/run_code_session.rs`).

### What changes about lane-2 implementer behavior

`docs/guides/workflow.md` step 2 (Implement) gains one paragraph: if
the diff touches `crates/{app,kernel,channels,acp,sandbox}/src/`, the
implementer must either add or extend an e2e in the corresponding
`tests/` directory exercising the changed flow, or state in the
PR body which lane (1/2/3 above) makes coverage infeasible. This is
prose guidance, not a CI gate — the existing reviewer step catches
violations.

### What does Part A actually ship in this PR

- New file `docs/guides/e2e-style.md` containing the contract.
- One paragraph appended to the "Step 2: Implement" section of
  `docs/guides/workflow.md`.
- Two example tests demonstrating lane 1 and lane 2 of the contract,
  written so their names and assertions read as documentation:
  - `crates/kernel/tests/e2e_contract_lane1_no_llm.rs` —
    no-LLM flow asserting a `TapEntry` is written when an
    `InboundMessage` is routed through the kernel via a path that
    short-circuits before the agent loop (e.g. guard-deny or a
    routing-only assertion that does not require an LLM turn).
  - `crates/kernel/tests/e2e_contract_lane2_scripted.rs` —
    scripted-LLM flow that injects a 1-turn `ScriptedLlmDriver`
    response and asserts the resulting `TurnTrace` shape.

Both must run under `cargo nextest run --workspace --profile ci` —
no `#[ignore]`. Each test binds 1:1 to a `Test:` selector below so the
acceptance check is mechanical.

### What does Part B look like

A backlog list in the GitHub issue body (not in this spec, not
implemented). One bullet per missing PR-time e2e flow, each with
"name + one-sentence outcome". The implementer of Part A is not
responsible for Part B; Part B becomes follow-up issues filed by
spec-author after this PR merges.

## Boundaries

### Allowed Changes

- New file `docs/guides/e2e-style.md`.
- One paragraph appended to the "Step 2: Implement" section of
  `docs/guides/workflow.md`.
- Two new test files: `crates/kernel/tests/e2e_contract_lane1_no_llm.rs`,
  `crates/kernel/tests/e2e_contract_lane2_scripted.rs`.
- If `crates/kernel/Cargo.toml` `[dev-dependencies]` need a missing crate
  for the new tests (e.g. `tokio-test`), add it — but do not add or
  reintroduce `wiremock`, `mockito`, or any HTTP-mock crate.
- **/docs/guides/e2e-style.md
- **/docs/guides/workflow.md
- **/crates/kernel/tests/e2e_contract_lane1_no_llm.rs
- **/crates/kernel/tests/e2e_contract_lane2_scripted.rs
- **/crates/kernel/Cargo.toml
- **/specs/issue-1973-e2e-contract.spec.md

### Forbidden

- Do NOT add `wiremock`, `mockito`, or any HTTP-fake crate to any
  `Cargo.toml`. Decision chain: issue 1930 / PR 1933.
- Do NOT modify `.github/workflows/e2e.yml` or `.github/workflows/rust.yml`.
- Do NOT modify or delete `crates/kernel/src/llm/scripted.rs` or
  `crates/kernel/src/testing.rs` — they are explicitly kept by issue 1930.
- Do NOT remove or `#[ignore]`-flag any existing e2e in
  `crates/app/tests/` or `crates/kernel/tests/`.
- Do NOT mark the two new example tests `#[ignore]` — the contract is
  that lane-1/lane-2 e2e run on every PR.
- Do NOT exceed ~200 lines of code change across the two new test files
  plus the workflow.md paragraph. The doc file `e2e-style.md` is prose
  and does not count toward the LOC budget but should stay under ~150
  lines for readability.
- Do NOT implement any flow from Part B in this PR.
- Do NOT introduce a new top-level e2e crate or test harness — reuse
  `KernelTestHarness` and `start_with_options()` exclusively.

## Completion Criteria

Scenario: Lane-1 example test runs without invoking the LLM
  Test:
    Package: rara-kernel
    Filter: e2e_contract_lane1_no_llm
  Given a KernelTestHarness configured with no LLM provider available to the agent loop
  When an InboundMessage is routed through the kernel along a path that short-circuits before the LLM is consulted
  Then a TapEntry capturing the short-circuit outcome is persisted and no agent-loop turn is recorded

Scenario: Lane-2 example test drives a scripted LLM and asserts on TurnTrace
  Test:
    Package: rara-kernel
    Filter: e2e_contract_lane2_scripted
  Given a KernelTestHarness with ScriptedLlmDriver loaded with a single deterministic turn response
  When an InboundMessage triggers the agent loop
  Then the TurnTrace contains exactly one turn whose final assistant message matches the scripted response

Scenario: Both example tests run under the PR-time CI nextest profile
  Test:
    Package: rara-kernel
    Filter: e2e_contract_lane
  Given the workspace is built with the ci nextest profile
  When cargo nextest run --workspace --profile ci is invoked
  Then both e2e_contract_lane1_no_llm and e2e_contract_lane2_scripted execute and pass without the --ignored flag

## Out of Scope

- Implementing any of the Part B backlog flows (session creation, message
  routing, guard rejection, tool happy-path, memory write/read, and
  related app-level flows). Those become follow-up issues after this PR
  merges.
- Modifying `e2e.yml` or `rust.yml`. PR-time CI already runs `cargo nextest
  run --workspace --profile ci`; the new tests are picked up automatically.
- Refactoring existing e2e tests in `crates/app/tests/` or
  `crates/kernel/tests/` to match the contract. The contract documents
  the existing canonical shape; pre-existing tests are conformant.
- Adding a CI gate that mechanically enforces "diff touches src/ → e2e
  added". The rule is reviewer-enforced prose, not a workflow check.
- Reintroducing the `e2e_scripted.rs` flow-suite under any name. The
  decision in issue 1930 stands; the two example tests in this PR are
  narrow contract demonstrations, not a flow suite.
