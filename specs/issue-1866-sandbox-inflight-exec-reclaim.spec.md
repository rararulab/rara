spec: task
name: "issue-1866-sandbox-inflight-exec-reclaim"
inherits: project
tags: []
---

## Intent

`run_code` reuses one boxlite microVM per session, keyed in a
`SandboxMap` (`Arc<DashMap<SessionKey, Arc<Mutex<Sandbox>>>>`) and destroyed
by `SandboxCleanupHook::on_session_end`
(`crates/app/src/tools/run_code.rs`, around line 218). `Sandbox::destroy`
consumes `self`, so the hook removes the map entry and — inside a detached
`tokio::spawn` — tries `Arc::try_unwrap` to reclaim ownership. When an
`exec` is still in flight on another task, that task holds a clone of the
`Arc` (see `RunCodeTool::run`, which binds `let sandbox = …` and
`let guard = sandbox.lock().await` for the whole exec), so the strong count
is still greater than one. `try_unwrap` fails and the reaper immediately
`return`s. The map entry is already gone, so when the in-flight task later
drops its clone, nothing is watching — the microVM is never `destroy`ed and
stays registered in the boxlite runtime, holding host memory and file
descriptors until the rara process exits. The code comments this leak and
tags it "issue 1866".

Reproducer for the failure mode:

1. Start rara with a `sandbox:` block configured. In a session, the agent
   invokes `run_code` to launch a long command (e.g. `sh -c 'sleep 30'`).
   That exec is now mid-flight, holding the per-session
   `Arc<Mutex<Sandbox>>` clone and the mutex guard.
2. While the exec is in flight, the session ends (user closes it, or the
   turn is cancelled). The kernel fires `on_session_end`
   (`crates/kernel/src/lifecycle.rs::fire_session_end`, 5s per-hook
   timeout). The hook removes the map entry and spawns the reaper;
   `Arc::try_unwrap` fails because `strong_count > 1`; the reaper logs
   "leaking VM until process exit" and returns.
3. Moments later the in-flight `run` future is dropped by turn
   cancellation, releasing its clone — but the reaper already gave up.
   The boxlite microVM is never reclaimed.
4. Observed on a long-running instance: repeat across many
   session-with-in-flight-exec teardowns → zombie microVMs accumulate,
   host memory and fd usage climb monotonically, and nothing short of a
   process restart reclaims them.

The fix replaces the single-shot `try_unwrap` with a **bounded-retry
reclamation** inside the same detached task: the reaper keeps the `Arc`,
waits for the last outstanding clone to drop (which turn cancellation
guarantees for the timing window this issue describes), then takes
ownership and calls `Sandbox::destroy`. Reclamation stays fire-and-forget
so it never blocks the 5s lifecycle-hook pipeline.

Prior art reviewed (mandatory search):

- Issues / PRs: `gh issue list/pr list --search "sandbox"` and
  `--search "1866"` — no open PR references issue 1866; it is still open,
  authored alongside the run_code landing. The sandbox subsystem lands in
  PR 1840 (`rara-sandbox` crate), PR 1844 (runtime staging), PR 1861
  (`run_code` tool + the cleanup hook that carries this leak), PR 1946
  (FS-boundary + network fusion). None of them reclaim an in-flight VM at
  session end — this is a gap in PR 1861, not a decision being reversed.
- `git log --all --grep 1866` returns only PR 1861 (the commit that
  introduced the hook and the "Tracked in #1866" comment).
- Closest sibling: issue 2097 / PR (closed) — boxlite runtime-dir GC. Same
  leak *class* (resource accumulation on a long-running install, goal
  signal 1) but a different surface (on-disk staging dirs vs. in-memory
  VM handles) and a different owner (`rara-cli setup boxlite` vs. the
  session lifecycle hook). No overlap in code paths; the lesson carried
  over is only "reclaim on the boring, already-present trigger."
- `rg` in `crates/rara-sandbox` shows `Sandbox::destroy(self)` is the only
  teardown path and `crates/rara-sandbox/AGENT.md` is emphatic about
  keeping the crate's public surface minimal (no `SandboxBackend` trait,
  no mock backend, no boxlite re-exports). This spec honors that: the fix
  lives entirely at the app boundary and does not widen `rara-sandbox`.

Goal alignment: signal 1 ("the process runs for months without
intervention. Memory does not grow unboundedly, file descriptors do not
leak"). A leaked microVM per session-with-in-flight-exec is a direct
violation of that signal on a long-running instance. Crosses no `NOT`
line — this is single-user local stability hygiene, not feature-parity,
not multi-user, not a framework surface. Hermes parity is N/A: this is
internal resource discipline, not a user-facing capability.

## Decisions

- **Root cause is the single-shot `try_unwrap`, not `Sandbox::destroy`.**
  The in-flight clone is only *transiently* live: turn cancellation drops
  the `run` future — and with it the clone — shortly after the hook fires.
  The reaper must wait out that window instead of giving up on the first
  attempt. This is a root fix (reclaim once contention clears), not a
  narrowing.
- **Extract the reclamation loop as a generic, unit-testable helper** in
  `crates/app/src/sandbox.rs` — e.g.
  `reclaim_when_idle<T>(arc: Arc<Mutex<T>>, max_attempts, backoff, reclaim)`
  where `reclaim: FnOnce(T) -> impl Future`. It loops:
  `Arc::try_unwrap` → on `Ok`, `mutex.into_inner()` and hand the owned `T`
  to `reclaim`; on `Err(arc)`, keep the returned `Arc`, sleep `backoff`,
  and retry up to `max_attempts`. Making it generic over the payload lets
  the mechanism be tested with a probe type and **no boxlite dependency**
  (CI has no boxlite; the existing sandbox integration tests are
  `#[ignore]`). `SandboxCleanupHook::on_session_end` calls this helper
  inside its existing detached `tokio::spawn`, passing a `reclaim` closure
  that awaits `Sandbox::destroy` and `tracing::warn!`s on error.
- **Stays fire-and-forget.** The map entry is still removed synchronously
  before the spawn, and reclamation runs in the detached task — cleanup
  must not block the 5s per-hook timeout in
  `LifecycleManager::fire_session_end`
  (`crates/kernel/src/lifecycle.rs`, line ~237), because
  `Sandbox::destroy` plus the retry budget can exceed 5s.
- **Retry cadence is Rust `const`, not YAML.** `max_attempts` and
  `backoff` are mechanism-tuning constants next to the helper. A deploy
  operator has no reason to pick a different reclaim cadence
  (project.spec "Mechanism-tuning constants … are Rust `const`"). This is
  enforced structurally by the boundary: `config.example.yaml` is
  forbidden. The hook passes the named consts; the helper takes them as
  parameters purely so tests can drive a short schedule.
- **Bounded, never unbounded.** The budget is sized to comfortably outlast
  turn-cancellation propagation. If the outstanding clone somehow never
  drops (a genuinely wedged exec/VM — not the timing case this issue
  targets), the reaper stops after `max_attempts` and emits a loud
  `tracing::warn!` carrying `session_key` and the final `strong_count`, so
  the residual is observable and counted rather than a silent forever-spin.
  The detached task therefore always terminates.
- **No change to `rara-sandbox`.** `Sandbox::destroy(self)` is unchanged;
  the minimal-surface invariant in `crates/rara-sandbox/AGENT.md` stands.
  All ownership choreography happens at the app boundary.
- **No mock/noop `Sandbox`.** Per `crates/rara-sandbox/AGENT.md` and
  `docs/guides/anti-patterns.md`, the reclamation test fakes at the caller
  boundary (a probe payload behind the generic helper), never a hollow
  `Sandbox` impl.

## Boundaries

### Allowed Changes
- crates/app/src/tools/run_code.rs
- **/crates/app/src/tools/run_code.rs
- crates/app/src/sandbox.rs
- **/crates/app/src/sandbox.rs
- crates/app/tests/run_code_session.rs
- **/crates/app/tests/run_code_session.rs
- specs/issue-1866-sandbox-inflight-exec-reclaim.spec.md
- **/specs/issue-1866-sandbox-inflight-exec-reclaim.spec.md

### Forbidden
- crates/rara-sandbox/**
- crates/kernel/**
- config.example.yaml
- web/**
- extension/**
- .github/workflows/**

## Acceptance Criteria

Scenario: reclamation waits for an outstanding clone, then reclaims exactly once
  Test:
    Package: rara-app
    Filter: sandbox::tests::reclaim_waits_for_outstanding_clone_then_reclaims_once
  Given a shared handle Arc<Mutex<Probe>> with one extra outstanding clone held by a simulated in-flight task, and a reclaim callback that records each invocation
  When the reaper runs while the clone is briefly held and the clone is then dropped
  Then the reclaim callback is never invoked while the clone is held
  And after the clone drops the reclaim callback is invoked exactly once with owned ownership of the Probe

Scenario: reclamation fires promptly when no clone is outstanding
  Test:
    Package: rara-app
    Filter: sandbox::tests::reclaim_fires_immediately_when_idle
  Given a shared handle with no outstanding clones (strong count is one)
  When the reaper runs
  Then the reclaim callback is invoked exactly once on the first attempt without exhausting the retry budget

Scenario: reclamation is bounded when the outstanding clone never drops
  Test:
    Package: rara-app
    Filter: sandbox::tests::reclaim_bounded_when_clone_never_drops
  Given a shared handle whose outstanding clone is held for the entire run and never dropped, and a small max-attempts budget
  When the reaper runs to completion
  Then the reaper terminates after the bounded budget rather than spinning forever
  And the reclaim callback is never invoked
  And the exhaustion is surfaced (a warn-level escalation) rather than silently swallowed

## Out of Scope

- **Force-tearing-down a VM while an exec still holds a live handle**
  (destroy-by-key / boxlite `remove(force=true)` without owning `self`).
  That would additionally eliminate the pathological "clone never drops"
  residual, but it requires new `rara-sandbox` surface (a deterministic
  per-session box name plus a force-remove-by-name path) and reaches into
  tearing down a VM out from under an in-flight exec — a separate, larger
  change with its own hazards. Flagged for a follow-up; this issue fixes
  the timing leak the code comment actually describes.
- **Cancelling or interrupting the in-flight `exec` itself.** The kernel
  signal pipeline already cancels the turn; this issue only reclaims the
  VM after the exec's clone drops.
- **Real-boxlite end-to-end verification of `destroy`.** That stays in the
  existing `#[ignore]` integration test
  (`crates/app/tests/run_code_session.rs`); CI has no boxlite runtime.
- **Reclamation metrics/accounting beyond a warn log.** Counters or
  dashboards for reclaim outcomes are a later concern.
- Any change to `Sandbox::destroy`'s signature or the `SandboxMap` type.
