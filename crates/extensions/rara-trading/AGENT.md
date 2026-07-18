# rara-trading — Agent Guidelines

## Purpose
Finance/trading data-feed extension: ingests Binance candles and RSS finance
articles, persists OHLCV history, matches events against user subscriptions,
and evaluates newly closed candles for market anomalies — emitting ordinary
kernel `FeedEvent`s and, on the app side, enriched synthetic directives.

## Architecture
Six modules, each a `mod.rs` (re-exports + `//!` docs only) over sub-files:

- `dispatch/` — the **market-signal facade** (`pipeline.rs`). `on_feed_event`
  is the single entry point that runs the whole persist → upsert → evaluate →
  match → deliver pipeline for one kernel `FeedEvent`, returning
  `FeedDispatchOutcome`. It owns the app-boundary adapters that were previously
  app glue: the rolling-window pull + degradation policy (`evaluate_anomaly`),
  candle parsing/normalization (`market_candle_from_event`), directive wording
  (`finance_directive`), and the `FeedDispatchSink` trait (session-injection
  side effect, abstracted so the production `KernelHandle`-backed sink can stay
  in `crates/app`). Layer-②/③ consume `on_feed_event` instead of re-plumbing.
- `feed/` — finance-specific ingestion sources (Binance poller, RSS) that parse
  external data into kernel `FeedEvent`s. The kernel owns the generic event
  envelope, registry, and store; this crate only produces events.
  `feed/catalog.rs` also owns the built-in source and bundle catalog consumed
  by app-side finance feed tools. For large market-candle watchlists the app
  first plans against this catalog (`finance_plan_instrument_watchlist`) so
  Binance-style sources account for `symbols × timeframes` request fan-out
  before subscribing or starting a feed.
- `finance/registry.rs` — `FinanceSubscriptionRegistry`: matches events to
  subscriptions on metadata (venue/symbol/timeframe/source/watch_term) and
  decides delivery (`Immediate` vs `Silent`) under a cooldown + hourly budget.
  **Delivery policy lives here and nowhere else.**
- `market_data/` — durable OHLCV storage. `model.rs` (`MarketCandle`,
  `Timeframe`, the `Candle*Query` structs), `repository.rs`
  (`MarketDataRepository` trait + `InMemoryMarketDataRepository`),
  `timescale.rs` (TimescaleDB impl), `tools.rs` (agent-facing query tools).
  `recent_candles(CandleRecentQuery)` returns the newest N candles ascending
  and is the rolling-window source for anomaly evaluation.
- `anomaly/` — market-anomaly evaluation (issue 2415; generalized into a signal
  registry in issue 2429; tail-risk signals added in issue 2436).
  `evaluate(window, latest) ->
  Result<Option<AnomalySignal>>` is a **pure** function of its candle inputs. It
  prepares the shared context once, walks a `SignalRegistry` collecting each
  signal's `SignalOutput`, projects those onto `AnomalyMetrics`, then classifies
  severity. `registry.rs` = the `Signal` trait (`name` / `evaluate` /
  `fragment`), `SignalOutput` (value / fired), `SignalContext` (shared
  closes/returns/volumes), and `builtin_registry()` (**seven** builtin signals);
  the signal's stable name is
  a static property of the `Signal` (used to route its value into
  `AnomalyMetrics`), not per-output data. `rules.rs` = L1 pure functions + their
  `Signal` adapters
  (window return, rolling drawdown, volume surge, directional run — the signed
  trailing same-sign return run that catches a persistent one-directional
  grind); `statistics.rs` = L2 pure
  functions + adapters (MAD-based robust z-score, BNS realized-variance vs
  bipower-variation jump test, volatility regime — the recent-to-baseline
  per-bar realized-variance ratio that catches a sustained dispersion
  expansion); `signal.rs` = `AnomalySignal` / `Severity` /
  `AnomalyMetrics`; `error.rs` = `AnomalyError` (snafu). `evaluator.rs` holds the
  `evaluate` / `evaluate_with(&registry, …)` loop, the metrics projection, and
  `classify`.

  `tools.rs` exposes the evaluator as the deferred, read-only
  `finance_evaluate_candle_signal` agent tool. It pulls either the latest
  stored closed candle or one requested `open_time`, fetches the rolling window
  with `end = Some(latest.open_time)`, delegates to `evaluate`, and returns the
  resolved candle, window status, composite anomaly signal, and the per-builtin
  signal trace (`signal_name`, computed `value`, `fired`, and the fired
  `reason_fragment`). It is an inspection surface for "does this stream
  currently have a rara signal?" and "which builtin signals contributed?", not a
  strategy/deployment/order tool.

  `volatility_regime` and `directional_run` are **orthogonal** tail-risk
  detectors: the first measures dispersion (magnitude), the second sign
  persistence (direction). Both are structurally invisible to the first five —
  a sustained variance regime is in-family for the single-bar MAD z-score and
  diffusive for the BNS jump test; a slow grind crosses no magnitude threshold
  and a monotonic path has no drawdown. Their trip thresholds are semantic
  `const`s (`DIRECTIONAL_RUN_THRESHOLD = 6` bars, `VOLATILITY_REGIME_THRESHOLD =
  4.0×`), **never** tuned to dodge a test fixture.

  **Extending the signal set** = implement `Signal` (a struct wrapping a pure
  stat) + add one `Box::new(...)` line to `builtin_registry()` + (if the value
  should reach the public trace) one `Option<f64>` `AnomalyMetrics` field and one
  `.maybe_<field>(...)` line in `metrics_from`. The core loop in
  `evaluate_with` and `classify` do not change — the
  new signal contributes to the reason (via its `fragment`) and the fired-count.
  `builtin_registry()` order **is** the reason-fragment order (drawdown, return,
  volume, z-score, jump, volatility regime, directional run), so append new
  signals and keep the existing order stable. `evaluate_trace` is the
  agent-facing pure seam for composite + per-builtin trace over the builtin
  registry; `evaluate_with` is the explicit-registry test seam
  (`registered_signal_participates_in_evaluation` injects an extra signal).

- `backtest/` — signal-accuracy backtest harness (issue 2437), the first rung
  of layer ② (decision support). It replays a single symbol/timeframe stream of
  stored candles through `anomaly::evaluate` and, for every bar the evaluator
  fires on, applies one **fixed naive rule** (enter long at the trigger bar's
  close, exit `HOLD_BARS` bars later at that bar's close) to answer "is a signal
  actually any good?". `runner.rs` holds `run_backtest(candles) ->
  Result<BacktestReport>` (the **pure** deterministic composite core the unit
  tests bind to), `run_signal_attribution(candles) ->
  Result<SignalAttributionReport>` (the same no-look-ahead replay grouped by
  builtin signal name), and the thin async `backtest(repo, query)` entry that
  fetches via `MarketDataRepository::candles` and delegates to the core — the same
  pure-core/async-entry seam as `anomaly::evaluate` / the dispatch adapter.
  `report.rs` = `BacktestReport` (`bon::Builder`, the fixed metric set:
  `trigger_count`, `evaluated_trade_count`, `win_count`, `win_rate`, signed
  `mean`/`median_forward_return`, strategy `max_drawdown`) plus
  `SignalAttributionReport` / `SignalAttribution` (the same fixed metrics per
  builtin signal row; a bar can count in multiple rows when multiple signals
  fire); `error.rs` =
  `BacktestError` (snafu; `Evaluate` wraps `AnomalyError`, `FetchCandles` wraps
  the repository read). It only **consumes** `anomaly::evaluate` and
  `market_data`; it changes neither. `tools.rs` exposes the same narrow harness
  as the deferred, read-only `finance_backtest_signal` agent tool: the tool
  validates a single candle range, fetches from the shared market-data
  repository, rejects over-limit ranges instead of paginating a non-additive
  replay, delegates to `run_backtest` and `run_signal_attribution`, and returns
  the fixed composite report, per-signal attribution rows, and diagnostic hints.
  It is not the heavier research-desk `trading_backtest` surface.

The write→read→enrich wiring lives in `dispatch/pipeline.rs` (`on_feed_event`):
it upserts the closed candle, pulls the window via `recent_candles`, runs
`anomaly::evaluate`, and enriches `finance_directive` when a signal is
produced. The evaluator itself holds no repository handle. `crates/app` is thin
wiring: it constructs the deps, supplies the `KernelFeedDispatchSink`
(`crates/app/src/finance_event.rs`), and calls `on_feed_event` from its feed
dispatch loop (`crates/app/src/lib.rs`).

## Critical Invariants
- **The anomaly evaluator is pure — no clock, no I/O.** Every rule and
  statistic must be reproducible from a fixture candle slice. Consequence of
  violation: the L1/L2 logic stops being unit-testable and regressions hide
  behind live data.
- **Anomaly tuning values are `const`, never YAML** (window size, MAD-to-sigma
  ≈1.4826, bipower π/2, z-score/drawdown/volume/jump thresholds, min sample
  count). A deploy operator has no principled reason to retune them; a YAML
  knob recreates the #1804→#1817 footgun where a default config silently
  disables the fix (`docs/guides/anti-patterns.md`). The only value a
  deployment might legitimately tune (per-symbol sensitivity) is out of scope
  and, if ever added, goes through `config.example.yaml`.
- **`Severity` is ordered** (`Watch < Elevated < Critical`) so downstream policy
  compares with `>=`. `Critical` is the bypass-eligible level issue 2416 will
  consume; do not reorder variants.
- **Candle selectors are normalized before storage** (venue lowercase, symbol
  uppercase, timeframe canonical). `recent_candles`/`latest_closed_candle`
  match on the normalized form.
- **Binance watchlist fan-out is guarded before starting a feed.** The app-side
  `finance_subscribe_instruments` path computes the final configured
  `symbols × timeframes` request count and rejects unsafe `start_now=true`
  subscriptions. Unsafe staged subscriptions keep the feed disabled until an
  operator raises `transport.interval_secs`. The app-side
  `finance_enable_feed_source` and `finance_restart_feed_source` controls must
  enforce the same guard, because enabling a persisted disabled feed can start
  it on app restart even when `start_now=false`. Candle diagnostics must reuse
  the same fan-out calculation and avoid enable/restart `next_action_hint`
  values for unsafe feeds. Do not silently persist, start, or recommend starting
  an enabled poller that will hit provider rate limits.
- **`backtest` has no look-ahead** — the number-one backtest bug. Signal
  evaluation for bar `i` sees only `candles[i.saturating_sub(EVAL_WINDOW)..i]`
  plus `latest = candles[i]`, never a bar `> i` (the same invariant `dispatch`
  enforces with `end = Some(latest.open_time)`); forward return reads only bars
  strictly after `i`. A trigger with fewer than `HOLD_BARS` bars remaining is
  counted in `trigger_count` but **excluded** from `evaluated_trade_count` and
  every P&L metric — never zero-filled. Consequence of violation: fabricated
  edge that would ship a future ③ execution gate on a lie.
- **The primary backtest unit is the composite `AnomalySignal`, and forward
  returns are signed.** A composite "trigger" is one bar where `evaluate`
  returns `Some` (what `dispatch` acts on). Per-signal attribution is a secondary
  grouped view over the same replay, not a separate strategy definition. A trade
  wins **iff** its forward return is strictly `> 0.0`;
  because the detectors are tail/volatility signals, a low win rate with a
  negative mean is a valid, informative result. Report the signed mean/median;
  do not take absolute value. `HOLD_BARS` is a `const`, never YAML.

## What NOT To Do
- Do NOT change delivery policy (cooldown / hourly budget / `Silent` downgrade)
  from the `anomaly/` module or `dispatch/` — that is `registry.rs`'s job
  (severity-based bypass is issue 2416). The facade calls `match_event` and
  supplies the already-computed severity; mixing evaluation and delivery
  couples two independently-tested concerns.
- Do NOT let `dispatch/` depend on `crates/app` types (e.g. `KernelHandle`) —
  that inverts the `rara-app → rara-trading` dependency and will not compile.
  Session injection is abstracted behind the `FeedDispatchSink` trait; the
  kernel-backed sink stays in `crates/app`.
- Do NOT return a fixed `Severity` regardless of input, or add an
  `AnomalyMetrics` field nothing reads — that is a hollow implementation
  (`docs/guides/anti-patterns.md`). Every severity level and metric must be
  reachable and asserted by a test. The same bar applies to a `Signal`: a
  production impl that ignores its `SignalContext` and returns a constant, or a
  `SignalOutput` field nothing consumes, is hollow. Every `SignalOutput` field
  has a consumer — `value` populates the `AnomalyMetrics` trace and the
  fragment, `fired` drives the count / severity / reason; the routing name comes
  from `Signal::name()`.
- Do NOT turn severity into a weighted / scored / factor-combination framework
  when adding signals — **why:** `classify` stays concrete (fired-count plus the
  drawdown-keyed critical escalation); a scoring framework is speculative
  machinery (issue 2429 Boundaries). Per-signal trip thresholds stay `const`
  next to each signal, never YAML.
- Do NOT feed the newest candle into `evaluate` twice: `window` is the history
  strictly before `latest`. `dispatch/pipeline.rs` queries `recent_candles`
  with `end = Some(latest.open_time)` to enforce this.
- Do NOT use mean/stddev for the return z-score — one outlier bar inflates the
  scale and masks a fresh move; use median/MAD (`statistics::robust_zscore`).
- Do NOT compute log-returns on non-positive prices — `evaluate` returns
  `AnomalyError::NonPositivePrice` instead of producing garbage.
- Do NOT grow `backtest` into a quant platform — **why:** it self-evaluates
  rara's own signals with one fixed rule and a fixed metric set over a single
  stream (issue 2437 Boundaries). No strategy DSL / selectable-or-parameterized
  rule / parameter search / multi-asset portfolio / Python sandbox /
  `BacktestArtifact` / transaction-cost model / paper-live execution — several
  would cross the "NOT a quant platform for others" line. The heavier
  `trading_backtest` research-desk tool is a separate later issue.
- Do NOT add a `BacktestReport` field nothing reads, or emit `NaN` on an
  empty/trigger-less stream — **why:** hollow output is forbidden
  (`docs/guides/anti-patterns.md`); the empty result is `Option::None` (win
  rate / returns) and `0.0` (drawdown), and every reported field is asserted by
  a `runner.rs` test.

## Dependencies
- Upstream: `rara-kernel` (`FeedEvent`, `Subscription`, tool traits).
- Downstream: `crates/app` calls `dispatch::on_feed_event` from its feed
  dispatch loop and supplies the `KernelHandle`-backed `FeedDispatchSink`
  (`finance_event.rs`); `market_data::tools` are registered as agent tools.
- External: Binance REST (candles), RSS feeds, TimescaleDB/Postgres
  (`sqlx-postgres`) for durable candle history.
- Style: `snafu` for domain errors, `bon::Builder` for 3+ field structs
  (`AnomalySignal`, `AnomalyMetrics`), `#[async_trait]` on repository traits.
