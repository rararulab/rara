# E2E Test Style — What rara's End-to-End Tests Look Like

rara is an HTTP-served, in-process Rust agent. Its end-to-end behavior is
testable in pure Rust — no separate binary, no language runtime spin-up,
and no HTTP fakes outside the one sanctioned driver-stack lane (lane 3).
This guide codifies what an e2e test *is* in this repo, which lane it
belongs in, and what it must (and must not) assert.

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

### Lane 3 — Mock-provider driver-stack e2e (runs on PRs and `main`)

Full app boot via `start_with_options()` + the **real openai driver over
HTTP** against a local in-process wiremock OpenAI fake serving scripted
SSE chat completions. This is the only lane that exercises what lane 2's
trait-level DI deliberately skips: HTTP client construction, auth
headers, URL building (`crates/kernel/src/llm/openai.rs`), and SSE
stream parsing / idle timeout / stream-close salvage
(`stream_chat_completions` + `StreamAccumulator`). Assertions are
rara-owned: `TurnTrace` shape, tape state, and post-hoc assertions on
the **captured requests** the driver actually sent (context assembly,
tool-result round-trips, `stream: true`).

The tests are `#[ignore]`'d (full-app boot is too heavy for the ordinary
`cargo nextest run --workspace` gate) and run via
`.github/workflows/e2e.yml` (`E2E (Mock Provider)`) on pull requests and
`main` pushes — deterministic, zero-cost, secret-free.

**Real-model-behavior assertions no longer run in CI at all** (#2190,
user decision: no real provider key, for cost). Tests that need a live
model (e.g. `crates/app/tests/run_code_session.rs`, real LLM + boxlite)
stay `#[ignore]`'d and are manual/local only, for humans with their own
key. If you find yourself wanting to add a real-LLM assertion from a
feature PR, stop — the assertion you are writing belongs in lane 1 or
lane 2 instead.

Anchors: `crates/app/tests/anchor_checkout_e2e.rs`, the shared fake in
`crates/app/tests/common/mock_provider.rs` (modeled on `openai/codex`
`codex-rs/core/tests/common/responses.rs`), the `e2e.yml` workflow.
Decision chain: #1930 → #1941 → #2016 → #2178 → #2190.

## Lane decision rule

> Does the assertion read meaningfully when the LLM returns a
> deterministic canned response? If yes → lane 1 or 2 (lane 1 if no LLM
> at all is needed); lane 3 only when the **wire itself** is the test
> target — driver HTTP/SSE behavior or full-app-boot integration. If the
> assertion only makes sense with a real model — e.g. "the model picks
> the read_file tool" or "the response contains an explanation" — it has
> no CI lane at all (#2190): keep it manual/local, and it is almost
> always the wrong assertion to be making in the first place.

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
cannot provide, or boots the full app:

- Full app boot via `start_with_options()` (lane 3) — too heavy for the
  ordinary workspace test gate; runs in `e2e.yml` via `-- --ignored`.
- A real LLM provider (manual/local only since #2190).
- The `boxlite` runtime files (see
  `crates/app/tests/run_code_session.rs`).

`#[ignore]` is **not** a way to silence flaky tests, slow tests, or
tests that need a temp directory. Fix the underlying cause.

## Forbidden

- `wiremock`, `mockito`, or any HTTP-fake crate **outside the lane-3
  driver-stack e2e** (`rara-app` dev-dependency, used by
  `crates/app/tests/anchor_checkout_e2e.rs` + its
  `tests/common/mock_provider.rs` helper). Everywhere else the kernel's
  LLM surface is a Rust trait — fake it at the trait, not at the wire.

  **Decision reversal, explicit (#2190):** #1930 / PR #1933 banned
  wiremock entirely because "CI is gaining a real LLM API key" made
  HTTP fakes redundant. That premise was revoked on 2026-07-05 (user
  decision: no real key in CI, for cost) — with no real key, the wire
  path (`openai.rs` HTTP + SSE parsing) has **zero** CI coverage unless
  an HTTP fake provides it. The carve-out is narrow: wiremock is
  sanctioned ONLY where the wire is the test target; kernel/channels
  tests keep the trait-DI rule. Decision chain:
  #1930 → #1941 → #2016 → #2178 → #2190.
- Resurrecting `crates/app/tests/e2e_scripted.rs` or any equivalent
  flow-suite that wires `ScriptedLlmDriver` through the full app stack.
  The keep-list for `ScriptedLlmDriver` is narrow kernel-loop scenarios,
  not flow suites. Decision chain: issue #1930 / PR #1933 (unchanged by
  #2190).
- New top-level e2e crates or test harnesses. Reuse `KernelTestHarness`
  (`TestKernelBuilder`) and `start_with_options()` exclusively.
- Asserting on real-model behavior in any CI test (the PR #1941
  pattern). Since #2190 no CI job talks to a real model; such an
  assertion is deterministic-canned-response-meaningless and almost
  certainly should not be added at all.

## Pairing with workflow.md

`docs/guides/workflow.md` step 2 codifies the implementer-side rule:
when a diff touches `crates/{app,kernel,channels,acp,sandbox}/src/`,
the implementer adds or extends a PR-time e2e in this style, or states
in the PR body which lane (1/2/3) makes coverage infeasible. This guide
is the contract that rule points to.
