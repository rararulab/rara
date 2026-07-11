spec: task
name: "issue-2217-run-code-exec-timeout"
inherits: project
tags: []
---

## Intent

The `run_code` tool executes untrusted, LLM-generated code inside the
shared per-session boxlite microVM. Unlike its sibling `bash`, it builds
its `ExecRequest` **without a timeout**:
`crates/app/src/tools/run_code.rs` (around line 138) calls
`ExecRequest::builder().command(...).args(...).build()` — no `.timeout(...)`
— then holds the per-session `Arc<Mutex<Sandbox>>` lock for the whole
exec and streams stdout to completion. `bash` does the opposite
(`crates/app/src/tools/bash.rs`, ~line 243): it resolves a duration from
`params.timeout` (else the `DEFAULT_TIMEOUT_SECS` const), passes
`.timeout(timeout_dur)` so boxlite hard-kills the exec, and maps the
kill into a clean `timed_out = true` result. `ExecRequest.timeout:
Option<Duration>` already exists and is enforced by boxlite
(`crates/rara-sandbox/src/config.rs`, ~line 84) — `run_code` simply never
sets it.

Because `run_code` sets no boxlite timeout, its only backstop is the
kernel's coarse per-tool wall: `crates/kernel/src/agent/mod.rs` (~line
2932) wraps every `tool.execute()` in
`tokio::time::timeout(per_tool_timeout, ...)` where `per_tool_timeout =
tool.execution_timeout().unwrap_or(default_tool_timeout)`. `bash`'s
`ToolDef` sets `timeout_secs = 150` so its `execution_timeout()` returns
`Some(150s)`; `run_code`'s `ToolDef` sets no `timeout_secs`, so it falls
back to `default_tool_timeout` (kernel default 2 minutes). That backstop
fires by **dropping** the `run` future — which is a blunt instrument, not
a clean kill.

Reproducer for the failure mode (what breaks if we do not do this):

1. Start rara with a `sandbox:` block configured. In a session the agent
   invokes `run_code` with a non-terminating command, e.g.
   `sh -c 'while true; do :; done'` (or `sh -c 'sleep 999'`).
2. `run_code` acquires the per-session sandbox mutex and enters the exec.
   boxlite was given no timeout, so it never kills the guest process. The
   `run` future stays alive holding both the `Arc<Mutex<Sandbox>>` clone
   and the mutex guard. For up to `default_tool_timeout` (~2 minutes)
   **every other `bash` / `run_code` call in that session blocks on
   `sandbox.lock().await`** — the whole session's exec surface is stalled
   on one runaway command, with no per-call timeout the agent can use to
   bound it.
3. When the 2-minute kernel wall finally fires it drops the `run` future.
   That releases the lock and the `Arc` clone, but it does **not** kill
   the boxlite guest process — the `while true` keeps burning a guest
   vCPU inside the VM until the VM itself is destroyed at session end.
   The agent receives a generic "tool execution timed out" error with no
   `timed_out` signal it can reason about, unlike `bash`.
4. This is exactly the fragile pattern issue 1866 documents. Relying on
   future-drop for cleanup means the `run` future holds its `Arc` clone
   right up to the 2-minute wall; if the session ends inside that window
   the `SandboxCleanupHook` reaper sees `strong_count > 1` and the runaway
   VM leaks (issue 1866's race). A boxlite-enforced timeout makes
   `run_code` **return normally** — like `bash` — so the lock and the
   `Arc` clone are released at a deterministic, short bound instead of via
   cancellation, and the guest process is actually killed.

The fix mirrors `bash` exactly: resolve a duration (per-call
`params.timeout`, else a `run_code`-local default `const`), pass
`.timeout(...)` on the `ExecRequest`, and map the boxlite timeout error
into a `timed_out = true` field on `RunCodeResult`. The security /
network surface is untouched (that is issue 2216's concern); this issue
is purely about bounding exec duration.

Goal alignment: signal 1 ("the process runs for months without
intervention. Memory does not grow unboundedly, file descriptors do not
leak, internal state recovers without supervisor restarts") and the
stated 2026-Q2 "Safety and stability hardening" focus. A single runaway
`run_code` today stalls a session's entire exec surface for two minutes
and leaves a guest process burning CPU until VM teardown — a direct
stability defect. It also reinforces signal 4 ("every action is
inspectable"): a timeout becomes an explicit `timed_out` result the model
can see and act on, not a swallowed generic error. Crosses no `NOT` line
— single-user local stability hygiene, not feature-parity, not
multi-user, not a framework surface. Hermes parity: N/A — bounding a
local untrusted-code exec is internal resource discipline, not a
user-facing capability.

Prior art reviewed (mandatory search):

- `gh issue list --search "run_code timeout" / "sandbox timeout exec" /
  "bash timeout"` — no open or closed issue asks for a `run_code`
  timeout. The only structurally related open item is issue 1866
  ("handle in-flight exec at session end without leaking VM"), which this
  issue complements rather than duplicates: 1866 fixes the reaper that
  reclaims a leaked VM; this fixes the source that makes the leak likely
  (an unbounded exec holding the clone). Distinct surfaces
  (`SandboxCleanupHook` reclamation loop vs. `ExecRequest.timeout`
  resolution), no code overlap.
- `gh pr list --search "bash timeout" / "run_code timeout"` — the bash
  timeout machinery landed in PR 746 (issue 744, "spawn with process
  group, incremental read, clean kill"), PR 1115 (issue 1114, coerce
  string timeout to Duration), PR 1308 (issue 1307, accept Duration-style
  map), and the per-tool kernel granularity in PR 782 (issue 778). None
  of these touched `run_code`, which did not exist yet. No PR proposes or
  reverts a `run_code` timeout.
- `git log --all --grep "run_code"` — `run_code` was introduced by PR 1861
  (issue 1700, "expose run_code via boxlite"). Its body and diff mention
  no timeout at all: the omission is a gap, **not** a documented decision
  to leave `run_code` unbounded. There is no prior decision this issue
  reverses.
- `rg` on `crates/app/src/tools/{bash.rs,run_code.rs}` confirms the
  asymmetry: `bash` owns `DEFAULT_TIMEOUT_SECS`, a `deserialize_timeout`
  visitor for the `timeout` param, and `.timeout(timeout_dur)` on its
  `ExecRequest`; `run_code` owns none of these. `ExecRequest.timeout`
  (`crates/rara-sandbox/src/config.rs`) is already a public builder field,
  so no `rara-sandbox` surface change is needed.
- Sibling touching the same file: issue 2216
  (`specs/issue-2216-run-code-default-deny-egress.spec.md`) also edits
  `crates/app/src/tools/run_code.rs`, but on the **network policy**
  surface (`fused_network_policy`), not exec timeout. No conceptual
  overlap; flagged only so the parent can sequence the two run_code.rs
  edits to avoid a merge conflict.

## Decisions

- **Mirror `bash`: resolve a duration and set `.timeout(...)` on the
  `ExecRequest`.** In `RunCodeTool::run`, resolve
  `params.timeout.unwrap_or_else(|| Duration::from_secs(
  DEFAULT_EXEC_TIMEOUT_SECS))` and pass it to
  `ExecRequest::builder().timeout(...)`. This is the core fix — boxlite
  then hard-kills a runaway exec and `run` returns normally, releasing
  the lock and the `Arc` clone at a bounded, deterministic point.
- **`DEFAULT_EXEC_TIMEOUT_SECS` is a `run_code`-local Rust `const`, not
  YAML, and not shared with `bash`'s const.** It is a mechanism-level
  safety backstop: a deploy operator has no principled "right" value, so
  per `docs/guides/anti-patterns.md` ("mechanism-tuning constants … are
  Rust `const`") and the #1804→#1882 lineage it must not become a YAML
  knob. It is deliberately a **separate** const from `bash`'s
  `DEFAULT_TIMEOUT_SECS` (not a shared import): coupling two independent
  tools' safety backstops via one symbol is a footgun — a future change
  to bash's default would silently move run_code's, and the two tools are
  semantically different (a shell one-liner vs. an arbitrary program that
  may legitimately compile/run for a while). Value: **120 seconds**,
  matching bash's proven default rather than inventing a new number.
- **Add a per-call `timeout` field to `RunCodeParams`, with bash's exact
  coercion semantics.** `run_code`'s whole purpose is running
  LLM-generated code that can legitimately run for minutes, so the agent
  needs the same escape hatch `bash` already has. It must accept an
  integer (`120`), a stringified integer (`"120"`), a humantime duration
  (`"2m"`), and the `{"secs": N, "nanos": N}` Duration-map form that some
  LLMs emit — i.e. the identical behavior of bash's `deserialize_timeout`.
  To avoid drift, the implementer extracts bash's `deserialize_timeout`
  (and its `DurationVisitor`) into a shared module under
  `crates/app/src/tools/` (e.g. `timeout.rs`) and has **both** `bash` and
  `run_code` use it, rather than copy-pasting the visitor. Moving a
  private helper is a mechanical, behavior-preserving refactor; bash's
  existing behavior must not change.
- **Add `timed_out: bool` to `RunCodeResult`, mirroring `BashResult`.**
  Derive it the same way `bash` does: after `outcome.execution.wait()`,
  treat a wait error whose message contains "timeout" / "timed out" as
  `timed_out = true`. This turns a boxlite kill into an explicit,
  inspectable signal the model can reason about (goal signal 4) instead
  of the current generic error, and it is what "照 bash 的做法" means. The
  addition is additive to the result schema (a new boolean field);
  `exit_code` / `stdout` / `stderr` keep their current meaning.
- **Also give `run_code`'s `ToolDef` a kernel-level `timeout_secs`, set
  above the boxlite default so boxlite fires first.** Add
  `timeout_secs = 150` to the `#[tool(...)]` attribute (matching bash's
  150 over its 120s boxlite default). This keeps the kernel wall strictly
  above the boxlite kill for the default case, so in the common runaway
  path boxlite kills the exec and `run_code` returns a clean
  `timed_out = true` result rather than the kernel dropping the future.
  It is defense-in-depth parity with bash, not a replacement for the
  boxlite timeout.
- **Known inherited limitation (not fixed here): a per-call
  `timeout` larger than the kernel `timeout_secs` still hits the kernel
  wall first.** If the agent passes `timeout = "10m"` while the ToolDef
  wall is 150s, the kernel drops the future at 150s — the same latent
  tension `bash` already has. Bounding the common no-timeout runaway case
  is the win here; reconciling a user-supplied timeout that exceeds the
  kernel wall is a separate, cross-cutting concern (it would touch the
  kernel per-tool-timeout contract) and is out of scope.
- **Testable seam so the fix is falsifiable in CI without boxlite.**
  `run_code`'s real exec goes through boxlite, which CI cannot run (the
  existing `crates/app/tests/run_code_session.rs` is `#[ignore]`d for
  exactly this reason). So the fail-before/pass-after binding lives in
  pure, unit-testable seams, following the pattern issues 1866 and 2216
  used for the same crate:
  - Extract `ExecRequest` construction into a pure function
    (e.g. `build_exec_request(command, args, timeout: Option<Duration>)
    -> ExecRequest`) that resolves the default and sets `.timeout(...)`.
    A unit test reads back `request.timeout` (a public field) and asserts
    it is `Some(DEFAULT_EXEC_TIMEOUT_SECS)` when the param is absent and
    `Some(v)` when the param is `v`. This test **fails before** the change
    (today `run_code` never sets `.timeout`, so the field is `None`) and
    **passes after**.
  - A `RunCodeParams` parse test asserts the `timeout` field accepts the
    same forms bash accepts (integer, stringified integer, humantime,
    Duration-map), reusing the shared visitor.
  - The timeout→`timed_out` classification is a small pure helper
    (mapping a wait-error message to a bool) with its own unit test.
  The genuine end-to-end ("a `while true` is actually killed and the lock
  is released") stays an `#[ignore]`d real-boxlite integration test — not
  bound to a lifecycle scenario, same as 1866/2216 leave their boxlite
  behavior to `#[ignore]`d tests.
- **No `rara-sandbox` change.** `ExecRequest.timeout` is already public
  and boxlite already enforces it; the crate's minimal-surface invariant
  (`crates/rara-sandbox/AGENT.md`) stands. All changes live at the app
  boundary.

## Boundaries

### Allowed Changes
- crates/app/src/tools/run_code.rs
- **/crates/app/src/tools/run_code.rs
- crates/app/src/tools/bash.rs
- **/crates/app/src/tools/bash.rs
- crates/app/src/tools/timeout.rs
- **/crates/app/src/tools/timeout.rs
- crates/app/src/tools/mod.rs
- **/crates/app/src/tools/mod.rs
- crates/app/src/tools/AGENT.md
- **/crates/app/src/tools/AGENT.md
- crates/app/tests/run_code_session.rs
- **/crates/app/tests/run_code_session.rs
- specs/issue-2217-run-code-exec-timeout.spec.md
- **/specs/issue-2217-run-code-exec-timeout.spec.md

### Forbidden
- crates/app/src/sandbox.rs
- crates/rara-sandbox/**
- crates/kernel/**
- crates/rara-model/**
- crates/rara-fleet/**
- config.example.yaml
- web/**
- extension/**
- .github/workflows/**

## Acceptance Criteria

Scenario: run_code sets a default boxlite timeout when the caller omits one
  Test:
    Package: rara-app
    Filter: tools::run_code::tests::run_code_sets_default_exec_timeout
  Given a run_code invocation whose params carry no timeout
  When the ExecRequest for that invocation is built
  Then its timeout field is Some, equal to DEFAULT_EXEC_TIMEOUT_SECS
  And this inverts the pre-change behavior where the ExecRequest timeout was None

Scenario: run_code honors a per-call timeout when the caller supplies one
  Test:
    Package: rara-app
    Filter: tools::run_code::tests::run_code_honors_per_call_timeout
  Given a run_code invocation whose params carry an explicit timeout of five seconds
  When the ExecRequest for that invocation is built
  Then its timeout field is Some equal to five seconds
  And the default const is not used

Scenario: run_code timeout param accepts the same forms as bash
  Test:
    Package: rara-app
    Filter: tools::run_code::tests::run_code_params_accepts_timeout_forms
  Given run_code params JSON using an integer, a stringified integer, a humantime string, and a secs/nanos map for the timeout
  When each is deserialized into RunCodeParams
  Then every form parses to the expected Duration using the shared timeout visitor

Scenario: a boxlite timeout error is reported as timed_out on the result
  Test:
    Package: rara-app
    Filter: tools::run_code::tests::run_code_maps_timeout_error_to_flag
  Given a wait-error message that indicates a boxlite timeout
  When run_code classifies that error for the result
  Then the RunCodeResult timed_out flag is true
  And a non-timeout wait error classifies as timed_out false

## Out of Scope

- **Real-boxlite end-to-end verification that a `while true` is actually
  killed and the session lock released.** That behavior is real and worth
  a test, but boxlite is unavailable in CI (the existing
  `crates/app/tests/run_code_session.rs` is `#[ignore]`d). It is added as
  an `#[ignore]`d integration test, not bound to a lifecycle scenario,
  matching how issues 1866 and 2216 keep their boxlite behavior out of the
  CI-runnable binding.
- **Reconciling a per-call `timeout` that exceeds the kernel per-tool
  wall** (`timeout_secs`). The kernel would still drop the future first;
  fixing that touches the kernel per-tool-timeout contract
  (`crates/kernel/src/agent/mod.rs`) and is a separate cross-cutting
  concern. bash has the same latent limitation today.
- **The session-end VM reclaim race itself** — that is issue 1866. This
  issue reduces how often that race is hit (by bounding the exec that
  holds the clone) but does not change `SandboxCleanupHook`.
- **Any change to `run_code`'s network / egress policy** — that is issue
  2216. This issue does not touch `fused_network_policy` or the network
  surface.
- **Killing an in-flight exec on turn cancellation before the boxlite
  timeout fires.** The kernel signal pipeline already cancels the turn;
  this issue only bounds the exec's own duration.
- **Any change to `ExecRequest` / `Sandbox` signatures in
  `rara-sandbox`.** The timeout field already exists and is enforced.
