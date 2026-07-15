spec: task
name: "issue-2416-severity-graded-delivery"
inherits: project
tags: [enhancement, backend, trading]
---

## Intent

The finance delivery layer has a reverse-design defect that directly defeats
black-swan alerting. `crates/extensions/rara-trading/src/finance/registry.rs::delivery_action`
downgrades an `Immediate` subscription to `Silent` whenever, within the trailing
hour, the subscription has either delivered inside its `cooldown_secs` window
(default 900s) or already spent its `max_immediate_per_hour` budget (default 6).
That policy is metadata-blind: it cannot tell a routine drift bar from a crash.
The consequence is exactly backwards — when a market event is *most* dense and
*most* severe (a crash prints many bars fast), the subscription hits its budget
/ cooldown and rara falls **silent** precisely when it should be shouting.

Issue 2415 introduced the anomaly-evaluation layer that produces an
`AnomalySignal { severity, reason, metrics }` for each matched candle and
enriches the injected directive. This issue consumes that severity to make
delivery severity-aware: a high-severity anomaly (an actual alert-grade event)
**bypasses** the cooldown and hourly budget and is delivered immediately, while
low-severity / routine events remain budget-constrained (the "digest" lane).
This splits one undifferentiated stream into two intents — routine digest vs
anomaly alert — so a burst of critical bars wakes the session every time
instead of being throttled into silence.

If we do not do this, the following concrete bug appears. Reproducer:

1. A session subscribes to `binance / BTCUSDT / 1m`, `delivery: immediate`,
   `cooldown_secs: 900`, `max_immediate_per_hour: 6` (the defaults).
2. BTCUSDT flash-crashes: eight consecutive 1m bars each fire a high-severity
   anomaly signal from the 2415 evaluator.
3. `delivery_action` delivers bars 1–6 immediately, then downgrades bars 7 and
   8 to `Silent` on `hourly_budget` (and, with the default 900s cooldown, would
   in fact silence everything after bar 1 on `cooldown`). The observed bad
   outcome: during the single most dangerous window of the day, the alerts that
   matter most are appended silently to the tape and never wake the agent —
   the throttle designed for routine chatter muzzles the crash.

This advances `goal.md` signal 2 ("surface the right thing at the right time,
unprompted") — the whole point of the throttle-bypass is that a genuine alert
is never the thing that gets throttled. It also keeps signal 4 intact: the
`downgraded_reason` / bypass decision stays an inspectable field on the delivery
decision. It crosses no "What rara is NOT" line.

Prior-art search (2026-07-14): the budget / cooldown logic was introduced with
the finance-subscription feature (PR 2223) and refined by PR 2278
(selector-scope) and PR 2270 (config normalization). No PR reversed or
reconsidered the "downgrade to Silent under budget" behavior — this issue is
the first to make it severity-aware, not a re-introduction of something a prior
PR removed. `git log --all --grep "cooldown|delivery_action"` since 180 days
shows no removal of a severity-aware path. So this is a first-time fix, not a
regression-decision reversal.

## Decisions

- Thread the `AnomalySignal.severity` produced in issue 2415 into the delivery
  decision. High-severity (at or above a bypass threshold, expressed against the
  ordered `Severity` enum) forces `FinanceDeliveryAction::Immediate` regardless
  of `cooldown_secs` and `max_immediate_per_hour`. Below the threshold, the
  existing budget / cooldown logic is preserved unchanged (routine digest lane).
- The bypass threshold is a **mechanism constant** (Rust `const` next to the
  delivery logic), not YAML — a deploy operator has no principled reason to
  retune "which severity counts as an alert" per deployment, and a YAML knob
  would recreate the config-silently-disables-the-fix footgun
  (`docs/guides/anti-patterns.md`). `cooldown_secs` and `max_immediate_per_hour`
  remain per-subscription fields as today (they are genuine per-user
  preferences, already persisted).
- Bypass must remain **observable**: when a high-severity event bypasses the
  throttle, the delivery decision records that it was an alert bypass (e.g. a
  populated reason/flag), so the choice is inspectable per `goal.md` signal 4.
  Do NOT silently drop the `downgraded_reason` bookkeeping for the routine lane.
- Idempotency is unchanged: the existing `already_delivered` ledger guard still
  prevents a re-observed event id from being delivered twice; bypass changes
  *whether* a first delivery is Immediate vs Silent, not the dedupe.
- The seam: severity is computed in `crates/app/src/finance_event.rs`
  (issue 2415 already evaluates the window there). This issue passes that
  severity into the registry decision — `registry.rs` gains a severity-aware
  delivery path (e.g. `delivery_action` / `match_event` take an optional
  severity), and `finance_event.rs` supplies it. The registry stays the single
  home of the budget/cooldown/bypass rule; the app does not fork a parallel
  policy.
- Directive wording differentiates the two lanes so the agent knows whether it
  was woken for an alert or a routine update (building on the 2415 enrichment).

## Boundaries

### Allowed Changes
- **/crates/extensions/rara-trading/src/finance/registry.rs
- **/crates/app/src/finance_event.rs
- **/specs/issue-2416-severity-graded-delivery.spec.md

### Forbidden
- **/crates/extensions/rara-trading/src/anomaly/**
- **/config.example.yaml
- **/web/**
- **/extension/**
- Do NOT change the anomaly evaluator or its constants — this issue consumes
  the `Severity` from issue 2415, it does not redefine it.
- Do NOT add a YAML knob for the bypass threshold — mechanism const
  (`docs/guides/anti-patterns.md`).
- Do NOT remove or weaken the routine-lane budget / cooldown for low-severity
  events — the throttle still exists for chatter; only alerts bypass it.
- Do NOT remove the `already_delivered` idempotency guard.
- Do NOT let bypass become a silent path: the bypass decision must stay a
  recorded, inspectable field (no hollow / undocumented control flow).

## Acceptance Criteria

Scenario: A high-severity anomaly bypasses an active cooldown
  Test:
    Package: rara-trading
    Filter: high_severity_bypasses_cooldown
  Given an immediate subscription with a non-zero cooldown that has just delivered
    And a subsequent matched event carrying a high (bypass-eligible) severity
  When the delivery action is computed inside the cooldown window
  Then the action is Immediate
    And the decision records that it was an alert bypass

Scenario: A high-severity anomaly bypasses an exhausted hourly budget
  Test:
    Package: rara-trading
    Filter: high_severity_bypasses_hourly_budget
  Given an immediate subscription whose max_immediate_per_hour is already spent
    And a subsequent matched event carrying a high (bypass-eligible) severity
  When the delivery action is computed
  Then the action is Immediate rather than a hourly_budget downgrade

Scenario: A low-severity event still obeys the routine budget
  Test:
    Package: rara-trading
    Filter: low_severity_still_downgrades_under_budget
  Given an immediate subscription whose hourly budget is already spent
    And a subsequent matched event carrying a low (below-threshold) severity
  When the delivery action is computed
  Then the action is Silent with a hourly_budget reason (routine lane unchanged)

Scenario: A burst of critical candles wakes the session on every bar
  Test:
    Package: rara-app
    Filter: critical_candle_burst_wakes_session_every_bar
  Given an active session subscribed immediate to a crypto stream with default cooldown and budget
  When a burst of consecutive candles each evaluates to a high-severity anomaly
  Then every bar in the burst produces a synthetic turn (none is silenced by throttle)
    And the same burst at low severity is throttled after the budget is spent

## Out of Scope

- Defining or computing the anomaly severity — issue 2415.
- Sub-minute poll latency — issue 2417.
- Changing the persisted per-subscription `cooldown_secs` /
  `max_immediate_per_hour` schema or defaults.
- WebSocket ingestion, non-crypto assets, and any web / extension UI.
