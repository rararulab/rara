spec: task
name: "issue-1976-loki-log-noise"
inherits: project
tags: ["telemetry", "kernel", "channels"]
---

## Intent

Since PR 1949 / 1952 turned on OTLP log export to Loki, the Loki tenant for
rara is dominated by three sources of repetitive, low-information lines.
Sampling 30 minutes of `service.name=rara` logs in Loki shows roughly 95%
of all entries come from:

1. `rara_kernel::data_feed::polling` `WARN poll fetch failed` — one
   misconfigured source (yahoo-tsla returning 429) emits a fresh WARN
   on every poll cycle. Same `error=` string, same `feed=` field, looping
   forever until the source is reconfigured.
2. `rara_kernel::schedule` `INFO scheduler drain completed` — fires on
   every scheduler tick regardless of whether anything was drained
   (`fired_count=0 cron_expired_count=0` is the typical shape).
3. `rara_channels::wechat::adapter` `INFO wechat poll returned messages` —
   fires on every successful poll, including the common case of `count=0`.

Real semantic events (`deliver_to_endpoints`, `delivering to adapter`,
`delivery succeeded`) are still INFO and stay INFO — they are exactly the
signal currently being drowned. The producer side, not Loki / Alloy, is
the place to fix this: the call sites are wrong about what severity they
should emit at, not Loki about what to keep.

Reproducer for "what bug appears if we don't do this": (1) open the Loki
explore view filtered to `service_name="rara"`, last 30 minutes; (2) the
visible page is wall-to-wall heartbeat lines from the three sources above;
(3) any attempt to inspect a real session-level event (e.g. find the line
where a message was delivered to the wechat adapter) requires writing a
LogQL `!=` filter excluding the heartbeat patterns by hand. The system
that was set up to make rara inspectable has become unreadable, defeating
the purpose of PR 1949.

Goal alignment: this advances `goal.md` signal 4 ("every action is
inspectable") — Loki is the inspection medium for rara's runtime
behavior, and a 95%-noise feed is not inspectable. It also defends signal
1 ("the process runs for months without intervention") — slow leaks and
recurring transient failures are exactly the things that show up in logs
first, and a noisy feed hides them. Does not cross any "What rara is
NOT" line: this is internal observability, not user-facing surface.

Hermes positioning: not applicable; this is producer-side log hygiene
specific to rara's tracing call sites.

Prior art search summary:

- `gh issue list --search "log noise verbose loki" --state all` — no prior
  issues.
- `gh pr list --search "log noise verbose throttle dedup" --state all` —
  no prior PRs touching log severity / dedup.
- `git log --grep "log level|verbos|noisy|throttl|dedup|loki"` since 180
  days — no prior demote/throttle attempts in this code area.
- `git log -- crates/common/telemetry/src/logging.rs` — telemetry stack
  was iterated three times (1855 Langfuse traces, 1928 OTLP proxy fix,
  1949/1952 Loki logs export, 1960/1962 blocking reqwest), but nobody
  has yet looked at producer-side log discipline.
- The mechanism-vs-config sequence (issues 1804 → 1817 → 1831 → 1882) is
  thematically relevant precedent for the rule "use Rust `const`, not
  YAML, for internal mechanism tuning". This spec adheres to that rule —
  no YAML knob is introduced.

No prior decision is being reversed. The fix is additive on top of the
shipping Loki integration.

## Decisions

### Heartbeat-class events: demote and condition on payload

`rara_kernel::schedule::JobWheel::drain_expired`
(`crates/kernel/src/schedule.rs:566`) emits a single `info!(... "scheduler
drain completed")` on every drain — including the common case where the
drain produced zero fires. Demote that line to `debug!`. When `fired.len()
> 0` or `cron_expired.len() > 0`, emit an `info!` instead with the same
fields, because a non-empty drain *is* the semantic event worth
recording. This keeps the existing "drain happened" structured log
available at the DEBUG level for deep debugging without flooding INFO.

`rara_channels::wechat::adapter` polling loop
(`crates/channels/src/wechat/adapter.rs:190`) emits `info!(count = ...,
"wechat poll returned messages")` on every poll, including `count=0` (the
overwhelming majority of polls). Replace with: when `messages.is_empty()`
→ `debug!`, when non-empty → keep `info!`. The non-empty case is the
semantic event ("the channel actually received traffic"); the empty case
is heartbeat noise.

### Repetitive failure: emit on transitions, not on every cycle

`rara_kernel::data_feed::polling::PollingSource::record_error`
(`crates/kernel/src/data_feed/polling.rs:143`) currently logs
`warn!(error = %message, "poll fetch failed")` on every failure. The same
struct already tracks transition state through `in_error: AtomicBool`
(`polling.rs:127, 145, 160`) — that boolean is the dedup key. The
existing `swap` already returns the previous value, so the WARN can be
gated behind it without any new state.

Behavior after the change:

- First failure in a streak → `warn!` (gate: `previous == false`).
- Subsequent failures in the same streak → `debug!` (gate: `previous == true`).
- Recovery (failure → success transition) → `info!("data feed recovered")`
  inside `record_success`, gated on the existing
  `if self.in_error.swap(false, ...)` block.

The streak state is per `PollingSource`, in-memory, lost on restart. The
first-failure WARN re-firing on restart is itself useful — it tells the
operator "this source is still broken after a restart" — so the
restart-resets-state semantics are intentional, not a bug to paper over.

The intermediate-failure DEBUG path keeps the structured `error =`
field intact, so a developer who wants to see every failure can do so by
raising the level filter; the default Loki feed only sees first-failure
WARN and recovery INFO.

### Why call-site surgery, not a shared throttle abstraction

The three call sites have three different "right" behaviors: schedule
needs a payload-conditional demotion, wechat needs the same shape but on
a different field, polling needs a transition-based dedup that already
has its state primitive in place. A shared `RateLimitedLogger` /
`LogThrottle` trait would unify the surface but would force each call
site through an indirection that does not match its actual semantics —
exactly the over-abstraction that
`docs/guides/anti-patterns.md` warns against, and exactly the failure
mode of the 1804/1817/1831/1882 sequence (a YAML knob meant to "be
flexible" turned into a footgun where every default config disabled the
fix). Three small surgical diffs — totalling a handful of lines per site
— are the entire correct surface. No new abstraction, no new constants,
no new YAML.

### Default log targets are not changed

`crates/common/telemetry/src/logging.rs:437`
`DEFAULT_LOG_TARGETS = "warn,rara=info,rara_=info,common_=info,yunara_=info,base=info"`
is left as-is. The flooding scopes are all rara-owned (`rara_kernel`,
`rara_channels`); the fix lives at the call sites in those crates, not
in the shared filter string. No third-party scope was observed flooding
in the sampled 30 minutes.

## Boundaries

### Allowed Changes

- `crates/kernel/src/schedule.rs` — demote `drain_expired`'s `info!` to
  `debug!`, add a conditional `info!` when at least one job fired or
  expired. Surgical: roughly 6 lines changed.
- `crates/channels/src/wechat/adapter.rs` — split the existing single
  `info!` into `debug!`/`info!` based on `messages.is_empty()`. Roughly
  4 lines changed.
- `crates/kernel/src/data_feed/polling.rs` — gate the existing
  `record_error` WARN behind the `in_error.swap(true, ...)` previous
  value, route subsequent failures to `debug!`, add a recovery `info!`
  in `record_success` inside the existing `if self.in_error.swap(false,
  ...)` block. Roughly 10 lines changed.
- New tests asserting the severity decisions (one per call site, see
  Completion Criteria for selectors). The verified call sites
  (`record_error`, `record_success`, `log_poll_result`) are private,
  so the tests are inline `#[cfg(test)] mod ...` blocks at the bottom
  of each source file rather than separate integration tests.
- `crates/kernel/Cargo.toml` and `crates/channels/Cargo.toml` —
  `tracing-subscriber` added to `[dev-dependencies]` so the inline
  tests can install a capturing subscriber. No runtime dep change.
- **/crates/kernel/src/schedule.rs
- **/crates/channels/src/wechat/adapter.rs
- **/crates/kernel/src/data_feed/polling.rs
- **/crates/kernel/Cargo.toml
- **/crates/channels/Cargo.toml
- **/specs/issue-1976-loki-log-noise.spec.md

### Forbidden

- Do NOT introduce a `RateLimitedLogger`, `LogThrottle` trait, or any
  cross-cutting log-throttling abstraction. The three call sites get
  three surgical fixes.
- Do NOT add any YAML config knob for log levels, dedup windows, or
  rate limits. Per the mechanism-vs-config rule in
  `docs/guides/anti-patterns.md`, internal-tuning constants live as
  Rust `const` or are derived from existing state — not as YAML.
- Do NOT change `DEFAULT_LOG_TARGETS` in
  `crates/common/telemetry/src/logging.rs`. The fix is at the call
  sites, not in the global filter.
- Do NOT touch `crates/common/telemetry/src/payload_sampler.rs` or the
  payload-sampling layer — that is a separate concern with its own
  contract.
- Do NOT change anything in the Loki / Alloy / Grafana stack
  (`infra/`, `docs/guides/debug.md`). This is producer-side only.
- Do NOT introduce a new persistent state store for streak tracking —
  the existing `AtomicBool` in `PollingSource` is the dedup key. Restart
  resets it; that is intentional.
- Do NOT remove the structured `error=` field from intermediate-failure
  log lines. They drop in severity (WARN → DEBUG), not in fidelity.
- Do NOT mark any new test `#[ignore]`.

## Completion Criteria

Scenario: Empty scheduler drain emits DEBUG, non-empty drain emits INFO
  Test:
    Package: rara-kernel
    Filter: schedule_log_levels
  Given a JobWheel with no expired jobs at the current timestamp
  When drain_expired is invoked five consecutive times with an empty wheel
  Then exactly zero INFO "scheduler drain completed" lines are emitted and the same number of DEBUG lines is emitted as drain calls

Scenario: Wechat poll with zero messages emits DEBUG, non-empty emits INFO
  Test:
    Package: rara-channels
    Filter: wechat_adapter_log_levels
  Given a wechat poll handler that receives a response containing an empty msgs array
  When the poll-handling code path runs that response
  Then no INFO "wechat poll returned messages" line is emitted and a DEBUG line carrying count=0 is emitted instead
  Given the same handler receives a response with one message
  When the same code path runs
  Then exactly one INFO "wechat poll returned messages" line is emitted with count=1

Scenario: Repeated polling failures emit one WARN and a recovery INFO
  Test:
    Package: rara-kernel
    Filter: data_feed_polling_log_levels
  Given a PollingSource starting in the non-error state
  When record_error is invoked ten consecutive times with the same error message
  Then exactly one WARN "poll fetch failed" line is emitted and the remaining nine failures are emitted at DEBUG severity
  Given the source is currently in the error state
  When record_success is invoked
  Then exactly one INFO line announcing recovery is emitted

## Out of Scope

- Payload-sampling layer (`payload_sampler.rs`) — separate contract.
- Loki / Alloy / Grafana configuration (retention, query throughput,
  index labels) — this is a producer-side fix.
- Other log-noise sources not currently observed flooding in the Loki
  sample (e.g. `rara_app::http`, `tower_http`, third-party crates). If
  new flooding sources surface after this PR lands, they are addressed
  by separate issues with the same surgical pattern.
- Adding a CI gate that detects new heartbeat-class log lines
  automatically. The rule "INFO is for semantic events, not heartbeats"
  is reviewer-enforced prose, not a workflow check.
- Tightening `DEFAULT_LOG_TARGETS`. No third-party scope was observed
  flooding; demoting rara-owned call sites is sufficient.
