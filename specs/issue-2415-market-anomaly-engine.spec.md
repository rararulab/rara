spec: task
name: "issue-2415-market-anomaly-engine"
inherits: project
tags: [enhancement, backend, trading]
---

## Intent

rara's trading extension already ingests Binance `market_candle_closed` events,
stores OHLCV in the TimescaleDB market-data repository
(`crates/extensions/rara-trading/src/market_data/`), matches them against
finance subscriptions (`finance/registry.rs`), and injects synthetic directives
into a session via `crates/app/src/finance_event.rs`. The proactive-feedback
chain is wired end to end. What is missing is any notion of *abnormality*:
`registry.rs::match_event` filters purely on metadata (symbol / timeframe /
source / watch_term) and `finance_event.rs::finance_directive` emits a bare
candle ("Report the market update factually"). Every matched bar is treated
identically — a routine 0.1% drift and a 7% flash crash produce the same
inert directive.

This issue adds the anomaly-evaluation layer that sits between ingestion and
delivery: given the newly closed candle plus a rolling window pulled from the
market-data repository, it computes structured signals (window return, rolling
drawdown, volume surge, a robust MAD-based z-score of returns, and a
Barndorff-Nielsen–Shephard (BNS) jump test separating a true price jump from
normal diffusion) and produces an `AnomalySignal { severity, reason, metrics }`.
It then enriches the synthetic directive so the agent narrates *what happened*
(magnitude, which statistics fired) rather than restating a candle. Delivery
*policy* (cooldown / hourly budget) is unchanged in this issue — that reversal
is issue 2416.

If we do not do this, the following concrete black-swan bug appears.
Reproducer:

1. A user subscribes a session to `binance / BTCUSDT / 1m` with
   `delivery: immediate`.
2. BTCUSDT drops 7% across three consecutive 1m bars on a volume spike (a real
   flash-crash shape).
3. Each closed bar is matched by `match_event` and delivered, but the injected
   directive is `[FinanceMarketUpdate] ... close=<price>\nReport the market
   update factually; do not infer a trade unless the user asks.` — identical
   in wording and salience to a flat-tape bar. The agent has no signal that
   this was a crash, cannot narrate the drawdown or the volume surge, and the
   observed bad outcome is: rara "sees" the black swan yet surfaces nothing
   distinguishable from noise, which is precisely the failure the user is
   asking us to prevent.

This advances `goal.md` signal 2 ("The user stops asking ... they expect rara
to surface the right thing at the right time, unprompted") and signal 4 ("Every
action is inspectable" — `AnomalySignal.reason` + `metrics` make each alert a
readable trace, not a black box). It crosses no "What rara is NOT" line: this
is single-surface depth on the existing crypto feed, not a feature-parity race.

Prior-art search (2026-07-14): `gh issue list` / `gh pr list` on
`rararulab/rara` for `anomaly`, `severity`, `alert`, `volatility`, `market
candle`, `cooldown` returned no prior anomaly / statistical-evaluation work.
The finance feed lineage is PRs 2223 (subscriptions), 2243 / 2278 (candle
subscribe + selector scope), 2276 (candle timestamp validation), 2362 / 2363 /
2312 (candle pagination), and 2411 (bundle status). None added an anomaly or
statistics layer; none is reversed by this work. `git log --all --grep` for
`anomaly|z-score|bipower|jump` since 180 days found nothing in this area. No
prior commit removed a market-anomaly evaluator, so this is greenfield, not a
regression-decision reversal.

## Decisions

- New module `crates/extensions/rara-trading/src/anomaly/` (with its own
  `mod.rs` for re-exports + module docs, logic split into sub-files per the
  `mod.rs`-is-re-exports rule). Public surface: `AnomalySignal`, a `Severity`
  enum, and an evaluator entry point that takes an ordered candle window plus
  the newly closed candle and returns `Option<AnomalySignal>` (`None` = normal,
  no directive enrichment).
- `AnomalySignal` carries `severity: Severity`, a human-readable `reason`
  (which rules/statistics fired, with magnitudes), and the structured `metrics`
  behind them. `Severity` is an ordered enum so issue 2416 can compare against a
  bypass threshold. Use `#[derive(bon::Builder)]` for the 3+ field signal
  struct; `snafu` for any evaluator error type (domain path).
- L1 rules, all over the rolling window: N-bar window return, rolling maximum
  drawdown, and volume surge (current volume vs a rolling mean multiple).
- L2 statistics: a robust z-score of log-returns using the median / MAD (not
  mean / stddev, so a single outlier bar does not inflate the scale), and the
  BNS jump test comparing realized variance against bipower variation to flag
  a discontinuous jump versus continuous diffusion.
- Money/price math uses `rust_decimal` (already a dependency) for candle
  fields; statistical intermediates (log-returns, z-scores, variance) may use
  `f64` — document at the boundary why the conversion is safe (returns are
  ratios, not ledger amounts).
- Window sizes, the MAD-to-sigma consistency constant (~1.4826), the bipower
  scaling constant, the default z-score / drawdown / volume-surge thresholds,
  and the minimum sample count before a statistic is trusted are **mechanism
  constants** → Rust `const` next to the evaluator, NOT YAML. Rationale
  (`docs/guides/anti-patterns.md` "mechanism-tuning constants"): a deploy
  operator has no principled reason to pick a different bipower coefficient or
  MAD multiplier, and a YAML knob would recreate the #1804→#1817 footgun where
  a default config silently disables the fix. The single value a deployment
  might legitimately tune (e.g. per-symbol sensitivity) is out of scope here;
  if it is ever added it goes through `config.example.yaml`, never a hardcoded
  Rust default.
- Wiring: `crates/app/src/finance_event.rs::dispatch_feed_event` already holds
  a `&dyn MarketDataRepository`. On a matched `market_candle_closed` decision,
  query the rolling window via the existing
  `MarketDataRepository::recent_candles(CandleRecentQuery)`, run the evaluator,
  and when a signal is produced, enrich `finance_directive` so the injected
  text names the anomaly (severity + reason + a "describe what happened, the
  related context, and a suggested next action" instruction) instead of the
  bare-candle wording. When the evaluator returns `None`, directive wording is
  unchanged.
- The evaluator is a pure function of its candle inputs (no clock, no I/O), so
  every rule and statistic is unit-testable with fixture windows.

## Boundaries

### Allowed Changes
- **/crates/extensions/rara-trading/src/anomaly/**
- **/crates/extensions/rara-trading/src/lib.rs
- **/crates/app/src/finance_event.rs
- **/crates/extensions/rara-trading/Cargo.toml
- **/crates/extensions/rara-trading/AGENT.md
- **/Cargo.lock
- **/specs/issue-2415-market-anomaly-engine.spec.md

Note: the three paths above the spec file were added during implementation.
`Cargo.toml` gains the `bon` and `snafu` workspace deps this spec's own
Decisions mandate (`#[derive(bon::Builder)]` + snafu error type); `AGENT.md`
is the crate-guidelines file `docs/guides/agent-md.md` requires for the new
`anomaly/` domain; `Cargo.lock` is the mechanical lockfile update from the two
added deps. None touches the Forbidden set.

### Forbidden
- **/crates/extensions/rara-trading/src/finance/registry.rs
- **/config.example.yaml
- **/web/**
- **/extension/**
- Do NOT change the delivery policy (cooldown / hourly-budget / Silent
  downgrade) in this issue — `registry.rs::delivery_action` is untouched here;
  the severity-bypass reversal is issue 2416.
- Do NOT add a YAML knob for any window size, statistical constant, or default
  threshold — mechanism tuning is a Rust `const` (`docs/guides/anti-patterns.md`).
- Do NOT introduce WebSocket ingestion or change the polling transport — poll
  latency is issue 2417.
- Do NOT construct hollow signals: an evaluator path that always returns the
  same `Severity` regardless of input, or a metric field nothing reads, is a
  hollow implementation (`docs/guides/anti-patterns.md`). Every `Severity`
  level and metric must be reachable and asserted by a test.

## Acceptance Criteria

Scenario: A multi-bar crash on a volume spike produces a high-severity signal
  Test:
    Package: rara-trading
    Filter: crash_window_produces_high_severity_anomaly_signal
  Given an ordered candle window ending in a sharp multi-bar decline with a volume spike
  When the anomaly evaluator runs over the window and the newest closed candle
  Then it returns a signal whose severity is the highest (bypass-eligible) level
    And the reason names the drawdown / return magnitude that fired

Scenario: A flat, low-volatility tape produces no anomaly signal
  Test:
    Package: rara-trading
    Filter: flat_tape_produces_no_anomaly_signal
  Given an ordered candle window of small alternating moves at steady volume
  When the anomaly evaluator runs over the window and the newest closed candle
  Then it returns None (no directive enrichment)

Scenario: The robust z-score uses MAD so one prior outlier does not mask a new move
  Test:
    Package: rara-trading
    Filter: robust_zscore_uses_mad_not_stddev
  Given a return series containing one large historical outlier bar
  When the robust z-score of the newest return is computed
  Then the newest return is scored against the MAD-based scale
    And a fresh anomalous return still exceeds the z-score threshold that a
        stddev-based scale would have suppressed

Scenario: The BNS jump test separates a discontinuous jump from diffusion
  Test:
    Package: rara-trading
    Filter: bns_jump_test_flags_jump_over_diffusion
  Given two windows of equal realized variance, one dominated by a single jump bar and one purely diffusive
  When the BNS jump statistic (realized variance vs bipower variation) is computed for each
  Then the jump-dominated window is flagged as a jump
    And the diffusive window is not

Scenario: A produced signal enriches the injected directive with its narrative
  Test:
    Package: rara-app
    Filter: anomaly_signal_enriches_finance_directive
  Given a matched immediate BTCUSDT candle whose rolling window is crash-shaped
  When dispatch_feed_event evaluates the window and injects the synthetic directive
  Then the directive text contains the anomaly severity and reason
    And instructs the agent to describe what happened and a suggested action
    And a matched candle whose window is flat yields the unchanged factual wording

## Out of Scope

- Delivery-policy changes (cooldown / budget bypass) — issue 2416.
- Sub-minute poll latency and rate-limit floor — issue 2417.
- WebSocket ingestion (a later phase).
- Non-crypto assets and non-Binance venues.
- Per-symbol operator-tunable sensitivity via YAML (only if ever needed, in a
  later issue, through `config.example.yaml`).
- Any web / extension UI surface for anomalies.
