spec: task
name: "issue-2429-anomaly-signal-registry"
inherits: project
tags: [refactor, backend, trading]
---

## Intent

rara's trading extension evaluates market anomalies in
`crates/extensions/rara-trading/src/anomaly/`. Today the five signals are
hardcoded into the evaluator: `evaluator.rs::evaluate` builds an
`AnomalyMetrics` by calling each signal function by name — `rules::window_return`,
`rules::max_drawdown`, `rules::volume_surge` (L1) plus
`statistics::robust_zscore` and `statistics::jump_ratio` (L2) — and then
`classify` hardcodes, per signal, a `*_fired` boolean, the `fired_count` array,
and a bespoke `parts.push(...)` reason fragment. Adding a sixth signal means
editing three disjoint places (the builder call in `evaluate`, the fired-set and
count in `classify`, and the reason-formatting block) plus threading a new field
through the `AnomalyMetrics` struct. The signals are not an extensible set; they
are welded into the core control flow.

This issue is the foundation ("地基") for layer-② decision support: it
generalizes the five hardcoded signals into an **extensible signal registry**.
A minimal `Signal` trait describes one signal (evaluate over the prepared window
context → a structured output carrying its stable name, its value, whether it
fired, and its inspectable contribution). The five existing signals move behind
that trait and register into a builtin registry; `evaluate` changes from a
hand-composed builder call into a loop that walks the registry, collects each
signal's output, and hands the collected set to the severity classifier. The
observable win is the extensibility point: **adding a sixth signal becomes
"implement the trait + register one line", with no edit to the core `evaluate`
loop.** Behavior is preserved bit-for-bit for the existing five signals — same
`AnomalyMetrics`, same `Severity` verdicts, same `reason` text, same enriched
directive.

If we do not do this, the following concrete maintenance bug appears when
layer-② adds its next signal (e.g. a realized-volatility spike). Reproducer:

1. An engineer implements the new statistic as a pure function.
2. To make `evaluate` actually use it they must: add a field to
   `AnomalyMetrics`, add a `.maybe_<x>(...)` call to the builder inside
   `evaluate`, add an `<x>_fired` local plus a new entry in the `fired_count`
   array inside `classify`, and add a `parts.push(...)` branch in the reason
   block — four edits across two functions for one signal.
3. Observed bad outcome: `evaluate`/`classify` grow monotonically and become a
   merge-conflict magnet across the parallel signal-adding PRs the layer-②
   roadmap requires (more signals, then backtest, then trade suggestions).
   There is no isolated unit boundary for "signal N" — its threshold, its reason
   wording, and its metric live in three files — so each new signal re-touches
   shared control flow and risks perturbing the other four. This is precisely
   the "别扭、不可扩展" the layer-② plan flags as the thing to fix before
   growing the signal set.

This advances `goal.md` signal 3 (rara building its own market-analysis
capability — the extensible registry is the substrate the layer-② signals grow
on) and signal 4 ("Every action is inspectable" — each signal's structured
output carries a stable name and value, so the evaluation is a readable
per-signal trace rather than an opaque verdict). It crosses no "What rara is
NOT" line: the registry is scoped to rara's own anomaly evaluation and is
consumed only through the existing single-process `dispatch` facade — it is
**not** a general agent/plugin framework, and it exposes no new surface to any
other user or product. It is the prerequisite for the later layer-② blocks
(additional signals, backtest, trade suggestions), each of which is a separate
issue and explicitly out of scope here.

Prior-art search (2026-07-15) against `rararulab/rara` (the canonical repo; the
local `origin` fork has issues disabled). `gh issue list` / `gh pr list` for
`anomaly signal registry`, `trading signal extensible factor`, `signal registry
trait` returned only the existing anomaly lineage: issue 2415 / PR 2418 (the
anomaly engine that introduced these five hardcoded signals), issue 2416 / PR
2424 (severity-graded delivery), issue 2425 / PR 2426 (consolidating market
signal orchestration behind the `dispatch` facade), and issue 2417 / PR 2419
(candle polling floor). `git log --all --grep "anomaly"` / `--grep "signal
registry"` since 180 days shows the same three trading commits and nothing that
introduced or removed a signal-registry abstraction. No prior work built a
signal registry and no prior decision is being reversed — this is a forward
generalization of what PR 2418 landed as hardcoded, built on top of the
`dispatch` facade PR 2426 established.

## Decisions

- Work happens entirely inside `crates/extensions/rara-trading/src/anomaly/`.
  The public re-exports in `anomaly/mod.rs` — `evaluate`, `EVAL_WINDOW`,
  `AnomalySignal`, `AnomalyMetrics`, `Severity`, `AnomalyError`, `Result` — and
  the `evaluate(window, latest) -> Result<Option<AnomalySignal>>` signature stay
  unchanged so the `dispatch` facade consumer (`dispatch/pipeline.rs`,
  `anomaly::evaluate`) needs no edit. This issue is invisible from outside the
  module.
- Introduce a minimal `Signal` trait. One signal computes, from a prepared
  per-evaluation context, a structured `SignalOutput` carrying: a stable `name`
  (the identifier used in the inspectable trace and as the reason-fragment
  label), the signal's `value` (or `Option` when the window was too short to
  compute it, mirroring today's `Option<f64>` metrics), a `fired` flag, and the
  ability to format its own reason fragment. Prepare the shared inputs
  (`closes`, log-`returns`, split history/newest return, historical volumes,
  latest volume) **once** in `evaluate` and pass them to every signal — do not
  recompute per signal, and do not change what each signal reads.
- `evaluate` becomes: build the context, walk the builtin registry collecting
  each signal's `SignalOutput`, assemble the existing `AnomalyMetrics` from the
  collected outputs, then classify. The five signals move into trait impls (L1:
  window return, max drawdown, volume surge; L2: robust MAD z-score, BNS jump)
  that wrap the existing pure functions in `rules.rs` / `statistics.rs` — keep
  those pure functions and their unit tests as the computational core; the trait
  impls are thin adapters.
- Reason text and severity classification are preserved **byte-for-byte**. The
  `reason` string keeps the exact `"{severity_label} anomaly — {parts}"` shape
  with the same per-signal fragments (`drawdown {:.1}%`, `window return
  {:+.1}%`, `volume surge {:.1}x`, `robust z-score {:.1}`, `jump ratio {:.1}`)
  in the same order (drawdown, return, volume, z-score, jump — encoded as the
  registry's builtin order). Each fragment still appears only when that signal
  fired. Severity stays the current rule: `fired_count`, with the
  `CRITICAL_DRAWDOWN && fired_count >= 2` escalation keyed on the drawdown value.
- Severity classification stays concrete. This issue does **not** turn severity
  into a weighted / scored / factor-combination framework — the drawdown-aware
  critical escalation is domain logic that stays where it is. Generalizing
  severity into weights is explicitly a later layer-② concern, not this one.
- Keep the module minimal (dtolnay's "least surface" anchor): the trait carries
  only what the five signals plus "a sixth signal can plug in" genuinely need.
  Do **not** add speculative machinery — no factor metadata catalog, no
  per-signal weights, no config-driven signal selection, no signal-dependency
  graph. Every field on `SignalOutput` must be read by `evaluate`, the metrics
  assembly, the reason builder, or the classifier — a field nothing consumes is
  a hollow field (`docs/guides/anti-patterns.md`) and must be dropped. In
  particular, only add a signal `category` (L1/L2) tag if some behavior-
  preserving output actually reads it; otherwise omit it.
- Provide a seam so a signal registered into the registry demonstrably
  participates without editing the core loop: `evaluate` delegates to an internal
  evaluation over a passed-in registry (e.g. `evaluate_with(&registry, window,
  latest)`), and the public `evaluate` calls it with the builtin registry. This
  is the seam the extensibility acceptance test exercises; it is internal to the
  module (may be `pub(crate)` / `#[cfg(test)]`), not new public API.
- Thresholds and statistical constants remain Rust `const` next to their
  mechanism (`docs/guides/anti-patterns.md` mechanism-tuning rule). No new YAML
  knob. Errors stay `snafu`; 3+-field structs use `#[derive(bon::Builder)]`.
- Update `crates/extensions/rara-trading/AGENT.md` to document the signal
  registry (the `Signal` trait, the builtin registry, and the "implement + one
  line to register" extension path) since this changes how the anomaly module is
  structured and extended.

## Boundaries

### Allowed Changes
- **/crates/extensions/rara-trading/src/anomaly/**
- **/crates/extensions/rara-trading/AGENT.md
- **/specs/issue-2429-anomaly-signal-registry.spec.md

### Forbidden
- **/crates/extensions/rara-trading/src/dispatch/**
- **/crates/extensions/rara-trading/src/finance/registry.rs
- **/crates/extensions/rara-trading/src/lib.rs
- **/config.example.yaml
- **/web/**
- **/extension/**
- Do NOT change the public re-exports or the `evaluate` signature — the
  `dispatch` facade consumer must compile and behave identically with no edit.
- Do NOT alter the `AnomalyMetrics` fields, the `reason` string wording/order,
  or the `Severity` classification outcome for the existing five signals. This
  is a behavior-preserving generalization; the existing anomaly unit tests and
  the delivery/directive tests are the parity gate and must stay green
  unchanged.
- Do NOT add a new signal in this issue — the registry ships with exactly the
  five existing signals. The sixth signal is a later issue.
- Do NOT build a weight / scoring / factor-metadata / combination framework, a
  signal-dependency graph, or config-driven signal selection — that is
  speculative machinery this issue must not introduce.
- Do NOT add a YAML knob for any signal, threshold, or window size — mechanism
  tuning stays a Rust `const` (`docs/guides/anti-patterns.md`).
- Do NOT construct hollow outputs: a `SignalOutput` field nothing reads, or a
  trait impl that returns a constant regardless of input, is a hollow
  implementation and is forbidden.

## Acceptance Criteria

Scenario: A signal registered into the registry participates in evaluation without editing the core loop
  Test:
    Package: rara-trading
    Filter: registered_signal_participates_in_evaluation
  Given a registry of the five builtin signals plus one additional test signal that fires with a recognizable name
  And a candle window that the five builtin signals alone would leave unremarkable (evaluate returns None)
  When the evaluator runs over that window using the extended registry
  Then it now returns a signal because the added signal fired
    And the added signal's name/fragment appears in the collected output
    And running the same window through the builtin-only registry still returns None

Scenario: A multi-bar crash on a volume spike still produces the same high-severity signal
  Test:
    Package: rara-trading
    Filter: crash_window_produces_high_severity_anomaly_signal
  Given an ordered candle window ending in a sharp multi-bar decline with a volume spike
  When the registry-driven evaluator runs over the window and the newest closed candle
  Then it returns the highest (bypass-eligible) severity as before
    And the reason still names the drawdown and window-return magnitudes

Scenario: A flat, low-volatility tape still produces no anomaly signal
  Test:
    Package: rara-trading
    Filter: flat_tape_produces_no_anomaly_signal
  Given an ordered candle window of small alternating moves at steady volume
  When the registry-driven evaluator runs over the window and the newest closed candle
  Then it returns None, unchanged from before the registry migration

Scenario: Multiple signals firing without a deep drawdown still classify as elevated
  Test:
    Package: rara-trading
    Filter: two_rules_without_deep_drawdown_produce_elevated_severity
  Given a single-bar drop on a volume surge where several signals fire but the drawdown stays under the critical floor
  When the registry-driven evaluator collects the fired signals and classifies severity
  Then the severity is the middle (elevated) level, identical to the pre-registry classifier

Scenario: A produced signal still enriches the injected directive with its narrative
  Test:
    Package: rara-trading
    Filter: anomaly_signal_enriches_finance_directive
  Given a matched immediate BTCUSDT candle whose rolling window is crash-shaped
  When the dispatch facade evaluates the window and injects the synthetic directive
  Then the directive text contains the same anomaly severity and reason as before
    And a matched candle whose window is flat still yields the unchanged factual wording

Scenario: A non-positive candle close still fails with the typed error
  Test:
    Package: rara-trading
    Filter: non_positive_close_is_a_typed_error
  Given a candle window where the newest close is not strictly positive
  When the registry-driven evaluator prepares the shared context before walking the registry
  Then it returns the same NonPositivePrice typed error, unchanged by the migration

## Out of Scope

- Adding any new signal (realized-volatility spike, RSI divergence, etc.) — a
  later layer-② issue; the registry ships with the existing five only.
- Backtesting / historical replay of signals — later layer-② block.
- Trade suggestions / actionable recommendations — later layer-② block.
- Turning severity into a weighted or scored combination of signals, or any
  factor-metadata / signal-weighting framework.
- Per-signal or per-symbol operator-tunable config (YAML) — only if ever needed,
  in a later issue, through `config.example.yaml`.
- Any change to the delivery policy (cooldown / budget / severity bypass) in
  `finance/registry.rs` or to the `dispatch` facade.
- Any web / extension UI surface for signals.
