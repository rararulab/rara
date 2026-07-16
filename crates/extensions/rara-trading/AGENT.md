# rara-trading — Agent Guidelines

## Purpose
Finance/trading data-feed extension: ingests Binance candles and RSS finance
articles, persists OHLCV history, matches events against user subscriptions,
and evaluates newly closed candles for market anomalies — emitting ordinary
kernel `FeedEvent`s and, on the app side, enriched synthetic directives.

## Architecture
Five modules, each a `mod.rs` (re-exports + `//!` docs only) over sub-files:

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
  signals and keep the existing order stable. `evaluate_with` is the test seam
  (`registered_signal_participates_in_evaluation` injects an extra signal).

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

## Dependencies
- Upstream: `rara-kernel` (`FeedEvent`, `Subscription`, tool traits).
- Downstream: `crates/app` calls `dispatch::on_feed_event` from its feed
  dispatch loop and supplies the `KernelHandle`-backed `FeedDispatchSink`
  (`finance_event.rs`); `market_data::tools` are registered as agent tools.
- External: Binance REST (candles), RSS feeds, TimescaleDB/Postgres
  (`sqlx-postgres`) for durable candle history.
- Style: `snafu` for domain errors, `bon::Builder` for 3+ field structs
  (`AnomalySignal`, `AnomalyMetrics`), `#[async_trait]` on repository traits.
