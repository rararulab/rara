spec: task
name: "issue-2436-anomaly-tail-risk-signals"
inherits: project
tags: [enhancement, backend, trading]
---

## Intent

rara's trading extension evaluates market anomalies in
`crates/extensions/rara-trading/src/anomaly/`. PR 2430 (issue 2429) generalized
the five hardcoded signals into an extensible registry: a `Signal` trait
(`name` / `evaluate(ctx)` / `fragment`), a structured `SignalOutput` (`value` /
`fired`), a shared `SignalContext` (prepared `closes` / `returns` /
`history_volumes` / `latest_volume`), and `builtin_registry()` returning the
five builtin signals (window return, max drawdown, volume surge, MAD robust
z-score, BNS jump). Adding a signal is now "implement `Signal` + add one
`Box::new(...)` line to `builtin_registry()`" — the core loop `evaluate_with`
and the `classify` severity logic do not change.

That extensibility seam has exactly one production signal set today: the five it
shipped with. This issue is the first layer-② signal expansion — it cashes the
registry in by **adding two new single-instrument tail-risk signals** the
existing five structurally cannot detect, so rara recognizes a broader class of
anomalies and the "add a signal = trait impl + one line" claim is exercised by
real signals, not just the `#[cfg(test)]` beacon:

- **Volatility-regime shift** (`volatility_regime`): the ratio of recent
  per-bar realized variance to a longer rolling baseline. It reuses
  `statistics::realized_variance`. It fires on a *sustained* expansion of return
  dispersion — many elevated-magnitude bars in a row — which the single-bar
  outlier detectors miss by construction: the MAD z-score compares one fresh
  return against its history (a whole cluster of elevated bars is not an
  outlier once the history is also elevated), and the BNS jump test flags a
  discontinuity against a diffusive path (a regime of many moderate bars is
  diffusive, ratio near one). A quiet tape shifting into a choppy high-variance
  regime therefore trips none of the five today.

- **Directional run** (`directional_run`): the length of the trailing run of
  consecutive same-sign log-returns (signed: `+N` for an up-run, `-N` for a
  down-run). It fires on a sustained one-directional grind — a slow, steady
  march where no single bar is large enough to trip the window-return,
  drawdown, z-score, or jump thresholds, but the *persistence* itself is the
  anomaly. A monotonic grind of small same-sign bars trips none of the five
  today (net move stays under the 3% window-return floor, a monotonic rise has
  zero drawdown, each identical small return is not a z-score outlier, and a
  constant-magnitude path is diffusive).

The two are deliberately orthogonal: `volatility_regime` measures dispersion
(magnitude), `directional_run` measures sign persistence (direction), so a
window crafted to trip one leaves the other silent. Each new signal is a clean
`Signal` impl over a pure function plus one registry line. Because the existing
metrics projection routes each builtin signal's value into a named
`AnomalyMetrics` field, each new signal also adds one `Option<f64>` field to
`AnomalyMetrics` and one `.maybe_<field>(...)` projection line in `metrics_from`
so its value stays inspectable — the minimal coupling the registry design
already anticipated (a signal beyond the builtin set still contributes to the
reason and severity, but its value only reaches the public trace through a named
field). The core `evaluate_with` loop and the `classify` severity logic are
**not** touched; each new signal contributes to the reason via its `fragment`
and to the generic `fired`-count, exactly like the five.

If we do not do this, the following concrete gap appears. Reproducer:

1. A user watches a symbol that drifts from a quiet regime into a choppy,
   high-variance regime over ~5 bars — each bar ±0.8% where the prior baseline
   was ±0.15%, alternating so the net move is ~0 — and separately grinds
   steadily in one direction for six consecutive +0.3% bars.
2. rara's anomaly evaluator runs `evaluate(window, latest)` over each window.
3. Observed bad outcome: `evaluate` returns `None` for both. The variance
   regime change is invisible because the MAD z-score sees the newest bar as
   in-family with an already-elevated history and the BNS jump test sees a
   diffusive (non-discontinuous) path; the directional grind is invisible
   because no single bar crosses any threshold and the monotonic path has no
   drawdown. Two genuine tail-risk shapes — a volatility regime change and a
   persistent one-way trend — pass silently, so the finance directive is never
   enriched and the user is never told. The registry built for exactly this
   expansion sits with a single production signal set.

This advances `goal.md` signal 3 (rara building its own market-analysis
capability — a broader, still-inspectable anomaly vocabulary is the substrate
the layer-② stock-analysis behavior grows on) and signal 4 ("Every action is
inspectable" — each new signal carries a stable `name` and projects a named
`AnomalyMetrics` value, so the evaluation stays a readable per-signal trace, not
an opaque verdict). It crosses no "What rara is NOT" line: the signals are
scoped to rara's own single-process anomaly evaluation, consumed only through
the existing `dispatch` facade, and reinforce the "not a black box" line by
naming and tracing every new detection. It builds directly on the PR 2430
registry and is the layer-② signal-expansion block; backtest, trade
suggestions, and cross-asset correlation are separate later issues, explicitly
out of scope here.

Prior-art search (2026-07-16) against `rararulab/rara` (the canonical repo; the
local `origin` fork has issues disabled). `gh issue list` / `gh pr list` for
`anomaly signal registry volatility`, `tail risk regime momentum gap`,
`anomaly signal registry` returned only the existing anomaly lineage: issue
2415 / PR 2418 (the anomaly engine that introduced the five hardcoded signals),
issue 2416 (severity-graded delivery), issue 2425 / PR 2426 (the market-signal
`dispatch` facade), and issue 2429 / PR 2430 (generalizing the five signals into
the extensible registry this issue extends). `git log --all --grep "anomaly"
--grep "volatility" --grep "regime" --grep "signal registry"` since 180 days
shows the same commits and nothing that added or removed a volatility-regime or
directional-run signal. `rg` for `volatility|regime|momentum|acceleration` in
the trading crate found no existing signal implementation. No prior work added
these signals and no prior decision is being reversed — this is the first
forward extension of the registry PR 2430 landed.

## Decisions

- Work happens entirely inside `crates/extensions/rara-trading/src/anomaly/`
  plus the crate `AGENT.md`. The public re-exports in `anomaly/mod.rs`
  (`evaluate`, `EVAL_WINDOW`, `AnomalySignal`, `AnomalyMetrics`, `Severity`,
  `AnomalyError`, `Result`) and the `evaluate(window, latest) ->
  Result<Option<AnomalySignal>>` signature stay unchanged, so the `dispatch`
  facade consumer needs no edit.
- Add exactly two new `Signal` implementations, each a thin adapter over a new
  pure function:
  - `volatility_regime`: a pure function computing the ratio of recent per-bar
    realized variance (over the last `RECENT_VARIANCE_SAMPLES` returns) to the
    baseline per-bar realized variance (over the preceding returns), reusing
    `statistics::realized_variance` and normalizing each side by its sample
    count so windows of different lengths are comparable. Suggested home:
    `statistics.rs` (L2, next to `realized_variance`). `None` (not evaluable)
    when either side has too few samples (reuse the `MIN_SAMPLES` discipline) or
    the baseline variance collapses to ~zero.
  - `directional_run`: a pure function returning the signed length of the
    trailing run of consecutive same-sign log-returns (`+N` up-run, `-N`
    down-run; zero-magnitude returns break the run). Suggested home: `rules.rs`
    or `statistics.rs` (a structural rule over the return series). `None` when
    the return series is empty (window too short to form a return).
- Each new signal owns its trip threshold as a Rust `const` next to it
  (`docs/guides/anti-patterns.md` mechanism-tuning rule) — no YAML knob.
  Suggested starting values (implementer finalizes against the acceptance
  windows): `VOLATILITY_REGIME_THRESHOLD ≈ 4.0` (recent variance ≥ 4× baseline),
  `RECENT_VARIANCE_SAMPLES ≈ 5`, `DIRECTIONAL_RUN_THRESHOLD ≈ 5–6` (bars). These
  are mechanism constants, not deployment config.
- `DIRECTIONAL_RUN_THRESHOLD` is set by **domain semantics** — how many
  consecutive same-sign bars constitute a persistent, anomalous grind (~5–6) —
  and MUST NOT be inflated to dodge an existing test fixture. In particular, do
  **not** set it to ≥13 so it stays silent on the current
  `single_moderate_rule_produces_watch_severity` window (a 12-bar monotonic
  rise): that window *is* a persistent grind and `directional_run` **should**
  fire on it, so the correct fix is to update that fixture (next decision), not
  to couple the constant to the fixture. Coupling a mechanism constant to a test
  fixture is the anti-pattern this const rule exists to prevent.
- Register the two new signals by **appending** them to `builtin_registry()`
  after the existing five (`jump`), so the reason-fragment order of the existing
  five stays byte-for-byte unchanged (drawdown, return, volume, z-score, jump,
  then volatility_regime, directional_run). Each new signal's `fragment` is a
  human-readable phrase (e.g. `volatility regime {:.1}x`,
  `directional run {:+.0} bars`) appended to the joined reason only when it
  fires.
- Give each new signal an inspectable trace: add one `Option<f64>` field to
  `AnomalyMetrics` per new signal (`volatility_regime`, `directional_run`) and
  one `.maybe_<field>(...)` projection line in `evaluator::metrics_from` routing
  the signal's value into it by its stable `name`. `None` stays `None` (never
  flattened to `0.0`). This is the only edit to `evaluator.rs` — the
  `evaluate_with` loop and `classify` are not touched.
- Severity stays exactly as it is: the generic `fired`-count plus the
  drawdown-keyed `CRITICAL_DRAWDOWN && fired_count >= 2` escalation. The two new
  signals contribute to the `fired`-count like any other signal — a single new
  signal firing yields `Watch`; a new signal firing alongside another yields
  `Elevated` (or `Critical` under the existing drawdown escalation). This issue
  does **not** introduce weights, scores, per-signal severity contributions, or
  any factor-combination framework.
- Update the `single_moderate_rule_produces_watch_severity` evaluator fixture,
  because `directional_run` makes its current window semantically obsolete. That
  test's window is a 12-bar monotonic +0.33%/bar rise (13 rising closes = 12
  consecutive up-returns); once `directional_run` is registered at a semantic
  threshold (~5–6), it legitimately fires on that grind, so the window no longer
  isolates a single rule and `fired_count` becomes 2 → the severity would move
  `Watch → Elevated`. Preserve the test's **intent** ("a single moderate rule
  trips → `Watch`") by replacing the window with one that trips only the
  window-return rule (net move ≥ 3%) while the trailing same-sign run stays
  **below** `DIRECTIONAL_RUN_THRESHOLD` and per-bar variance stays uniform (no
  regime shift) — e.g. a rise whose final bars flatten or tick back so the
  ongoing run is short, or an oscillating staircase that nets ≥ 3% without a
  long trailing run. The assertions stay `severity == Watch`,
  `window_return >= 0.03`, `max_drawdown < 0.03`; only the fixture data changes.
  This is the **one** parity fixture this issue may edit.
- Every field on the new `SignalOutput`s and every new `AnomalyMetrics` field
  must have a consumer asserted by a test (`docs/guides/anti-patterns.md`
  hollow-implementation rule): the fire tests assert the projected
  `metrics.volatility_regime` / `metrics.directional_run` value and the reason
  fragment; a production `Signal` that ignores its `SignalContext` and returns a
  constant is forbidden.
- Errors stay `snafu`; 3+-field structs use `#[derive(bon::Builder)]`; new
  `pub` items (the new `AnomalyMetrics` fields) get `///` docs. Update
  `crates/extensions/rara-trading/AGENT.md` to list the two new builtin signals
  in the anomaly-module description and the count of builtin signals.

## Boundaries

### Allowed Changes
- **/crates/extensions/rara-trading/src/anomaly/**
- **/crates/extensions/rara-trading/AGENT.md
- **/specs/issue-2436-anomaly-tail-risk-signals.spec.md

### Forbidden
- **/crates/extensions/rara-trading/src/dispatch/**
- **/crates/extensions/rara-trading/src/finance/**
- **/crates/extensions/rara-trading/src/feed/**
- **/crates/extensions/rara-trading/src/market_data/**
- **/crates/extensions/rara-trading/src/lib.rs
- **/crates/app/**
- **/config.example.yaml
- **/web/**
- **/extension/**
- Do NOT change the `evaluate_with` core loop or the `classify` severity logic —
  the two new signals must participate purely by implementing `Signal` and
  appending one registry line each; the only `evaluator.rs` edit is the
  `metrics_from` projection (one `.maybe_<field>(...)` line per new signal) plus
  its imports.
- Do NOT alter the fire condition, reason fragment, or severity contribution of
  any of the five existing signals — their outputs must stay bit-for-bit
  identical.
- Do NOT change the parity **outcomes** of the crash (`→ Critical`), flat
  (`→ None`), two-rules (`→ Elevated`), and non-positive-price (`→ error`)
  evaluator/unit tests — these must stay bit-for-bit green (the two new signals
  must not fire on those windows, or must not change their severity outcome;
  the crash reason uses `contains`, so appended fragments are fine there).
- You MAY (and must) update exactly one existing parity fixture — the
  `single_moderate_rule_produces_watch_severity` window — because the new
  `directional_run` signal makes its old 12-bar-grind window semantically
  obsolete (see Decisions). Its Watch **intent** and assertions are preserved;
  only the window data changes. No other existing test may be edited.
- Do NOT reorder `builtin_registry()` — append the new signals after `jump` so
  the existing reason-fragment order is preserved.
- Do NOT add a YAML knob for any threshold or window size — mechanism tuning
  stays a Rust `const` (`docs/guides/anti-patterns.md`).
- Do NOT build a weight / scoring / factor-combination framework, per-signal
  severity weights, or config-driven signal selection — severity stays the
  concrete fired-count plus drawdown escalation.
- Do NOT construct hollow outputs: a `SignalOutput` field or `AnomalyMetrics`
  field nothing reads, or a `Signal` impl that returns a constant regardless of
  its `SignalContext`, is forbidden.

## Out of Scope

- **Cross-asset / cross-symbol correlation-breakdown signals.** They require a
  multi-instrument window, but the `dispatch` facade prepares and passes only a
  single-symbol window today (`SignalContext` holds one symbol's
  closes/returns/volumes). Adding them is an independent architecture change to
  the window-preparation seam and is a separate later issue — this issue does
  not touch it.
- Backtest, trade suggestions, and any layer-③ trade-execution behavior.
- Per-symbol sensitivity tuning or any deployment-config surface for signals.

## Acceptance Criteria

Scenario: The volatility-regime signal fires on a sustained variance expansion
  Test:
    Package: rara-trading
    Filter: volatility_regime_fires_on_sustained_variance_expansion
  Given a return series whose recent bars carry a much larger per-bar realized variance than the earlier baseline bars
  When the volatility-regime signal evaluates the prepared context
  Then it reports a computed ratio value
    And it fires because the recent-to-baseline variance ratio crosses its threshold

Scenario: The volatility-regime signal stays silent on a stable-variance tape
  Test:
    Package: rara-trading
    Filter: volatility_regime_silent_on_stable_variance
  Given a return series whose recent and baseline bars carry comparable per-bar variance
  When the volatility-regime signal evaluates the prepared context
  Then it does not fire
    And on a window too short to split into recent and baseline it reports no value (not evaluable)

Scenario: The directional-run signal fires on a sustained one-directional grind
  Test:
    Package: rara-trading
    Filter: directional_run_fires_on_sustained_directional_grind
  Given a return series ending in a run of consecutive same-sign returns at or above the run threshold
  When the directional-run signal evaluates the prepared context
  Then it reports the signed run length as its value
    And it fires because the run length crosses its threshold

Scenario: The directional-run signal stays silent on a choppy tape
  Test:
    Package: rara-trading
    Filter: directional_run_silent_on_choppy_tape
  Given a return series of alternating-sign returns so the trailing same-sign run is short
  When the directional-run signal evaluates the prepared context
  Then it does not fire

Scenario: The volatility-regime signal alone enriches an otherwise unremarkable tape
  Test:
    Package: rara-trading
    Filter: volatility_regime_alone_enriches_unremarkable_tape
  Given a candle window that shifts into a high-variance regime the five existing signals leave unremarkable
  When the registry-driven evaluator runs over the window and the newest closed candle
  Then it now returns a signal because the volatility-regime signal fired
    And the reason contains the volatility-regime fragment
    And the projected metrics carry the volatility-regime ratio value
    And none of the five existing signals' fragments appear (only the new signal lifted the tape)

Scenario: The directional-run signal alone enriches an otherwise unremarkable tape
  Test:
    Package: rara-trading
    Filter: directional_run_alone_enriches_unremarkable_tape
  Given a grind of small same-sign bars whose trailing run reaches the run threshold while the net move stays below the window-return floor, so the five existing signals leave it unremarkable
  When the registry-driven evaluator runs over the window and the newest closed candle
  Then it now returns a signal because the directional-run signal fired
    And the reason contains the directional-run fragment
    And the projected metrics carry the signed directional-run value
    And none of the five existing signals' fragments appear (only the new signal lifted the tape)

Scenario: A single moderate rule still yields Watch severity on a window with no persistent grind
  Test:
    Package: rara-trading
    Filter: single_moderate_rule_produces_watch_severity
  Given a candle window that trips only the window-return rule — a net move of at least 3% whose trailing same-sign run stays below the directional-run threshold and whose per-bar variance is uniform
  When the registry-driven evaluator runs over the window and the newest closed candle
  Then it returns Watch severity, unchanged in intent by the two added signals
    And only the window-return rule contributed to the fired count

Scenario: A multi-bar crash on a volume spike still produces the same high-severity signal
  Test:
    Package: rara-trading
    Filter: crash_window_produces_high_severity_anomaly_signal
  Given an ordered candle window ending in a sharp multi-bar decline with a volume spike
  When the registry-driven evaluator runs over the window and the newest closed candle
  Then it returns the highest (bypass-eligible) severity, unchanged by the two added signals
    And the reason still names the drawdown and window-return magnitudes

Scenario: A flat, low-volatility tape still produces no anomaly signal
  Test:
    Package: rara-trading
    Filter: flat_tape_produces_no_anomaly_signal
  Given an ordered candle window of small alternating moves at steady volume
  When the registry-driven evaluator runs over the window and the newest closed candle
  Then it returns None, unchanged by the two added signals
