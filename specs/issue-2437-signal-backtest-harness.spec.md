spec: task
name: "issue-2437-signal-backtest-harness"
inherits: project
tags: [enhancement, backend, trading]
---

## Intent

rara's trading extension now *sees* the market: `anomaly::evaluate(window,
latest) -> Result<Option<AnomalySignal>>` fires a composite `AnomalySignal`
(severity `Watch` / `Elevated` / `Critical`) on each newly closed candle, the
`dispatch` facade delivers it, and issue 2429 / PR 2430 turned the five
detectors into an extensible signal registry. That is layer ① (market
perception). What is missing is the first rung of layer ② (decision support):
**a way to ask "is a signal actually any good?"** by replaying stored history
through the same evaluator and measuring what happened next.

This issue builds a **signal-accuracy backtest harness**: replay a single
symbol/timeframe stream of stored candles in time order through
`anomaly::evaluate`, and for every bar where the evaluator fires, apply one
**fixed naive rule** — enter a long at the trigger bar's close, exit `N` bars
later at that bar's close — then report a fixed metric set over those trades:
trigger count, win rate, mean and median forward return (signed), and the
naive strategy's max drawdown. It answers "when this signal fires, what does
price do next, and would the dumbest possible rule have made or lost money?"

This is the concrete predecessor the `goal.md` ③ gate requires. `goal.md`
("Current focus", "Safety invariants (trading)") hard-gates live real-money
execution behind "① and ② being solid **plus** a stretch of paper-trading
track record", and "Simulation first" makes backtest precede live. A
deterministic, replayable signal-accuracy report is the first measurable
predecessor of that track record and the first artifact that makes a signal's
edge inspectable rather than asserted.

If we do not do this, the following concrete bug appears the moment layer ②
tries to advance to ③. Reproducer:

1. The anomaly evaluator fires a `Critical` signal on a historical BTCUSDT 15m
   crash window; the directive is delivered and the user sees it.
2. The user (or spec-author gating a future "rara should act on this signal"
   ③ issue) asks: "when a signal fires on this stream, what is the hit rate
   and the average move over the next few bars — does acting on it make or
   lose money?"
3. Observed bad outcome: there is no way to answer except by eyeballing
   charts. The signal's edge is unfalsifiable, so the ③ gate ("a stretch of
   paper-trading track record") has no measurable predecessor. ③ then either
   ships on faith — crossing the hard "NOT a black box / every trade a
   replayable trace" line — or deadlocks forever under the default-deny
   tiebreak. The ②→③ ladder is missing its bottom rung.

This advances `goal.md` "What working rara looks like" signal 4 (**Every
action is inspectable** — the backtest is a deterministic, replayable trace:
same stored candles in → same trigger set and metrics out, reproducible from a
fixture) and is the direct precursor of signal 7 (every trade a replayable
trace) and the ② layer of Current focus ("backtest, evaluate, and advise").
It crosses no "What rara is NOT" line — and the Boundaries below are what keep
it on the right side of **"NOT a quant platform for others"**: it evaluates
*rara's own* signals with **one fixed rule** and a **fixed metric set** over a
single stream. There is no strategy DSL, no parameter/grid search, no
multi-asset portfolio, no third-party research surface. It is a self-evaluation
of rara's own perception, not a framework for spawning strategies.

**Design divergence to record (not a reversal):** the trading design plan
(`docs/plans/2026-07-10-rara-trading-design.md`, "第二阶段：研究台 MVP")
sketches a much heavier ② backtest — a native `trading_backtest` tool taking
`BacktestSpec + StrategyRef + DatasetRef`, a per-run Python/vectorbt sandbox,
and a hash-manifested `BacktestArtifact`. This issue is a deliberately narrower
first cut: a Rust-native, in-process, single-rule signal-accuracy harness with
**no** Python sandbox, DSL, or dataset-export machinery. Nothing in the plan is
built yet, so this is not reversing a prior decision; it is the minimal rung
that earns "does the signal have edge" before any of that heavier research desk
is justified. The heavier `trading_backtest` tool remains a separate, later
issue and is explicitly out of scope here.

Prior-art search (2026-07-16) against `rararulab/rara` (the canonical repo; the
local `origin` is a behind fork). `gh issue list` / `gh pr list --search` for
`backtest`, `backtest replay pnl`, `signal replay historical`,
`anomaly signal registry` and `git log --all --grep backtest --since=180.days`
return **no** prior backtest issue, PR, or code — the only `rg backtest` hits
are this design doc's "第二阶段" section and passing mentions in the 2429 / 2436
spec Out-of-Scope lists ("Backtesting / historical replay of signals — later
layer-② block"). The adjacent trading work this builds on: issue 2415 / PR 2418
(the anomaly `evaluate` engine), issue 2429 / PR 2430 (the signal registry this
replays through), issue 2425 / PR 2426 (the `dispatch` facade), and the
`market_data` `MarketDataRepository` (`candles(CandleRangeQuery)`) that stores
the history. Sibling OPEN issue 2436 (`volatility_regime` + `directional_run`
signals) *adds* signals; this issue *measures* signals — orthogonal and
complementary, no overlap. No prior decision is being reversed.

## Decisions

- **Minimal backtest unit = the composite `AnomalySignal`, not per-signal.**
  A "trigger" is one bar where `anomaly::evaluate(window, latest)` returns
  `Some(signal)`. The composite `AnomalySignal` is exactly what `dispatch`
  delivers and what rara acts on, so measuring it measures the real act-unit.
  Per-signal attribution would require reaching into the `pub(crate)`
  `SignalRegistry` / `SignalOutput` internals or re-walking the registry; that
  is a documented **future extension**, out of scope here. The public
  `AnomalySignal::severity` is recorded on each trade so the report *may*
  expose a severity slice, but the required metric set below is stream-wide.
- **New module `crates/extensions/rara-trading/src/backtest/`** — `mod.rs`
  (re-exports + `//!` docs only) over sub-files (e.g. `runner.rs` the replay
  loop, `report.rs` the `BacktestReport`, `error.rs` the `snafu` error).
  Structure it around the same purity seam the anomaly module uses:
  - A **pure, deterministic core** `run_backtest(candles: &[MarketCandle]) ->
    Result<BacktestReport>` (no clock, no I/O) that walks an ordered
    single-stream slice, calls `anomaly::evaluate` per bar, applies the naive
    rule, and computes the report. This is the unit tests bind to.
  - A **thin async entry** `backtest(repo, query) -> Result<BacktestReport>`
    that fetches the stream via `MarketDataRepository::candles(CandleRangeQuery)`
    (single source/venue/symbol/timeframe/range) and delegates to the pure
    core. This is the `evaluate`/`evaluate_with` seam pattern the crate already
    uses.
- **Naive rule = a fixed long.** On a trigger bar, enter at that bar's `close`;
  exit `HOLD_BARS` bars later at that bar's `close`. Per-trade forward return =
  `(exit_close - entry_close) / entry_close`. There is exactly one rule; it is
  not selectable. `HOLD_BARS` is a Rust `const` next to the runner (mechanism
  tuning, `docs/guides/anti-patterns.md`) — never YAML.
- **Win-rate definition (unambiguous, testable): a trade wins iff its forward
  return is strictly positive** (`> 0.0`). Win rate = wins / trades-with-a-full-
  forward-window. Because rara's anomaly detectors are *tail/volatility*
  signals rather than directional buy signals, a **low win rate together with a
  negative mean forward return is a valid, informative result** — it says the
  signals are bearish and a future rule should short, not that the harness is
  broken. The signed mean/median forward return is reported precisely so
  direction is visible without being baked into "win". Report the signed
  numbers; do not take absolute value.
- **Fixed metric set** on `BacktestReport` (`#[derive(bon::Builder)]`, 3+
  fields):
  - `trigger_count: usize` — bars where `evaluate` returned `Some`.
  - `evaluated_trade_count: usize` — triggers that have a full `HOLD_BARS`
    forward window (the win-rate / return denominator).
  - `win_count: usize`.
  - `win_rate: Option<f64>` — `None` when `evaluated_trade_count == 0`
    (never a `NaN`).
  - `mean_forward_return: Option<f64>`, `median_forward_return: Option<f64>` —
    signed, `None` when there are no evaluated trades.
  - `max_drawdown: f64` — deepest peak-to-trough fractional decline of the
    naive strategy's equity curve, where the equity curve is the cumulative
    product of `(1 + per-trade forward return)` over trades ordered by trigger
    time; `0.0` when there are no trades.
  Exposing both `trigger_count` and `evaluated_trade_count` makes the
  end-of-stream exclusion inspectable (the `goal.md` inspectability signal),
  not silently dropped.
- **No look-ahead — the number-one backtest bug — is enforced structurally and
  falsified by a test:**
  - Signal evaluation for the bar at index `i` sees only `window` (the
    `EVAL_WINDOW` bars strictly before `i`) and `latest = candles[i]` — never
    any bar at index `> i`. This mirrors the production invariant already in
    the crate AGENT.md ("`window` is the history strictly before `latest`";
    `dispatch` queries `recent_candles` with `end = Some(latest.open_time)`).
  - Forward return for a trigger at `i` reads only bars strictly after `i`
    (up to `candles[i + HOLD_BARS]`). A trigger with fewer than `HOLD_BARS`
    bars remaining is **excluded from `evaluated_trade_count` and all P&L
    metrics** — it is never zero-filled or fabricated, though it still counts
    in `trigger_count`.
- Mechanism constants (`HOLD_BARS`, any win epsilon, the reused `EVAL_WINDOW`)
  are Rust `const`; symbol / venue / timeframe / time range are call inputs
  (`CandleRangeQuery`), not YAML. Errors are `snafu` (`BacktestError`,
  propagating `AnomalyError::NonPositivePrice` from `evaluate`); no `unwrap` in
  non-test code.
- Update `crates/extensions/rara-trading/AGENT.md` with a `backtest/` section
  (purpose, the pure-core/async-entry seam, the no-look-ahead invariant, the
  composite-unit and win-rate decisions) since this adds a module to the crate.

## Boundaries

### Allowed Changes
- **/crates/extensions/rara-trading/src/backtest/**
- **/crates/extensions/rara-trading/src/lib.rs
- **/crates/extensions/rara-trading/AGENT.md
- **/specs/issue-2437-signal-backtest-harness.spec.md

### Forbidden
- **/crates/extensions/rara-trading/src/anomaly/**
- **/crates/extensions/rara-trading/src/dispatch/**
- **/crates/extensions/rara-trading/src/finance/registry.rs
- **/crates/extensions/rara-trading/src/market_data/**
- **/config.example.yaml
- **/web/**
- **/extension/**
- Do NOT modify the anomaly evaluator, the signal registry, the `dispatch`
  facade, the delivery policy in `finance/registry.rs`, or the `market_data`
  repository. This issue only *consumes* `anomaly::evaluate` and
  `MarketDataRepository::candles`; it changes neither. The single edit outside
  `backtest/` is registering the new `pub mod backtest;` in `lib.rs`.
- Do NOT introduce a strategy DSL, a selectable/parameterized rule, parameter
  or grid search, walk-forward optimization, multi-asset or portfolio backtest,
  a Python/vectorbt sandbox, a `BacktestArtifact` manifest, transaction-cost /
  slippage modeling, or paper/live execution wiring — all are out of scope
  (see Out of Scope) and several would cross "NOT a quant platform for others".
- Do NOT let forward-return computation read any bar at or before the trigger
  bar, and do NOT let signal evaluation read any bar after the bar under
  evaluation — that is the look-ahead bug this issue exists to prevent.
- Do NOT fabricate, zero-fill, or extrapolate forward returns for triggers
  without a full `HOLD_BARS` window; exclude them from the P&L denominator.
- Do NOT emit `NaN` metrics for an empty/trigger-less stream — `Option::None`
  (win rate / returns) and `0.0` (drawdown) are the well-defined empty result.
- Do NOT add a YAML knob for `HOLD_BARS` or any threshold — mechanism tuning
  stays a Rust `const` (`docs/guides/anti-patterns.md`).
- Do NOT add a `BacktestReport` field nothing reads, or a code path that
  silently returns an empty report on a real error — hollow output is forbidden
  (`docs/guides/anti-patterns.md`).

## Acceptance Criteria

Scenario: Replaying a fixed candle fixture through the naive-long rule yields deterministic metrics
  Test:
    Package: rara-trading
    Filter: naive_long_backtest_reports_deterministic_metrics_on_fixture
  Given an ordered single-stream candle fixture containing at least one bar the anomaly evaluator fires on, each with a full forward window
  When run_backtest replays the fixture through anomaly::evaluate and applies the fixed enter-at-close / exit-N-bars-later long rule
  Then the report's trigger_count, evaluated_trade_count, win_count, win_rate, mean_forward_return, median_forward_return, and max_drawdown equal the exact values hand-computed from the fixture
    And a trade wins exactly when its forward return is strictly positive

Scenario: Perturbing only bars after each trigger changes P&L but never the trigger set (no look-ahead)
  Test:
    Package: rara-trading
    Filter: signal_evaluation_never_reads_bars_after_the_trigger
  Given a fixture that produces a known trigger set with a flat post-trigger tail, and a second fixture identical up to and including every trigger bar but with the forward bars moved within the no-new-trigger band
  When run_backtest replays both fixtures
  Then the trigger set (which bars fired) is identical across the two runs, proving signal evaluation never read a future bar
    And the forward-return metrics differ across the two runs, proving forward return actually reads the post-trigger window and the assertion is non-vacuous

Scenario: A trigger without a full forward window is counted but excluded from P&L
  Test:
    Package: rara-trading
    Filter: forward_return_excludes_triggers_without_full_hold_window
  Given a fixture whose only anomaly trigger occurs fewer than HOLD_BARS bars before the end of the stream
  When run_backtest replays the fixture
  Then trigger_count includes that trigger
    And evaluated_trade_count excludes it, and win_rate and the forward-return metrics are None rather than a fabricated or zero-filled value

Scenario: A flat, trigger-less stream yields a well-defined empty report
  Test:
    Package: rara-trading
    Filter: flat_tape_yields_zero_triggers_and_empty_report
  Given an ordered candle stream of small alternating moves at steady volume that the anomaly evaluator never fires on
  When run_backtest replays the stream
  Then trigger_count and evaluated_trade_count are zero
    And win_rate, mean_forward_return, and median_forward_return are None and max_drawdown is 0.0, with no NaN

Scenario: The async entry fetches the stream from the repository and produces the same report as the pure core
  Test:
    Package: rara-trading
    Filter: backtest_pulls_stream_via_repository_candles_and_matches_pure_core
  Given an InMemoryMarketDataRepository seeded with the same single-stream candle fixture used by the pure-core test
  When backtest(repo, query) fetches the stream via MarketDataRepository::candles and delegates to run_backtest
  Then the returned report equals the report the pure core produces from the same candles, verifying the repository→core wiring end to end

Scenario: A non-positive candle close fails the backtest with a typed error rather than a garbage report
  Test:
    Package: rara-trading
    Filter: non_positive_close_fails_backtest_with_typed_error
  Given a candle fixture containing a bar whose close is not strictly positive
  When run_backtest replays the fixture and anomaly::evaluate rejects the invalid close
  Then run_backtest returns a typed BacktestError propagating the NonPositivePrice cause
    And it does not return a report computed from the invalid data

## Out of Scope

- Any strategy DSL, selectable or parameterized rule, parameter/grid search,
  walk-forward or hyper-parameter optimization — the rule is one fixed long.
- Multi-asset, multi-stream, or portfolio backtesting; cross-symbol or
  cross-timeframe aggregation — one symbol/timeframe/range per run.
- Per-signal attribution (which of the five/N registry signals drove a
  trigger) — a documented future extension; v1 measures the composite
  `AnomalySignal`.
- Transaction-cost, fee, slippage, or fill modeling; position sizing, capital
  constraints, or netting of overlapping trades — each trigger is one
  independent unit-notional trade (future work).
- The heavier `trading_backtest` tool from the design plan: `BacktestSpec /
  StrategyRef / DatasetRef` inputs, Python/vectorbt sandbox, `BacktestArtifact`
  hash manifest, TSDB→Parquet dataset export — a separate later issue.
- Paper-trading or live/testnet execution wiring, order management, or any
  ③-layer autonomous-execution mechanism.
- Any web / extension UI surface for backtest results; exposing backtest as an
  agent tool.
- Operator-tunable config (YAML) for `HOLD_BARS` or thresholds — mechanism
  constants stay Rust `const`.
