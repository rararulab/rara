# E2E Test Style — What rara's End-to-End Tests Look Like

rara is an HTTP-served, in-process Rust agent. Its end-to-end behavior is
testable in pure Rust — no separate binary, no HTTP fakes, no language
runtime spin-up. This guide codifies what an e2e test *is* in this repo,
which lane it belongs in, and what it must (and must not) assert.

If you are touching `crates/{app,kernel,channels,acp,sandbox}/src/`, the
diff almost certainly needs an e2e in this style. Read the lanes first;
skip to the templates if you already know which lane you are in.

## The three lanes

Every e2e in this repo lives in exactly one of these three lanes. Pick
the lane based on what your assertion is actually asserting on.

### Lane 1 — No-LLM flows (default; runs on every PR)

The bulk of rara's behavior — session routing, channel adapters, guard
rejections, tape persistence, tool registry, event-bus topics, principal
resolution, scheduler, notification bus — does not require an LLM. These
tests exercise rara's own code along a path that short-circuits before
any LLM call. They run unconditionally under
`cargo nextest run --workspace --profile ci`.

Anchors:

- `crates/kernel/tests/guard_integration.rs` — guard pipeline rejects
  inbound calls before the agent loop ever sees them.
- `crates/kernel/tests/tool_concurrency.rs`,
  `crates/kernel/tests/tool_validate.rs` — tool registry and validation.
- `crates/kernel/tests/task_report_test.rs` — TaskReport publishing,
  subscription matching, and silent-append delivery via `TapeService`
  with no LLM involvement.
- `crates/kernel/tests/e2e_contract_lane1_no_llm.rs` — minimal contract
  example: write a tape entry through a running test kernel, assert
  it's persisted, assert no agent turn was triggered.

### Lane 2 — Kernel-DI scripted LLM (runs on every PR)

When the test's only meaningful precondition is "agent loop produced N
turns of shape X" and the assertion is deterministic on what the LLM
returned, inject `ScriptedLlmDriver` at the kernel boundary via
`TestKernelBuilder`. The scripted driver is in-process dependency
injection, not an LLM mock or HTTP fake — the kernel's `LlmSubsys` is
already a Rust trait, so the test simply hands the kernel a different
`Arc<dyn LlmDriver>`.

Anchors:

- `crates/kernel/tests/anchor_checkout_e2e.rs` — narrow kernel-loop
  scenarios with crisp turn-by-turn assertions on `TurnTrace` and
  `TapeService`.
- `crates/channels/tests/web_session_smoke.rs::session_ws_prompt_reaches_kernel`
  — channel adapter wired up to a `TestKernelBuilder`-built kernel,
  asserts the kernel records exactly one turn whose preview matches the
  scripted response.
- `crates/kernel/tests/e2e_contract_lane2_scripted.rs` — minimal
  contract example: one scripted turn, assert `TurnTrace.iterations`
  has length one and the preview matches.

### Lane 3 — Real LLM flows (runs on `main` push only, never on PRs)

Anything whose assertion depends on a real model's output (instruction
following, tool selection, reasoning quality) is `#[ignore]`'d by default
and runs only via `.github/workflows/e2e.yml` on push to `main`. This
lane is **not** expanded by ordinary feature work. If you find yourself
wanting to add a real-LLM test from a feature PR, stop — the assertion
you are writing probably belongs in lane 1 or lane 2 instead.

Anchors: `crates/app/tests/run_code_session.rs` (real LLM + boxlite
runtime), the `e2e.yml` workflow, the decision in issue #1941 / PR #1943.

## Lane decision rule

> Does the assertion read meaningfully when the LLM returns a
> deterministic canned response? If yes → lane 1 or 2 (lane 1 if no LLM
> at all is needed). If the assertion only makes sense with a real
> model — e.g. "the model picks the read_file tool" or "the response
> contains an explanation" — that's lane 3, and almost always the wrong
> assertion to be making in a feature PR.

PR #1941 is the cautionary tale: a real-LLM e2e was added whose
assertions (`saw_anchor`, `read_file_calls >= 9`) tested the model's
instruction-following rather than rara's own code. Lane 1 / lane 2
assertions never have that ambiguity — they assert on tape state,
`TurnTrace` shape, event-bus topics, or guard verdicts, which are all
rara-owned outputs.

## Canonical shape

### App-level e2e (`crates/app/tests/`)

Boot the app via `rara_app::start_with_options()` with `StartOptions`
overriding paths and config. Inject inbound traffic through the channel
layer (`WebAdapter`, etc.). Assert on `TapeService` entries, `TurnTrace`,
or HTTP responses. Anchors: `crates/app/tests/web_session_smoke.rs`,
`crates/app/tests/web_buffer_e2e.rs`.

### Kernel-level e2e (`crates/kernel/tests/`)

Build a test kernel via `TestKernelBuilder::new(tmp.path())...build().await`
(see `crates/kernel/src/testing.rs`). Drive the agent loop directly via
`tk.handle.submit_message(..)`, `tk.handle.ingest_user_message(..)`, or
write directly to `tk.handle.tape()` for the no-LLM lane. Assert on
`tk.handle.get_process_turns(session_key)` (returns
`Vec<TurnTrace>`), `tk.handle.tape().entries(..)`, or the notification
bus.

### When `#[ignore]` is allowed

Only when the test depends on an external resource the PR-time runner
cannot provide:

- A real LLM provider (lane 3).
- The `boxlite` runtime files (see
  `crates/app/tests/run_code_session.rs`).

`#[ignore]` is **not** a way to silence flaky tests, slow tests, or
tests that need a temp directory. Fix the underlying cause.

## Forbidden

- `wiremock`, `mockito`, or any HTTP-fake crate. The kernel's LLM
  surface is a Rust trait — fake it at the trait, not at the wire.
  Decision chain: issue #1930 / PR #1933.
- Resurrecting `crates/app/tests/e2e_scripted.rs` or any equivalent
  flow-suite that wires `ScriptedLlmDriver` through the full app stack.
  The keep-list for `ScriptedLlmDriver` is narrow kernel-loop scenarios,
  not flow suites. Decision chain: issue #1930 / PR #1933.
- New top-level e2e crates or test harnesses. Reuse `KernelTestHarness`
  (`TestKernelBuilder`) and `start_with_options()` exclusively.
- Asserting on real-model behavior in a PR-time test (the PR #1941
  pattern). If your assertion only makes sense for a real model, the
  test belongs in `e2e.yml` and almost certainly should not be added at
  all.

## Pairing with workflow.md

`docs/guides/workflow.md` step 2 codifies the implementer-side rule:
when a diff touches `crates/{app,kernel,channels,acp,sandbox}/src/`,
the implementer adds or extends a PR-time e2e in this style, or states
in the PR body which lane (1/2/3) makes coverage infeasible. This guide
is the contract that rule points to.
