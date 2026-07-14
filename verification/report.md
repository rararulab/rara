# Verification report — issue #2417 (sub-minute market-candle polling floor)

- base_sha: `911eab9fab1ba0ce08fb08baccaa35190cc80857`
- head_sha: `f5c0a8405ad6043f1f358dde56b90e68a744401a`
- lane: 1
- score_authority: verifier
- implementer_evidence: self_check_only (not read)

## Scope verified
A rate-limit floor (`MIN_INTERVAL_SECS = 5`) added to `validate_transport` in
`crates/extensions/rara-trading/src/feed/market_candle.rs`, so a Binance
market-candle feed with `interval_secs >= 5` is accepted and one below the floor
is rejected at config-load with a message naming the minimum interval and the
per-IP request-weight rationale. Doc updates in `catalog.rs` +
`config.example.yaml`. Existing `interval_secs > 0` guard preserved.

## (a) Quality gate — clean state
- `git status --short` → empty (committed work only).
- `cargo +nightly fmt --all -- --check` → exit 0.
- `cargo check --all --all-targets` → exit 0.
- `cargo clippy --workspace --all-targets --all-features --no-deps -- -D warnings` → exit 0.
- `RUSTDOCFLAGS="-D warnings" cargo +nightly doc --workspace --no-deps --document-private-items` → exit 0.
- `cargo test -p rara-trading` → exit 0:
  - `interval_below_rate_limit_floor_is_rejected ... ok`
  - `subminute_interval_within_floor_is_accepted ... ok`
  - `test result: ok. 68 passed; 0 failed` (+ doctests `ok. 1 passed`)

Note: `prek` not installed on this host; ran the four hook commands directly per
the equivalent-command allowance.

## (b) Spec lifecycle
`just spec-lifecycle specs/issue-2417-subminute-candle-polling.spec.md` → exit 0:
```
Spec: issue-2417-subminute-candle-polling  stage: complete  passed: true
  [PASS] A 5-second monitoring interval is accepted by transport validation
  [PASS] An interval below the rate-limit floor is rejected at load
spec-lifecycle-guard: OK — every Test selector executed >=1 test
```

## (c) End-to-end drive of the runtime config-load surface
Guard reached at runtime via `POST /api/v1/data-feeds` → `create_feed` →
`normalize_active_feed_config` → `MarketCandleSource::normalize_config` →
`validate_transport`, error mapped to `ProblemDetails::bad_request`. Also on
activation via `start_feed_task` → `MarketCandleSource::from_config` →
`validate_transport`. Built-in catalog presets (60s/300s) stay above the floor.

Drove the real HTTP surface using the crate's in-tree axum router harness
(`app_with_user(Role::Admin)` + `tower::ServiceExt::oneshot`, in-memory diesel
pool, real `Principal`, bearer) via a throwaway probe test, run then removed
(tree confirmed clean, HEAD unchanged). Observed (verbatim `detail`):
- interval=5 → `201 Created`, feed persisted.
- interval=4 → `400 Bad Request`: names the minimum (5) AND the per-IP
  request-weight rate-limit rationale.
- interval=0 → `400 Bad Request`: existing "greater than zero" guard preserved.

## Transition matrix
- fail_to_pass: `subminute_interval_within_floor_is_accepted` (5s accepted),
  `interval_below_rate_limit_floor_is_rejected` (4s rejected w/ min+rationale).
  At base_sha the floor does not exist (only `> 0`), so 4s would be accepted.
- pass_to_fail: 0 — full `rara-trading` suite 68/68; existing validations intact.

## Probes (all via real HTTP `POST /api/v1/data-feeds`)
| # | Input | Expected | Observed | Verdict |
|---|---|---|---|---|
| 1 | interval_secs=5 | 201, persisted | 201 Created, stored | PASS |
| 2 | interval_secs=4 | 400, names min + rationale | 400, names 5 + rate limit | PASS |
| 3 | interval_secs=0 | 400 | 400 "greater than zero" | PASS |
| 4 | missing `interval_secs` | 400 | 400 "missing field" | PASS |
| 5 | CJK name + interval=4 | 400, floor message | 400, correct message | PASS |
| 6 | concurrent 5 and 4 | 201 / 400 independently | ok=201, bad=400 | PASS |

## Verdict
**PASS** — gate green from clean state, both BDD scenarios pass, config-load
acceptance/rejection drives correctly end-to-end through the real HTTP surface
(incl. CJK, empty/boundary, concurrent probes), `pass_to_fail = 0`.
