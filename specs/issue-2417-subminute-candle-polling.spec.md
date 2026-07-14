spec: task
name: "issue-2417-subminute-candle-polling"
inherits: project
tags: [enhancement, backend, trading]
---

## Intent

The built-in Binance `market_candle` source polls `/api/v3/klines` on a fixed
cadence driven by `MarketCandleTransport.interval_secs`
(`crates/extensions/rara-trading/src/feed/market_candle.rs`). For real-time
black-swan detection the monitored streams need a sub-minute cadence (5–10s)
so a crash is observed within seconds, not up to a minute late. Reusing the
existing REST polling loop is the intended path — WebSocket ingestion is a
later phase and explicitly out of scope here.

`interval_secs` is already a free per-feed field, but `validate_transport`
only enforces `interval_secs > 0`. That is the wrong guard for two reasons:
(1) nothing documents or blesses the 5–10s operating point the MVP needs, and
(2) there is no floor stopping a config from polling at, say, 1s across many
`symbols × timeframes`, which fans out to a burst of klines calls per tick and
will trip Binance's per-IP request-weight limit and get the deployment
temporarily banned. The MVP needs sub-minute polling to be *enabled and safe*,
not merely *not rejected*.

If we do not do this, one of two concrete bugs appears. Reproducer A
(latency): the operator leaves `interval_secs: 60`; a flash crash that starts
and bottoms within 40s is only observed on the next minute boundary, after the
worst of the move — the anomaly engine (issue 2415) evaluates a candle that
already reflects a recovered or fully-collapsed price, defeating "timely
feedback". Reproducer B (ban): the operator, wanting speed, sets
`interval_secs: 1` with 15 symbols × 2 timeframes; each tick issues 30 klines
requests, ~1800/min, exceeding Binance's request-weight budget; the venue
returns HTTP 429/418 and the feed flips to `Error` (via the existing
`record_error` path) — rara goes blind on every symbol at once, the opposite of
resilient monitoring.

This advances `goal.md` signal 2 (surfacing the right thing *at the right
time* — timeliness is half of "the right time") and signal 1 ("runs for months
without intervention" — a rate-limit-aware floor keeps the feed from
self-inflicting a ban). It crosses no "What rara is NOT" line: single-surface
depth on the existing crypto feed, reusing the existing transport.

Prior-art search (2026-07-14): the market-candle transport and its
`validate_transport` guards came from the finance-feed lineage (PRs 2223, 2243,
2250 default presets, 2270 config normalization, 2276 timestamp validation).
No prior PR added or removed an interval floor / rate-limit guard, and none
set a minimum cadence that this issue would be reversing. `git log --all
--grep "interval|poll"` since 180 days shows no removed minimum-interval logic.
So this is a first-time guard, not a regression-decision reversal.

## Decisions

- Keep the existing REST polling loop and `interval_secs` field. This issue
  does not add a new transport or a WebSocket path.
- Add a minimum-interval floor to `validate_transport`: reject an
  `interval_secs` below the floor with a clear `snafu`/`anyhow` boundary error
  message, so a reckless sub-floor config fails fast at load instead of getting
  the deployment IP-banned at runtime. The floor is chosen to keep the
  worst-case fan-out (`symbols × timeframes` requests per tick) within Binance's
  documented per-IP request-weight budget at the intended 5–10s operating point.
- The floor value is a **mechanism constant** (Rust `const` next to the
  transport validation), NOT a YAML knob — it encodes a fixed property of the
  Binance public API's rate limit, which no deploy operator has a principled
  reason to raise past what the venue allows
  (`docs/guides/anti-patterns.md`). `interval_secs` itself stays a per-feed
  config value in `config.example.yaml` (a genuine deploy-relevant choice
  *within* the allowed range).
- Update `config.example.yaml`'s commented market-candle example and any
  built-in preset in `crates/extensions/rara-trading/src/feed/catalog.rs` to
  document the supported 5–10s monitoring cadence and the floor, so operators
  see the safe operating point instead of guessing.
- No change to event shape, dedupe, or downstream matching — only cadence and
  its validation.

## Boundaries

### Allowed Changes
- **/crates/extensions/rara-trading/src/feed/market_candle.rs
- **/crates/extensions/rara-trading/src/feed/catalog.rs
- **/config.example.yaml
- **/specs/issue-2417-subminute-candle-polling.spec.md

### Forbidden
- **/crates/extensions/rara-trading/src/anomaly/**
- **/crates/extensions/rara-trading/src/finance/registry.rs
- **/crates/app/src/finance_event.rs
- **/web/**
- **/extension/**
- Do NOT introduce WebSocket ingestion or a new transport type — reuse the
  existing REST poll loop.
- Do NOT expose the rate-limit floor as a YAML knob — it is a fixed property of
  the venue API, a mechanism const (`docs/guides/anti-patterns.md`).
- Do NOT change candle event shape, tags, dedupe, or the anomaly / delivery
  paths (issues 2415 / 2416).
- Do NOT remove the existing `interval_secs > 0` and other transport
  validations — the floor is added alongside them.

## Acceptance Criteria

Scenario: A 5-second monitoring interval is accepted by transport validation
  Test:
    Package: rara-trading
    Filter: subminute_interval_within_floor_is_accepted
  Given a Binance market-candle transport config with interval_secs at the intended monitoring cadence (e.g. 5s) and a small symbol set
  When the transport is normalized and validated
  Then validation succeeds

Scenario: An interval below the rate-limit floor is rejected at load
  Test:
    Package: rara-trading
    Filter: interval_below_rate_limit_floor_is_rejected
  Given a Binance market-candle transport config whose interval_secs is below the floor
  When the transport is validated
  Then validation fails with an error naming the minimum interval
    And the message explains the rate-limit rationale

## Out of Scope

- WebSocket ingestion (a later phase).
- The anomaly evaluation engine — issue 2415.
- Severity-graded delivery — issue 2416.
- Per-symbol adaptive cadence or dynamic backoff on 429s beyond the existing
  `record_error` reporting.
- Non-Binance venues and non-crypto assets.
