spec: task
name: "issue-1982-otlp-exporter-async-runtime"
inherits: project
tags: ["telemetry", "otlp", "tokio"]
---

## Intent

`rara server` panics on startup before reaching READY whenever
`telemetry.otlp.enabled: true` (the current remote config on
raratekiAir). `rara gateway` exhausts its three restart attempts and
exits, so `just run` is unusable until the OTLP path is rolled back.

The panic is:

```
thread 'main' panicked at .../tokio-1.50.0/src/runtime/blocking/shutdown.rs:51:
Cannot drop a runtime in a context where blocking is not allowed.
```

The trigger is the helper introduced in PR 1962 / commit a048dbc5,
`crates/common/telemetry/src/logging.rs:763`:

```rust
fn build_otlp_http_client() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .no_proxy()
        .build()
        ...
}
```

`reqwest::blocking::Client::build()` constructs and immediately drops a
temporary tokio Runtime on the calling thread. PR 1962 calls this helper
synchronously from `init_logging` (via `build_otlp_exporter`,
`init_meter_provider`, and `init_logger_provider`), which all run inside
the outer `#[tokio::main]` async context of `rara server`. Dropping a
Runtime while another Runtime is current panics in tokio 1.x — hence
the crash on the very first OTLP-related call.

This is the third revision in this area in two months and the prior-art
chain matters: PR 1931 introduced async `reqwest::Client` injection so
the OTLP exporter could enforce `.no_proxy()` (rara routes Telegram
traffic through an outbound HTTP proxy that must be bypassed for
self-hosted Loki / Langfuse / OTLP collector). Issue 1960 / PR 1962
flipped to `reqwest::blocking::Client` because OTel's
`BatchSpanProcessor` / `BatchLogProcessor` / `PeriodicReader` run on
dedicated OS threads with no tokio runtime, so async reqwest panicked
there with "no reactor running". The blocking direction is correct for
those worker threads, but the construction site (inside `#[tokio::main]`)
is wrong, and PR 1962's manual test "Boot rara on local-rara" was left
unchecked in the test plan — that is the one box that would have caught
this.

### Direction (option C — upstream-canonical, locked by user)

The first draft of this spec proposed reverting to async
`reqwest::Client` and binding OTel batch processors to the tokio runtime
via `rt-tokio` (option 2). That premise does not hold for
`opentelemetry_sdk` 0.31's standard processors: `rt-tokio` only enables
`runtime::Tokio` for the experimental `*_with_async_runtime::*` types,
which are gated behind separate `experimental_*` Cargo features.
Standard `BatchSpanProcessor` / `BatchLogProcessor` / `PeriodicReader`
always run on dedicated `std::thread` OS threads with
`futures_executor::block_on`, regardless of whether `rt-tokio` is
enabled. There is no public 0.31 API that binds the standard processors
to a tokio runtime without flipping experimental features.

A research agent surveyed 6+ OSS Rust projects shipping
`opentelemetry-otlp` over HTTP and identified the patterns:

- **Pattern 3** (`reqwest::blocking::Client` + standard BSP /
  `PeriodicReader` / log batch processor): used by
  **opentelemetry-rust's own upstream example**
  (`opentelemetry-otlp/examples/basic-otlp-http/`) and by
  `ZcashFoundation/zebra` in production (with explanatory source
  comment: "This works without an async runtime because:
  1. reqwest-blocking-client doesn't need tokio
  2. BatchSpanProcessor spawns its own background thread"). 3 projects
  total. **This is the upstream-blessed path.**
- Pattern 2 (async reqwest + experimental async-runtime BSP): 4 projects
  (spin, openobserve, fluree, casper). Locks us into `experimental_*`
  features.
- Pattern 1 (async reqwest + `Handle::block_on` bridging adapter):
  **0 OSS adopters**. Folklore.
- Pattern 5 (custom non-`reqwest` `HttpClient` impl): deno (hyper),
  sqlpage (awc). Bespoke and out of scope.

PR 1962's *direction* — `reqwest::blocking::Client` + standard BSP — is
already pattern 3 and matches upstream. The bug is purely in the
**construction site**: `reqwest::blocking::Client::builder().build()`
is called from inside `#[tokio::main]`, where dropping the temporary
tokio Runtime that `reqwest::blocking` creates internally panics.

The fix keeps `reqwest::blocking::Client` + standard processors (no
feature flips, no `experimental_*` gates, no custom `HttpClient`) and
moves the construction off the async context. Two viable mechanisms;
the implementer picks the cleaner one against the actual call graph in
`crates/app/`:

1. **Preferred**: invoke `init_logging` synchronously **before** the
   tokio runtime is constructed in `fn main()`. If the binary uses
   `#[tokio::main]`, switch to a manually-constructed
   `tokio::runtime::Runtime` so `init_logging` can run before
   `Runtime::new()` returns. This matches normal production Rust
   practice (subscriber installed before the first trace event = before
   the runtime).
2. **Fallback** (only if `init_logging` genuinely cannot run
   pre-runtime due to dependencies surfaced during implementation):
   wrap each `reqwest::blocking::Client::builder().build()` call inside
   `tokio::task::spawn_blocking(...).await` so the temp-runtime drop
   happens off the async context.

The implementer investigates the call graph and picks option 1 if
feasible. Either way, no Cargo feature flips, no nested `Runtime::new()`
inside async code, no custom `HttpClient` adapter.

### Reproducer for "what bug appears if we don't do this"

1. SSH to `local-rara`, `cd ~/code/rararulab/rara`, `just run`.
2. The current `config.yaml` has `telemetry.otlp.enabled: true`.
3. `rara gateway` spawns `rara server`. `init_logging` reaches
   `build_otlp_http_client()` from inside `#[tokio::main]`.
4. `reqwest::blocking::Client::builder().build()` constructs a temporary
   tokio Runtime and drops it on the current thread, which already has
   an outer Runtime — tokio panics with "Cannot drop a runtime in a
   context where blocking is not allowed."
5. `rara server` exits before emitting READY. Gateway supervisor
   restarts it; same panic. After three attempts, gateway exits.
6. Observed: `just run` produces the exact log the user pasted; remote
   is unusable until OTLP is disabled or the binary is rolled back.

### Goal alignment

Signal 1 ("the process runs for months without intervention"). A
gateway that crash-loops on every cold start is the exact opposite of
that signal — and right now it does not even take "months", it takes
one boot. Signal 4 ("every action is inspectable") — this is the
telemetry path itself; without it, no traces, no metrics, no logs reach
Langfuse / Loki / the OTLP collector, so every downstream eval and
replay is blind. Does not cross any "What rara is NOT" line.

Hermes positioning: not applicable — telemetry plumbing is internal
infrastructure, not a user-facing feature.

### Prior art

- opentelemetry-rust upstream HTTP example (pattern 3, the canonical
  recipe):
  https://github.com/open-telemetry/opentelemetry-rust/tree/main/opentelemetry-otlp/examples/basic-otlp-http
- ZcashFoundation/zebra production usage of the same pattern, with the
  load-bearing source comment about why no async runtime is needed:
  https://github.com/ZcashFoundation/zebra/blob/main/zebrad/src/components/tracing/otel.rs
- Issue 1960 / PR 1962 (the immediate cause of the regression — flipped
  to `reqwest::blocking::Client` for the right reason, panicked because
  it was built inside `#[tokio::main]`).
- PR 1931 (introduced `.no_proxy()` enforcement on the OTLP client;
  must be preserved across this change).
- PRs 1855 / 1857 / 1928 / 1949 / 1952 (full chain of telemetry work
  in this area).

`rg "reqwest::blocking|build_otlp|reqwest-blocking-client|reqwest-client"`
in tree confirms the helper lives only at
`crates/common/telemetry/src/logging.rs:763` and is called from three
sites (`build_otlp_exporter` 770, `init_meter_provider` 851,
`init_logger_provider` 894).

This direction does NOT reverse the `.no_proxy()` decision from
PR 1931 (preserved on the blocking client) and does NOT reverse the
"batch processors must not panic with no reactor running" decision
from PR 1962 — pattern 3 satisfies it by keeping the standard
processors on their own OS threads with the synchronous client, exactly
as upstream documents.

## Decisions

1. Keep `reqwest::blocking::Client` (do **not** flip back to async
   `reqwest::Client`). `.no_proxy()` is preserved on the blocking
   builder.
2. Keep `crates/common/telemetry/Cargo.toml` features as PR 1962 left
   them: `opentelemetry-otlp` uses `reqwest-blocking-client`; `reqwest`
   keeps its `blocking` feature. Do **not** enable any
   `experimental_*` feature on `opentelemetry_sdk`. `rt-tokio` may stay
   in the feature list (harmless under standard processors) or be
   removed if it is unused — implementer's call against `cargo check`.
3. Keep the standard `BatchSpanProcessor` / `BatchLogProcessor` /
   `PeriodicReader` (they run on dedicated OS threads with
   `futures_executor::block_on`; that is the upstream-canonical
   arrangement under pattern 3). Do not switch to
   `*_with_async_runtime::*`.
4. Fix the **construction site**, not the client choice. Preferred:
   invoke `init_logging` synchronously before the tokio runtime is
   constructed in the binary's `fn main()` (switch from
   `#[tokio::main]` to a manually-constructed
   `tokio::runtime::Runtime` if needed). Fallback: wrap each
   `reqwest::blocking::Client::builder().build()` call in
   `tokio::task::spawn_blocking(...).await`. Implementer picks against
   the actual call graph in `crates/app/`.
5. Do not introduce a custom `HttpClient` impl (no hyper, no awc, no
   bespoke adapter). Pattern 3 is sufficient.
6. No new YAML config keys. The construction-site fix is a mechanism
   choice — operators have no deployment-relevant reason to tune it.

## Boundaries

### Allowed Changes

- `**/crates/common/telemetry/src/logging.rs`
- `**/crates/common/telemetry/Cargo.toml`
- `**/crates/common/telemetry/tests/**`
- `**/crates/cmd/src/main.rs`
- `**/Cargo.lock`
- `**/specs/issue-1982-otlp-exporter-async-runtime.spec.md`

### Forbidden

- `**/config.example.yaml`
- `**/config.yaml`
- `**/crates/common/telemetry/src/lib.rs`
- `**/crates/common/telemetry/src/metrics.rs`
- `**/crates/common/telemetry/src/tracing.rs`
- `**/crates/kernel/**`
- `**/web/**`

## Completion Criteria

Scenario: telemetry init does not panic when OTLP is enabled
  Given an `init_logging` call exercised the way the production binary
    invokes it (synchronous, before the tokio runtime is current — or
    via `spawn_blocking` if the fallback mechanism was chosen)
    with `LoggingOptions { otlp_enabled: true, otlp_endpoint: "http://127.0.0.1:1/", ..default }`
  When the function returns
  Then it returns `Ok(())` (or the project's normal logging-init result)
    without panicking
  And no thread in the test process has panicked with
    `"Cannot drop a runtime in a context where blocking is not allowed"`
  Test:
    Package: common-telemetry
    Filter: otlp_init_does_not_panic_from_production_codepath

Scenario: OTLP HTTP client preserves no_proxy
  Given the helper `build_otlp_http_client()`
  When the resulting `reqwest::blocking::Client` is inspected via its
    public surface (issue a request to a URL whose `HTTPS_PROXY` would
    otherwise route it through a captive proxy address)
  Then the request is attempted directly, not through the env-var proxy
    (assertion can be done by setting `HTTPS_PROXY=http://127.0.0.1:1`
    in the test, expecting a connection failure to the actual target's
    DNS/IP rather than to `127.0.0.1:1`)
  Test:
    Package: common-telemetry
    Filter: otlp_http_client_bypasses_env_proxy

Scenario: batch exporter delivers spans via the blocking client
  Given a `tracing` span emitted after `init_logging` initialized the
    OTLP trace pipeline against a local stub HTTP server (the standard
    `BatchSpanProcessor` runs on its own OS thread, intentionally not
    in any tokio runtime — this is pattern 3)
  When the BatchSpanProcessor flushes
  Then the stub HTTP server receives at least one POST to the OTLP
    traces path within 5 seconds
  And no panic occurs in the test process (specifically, no
    `"Cannot drop a runtime in a context where blocking is not allowed"`
    and no `"there is no reactor running"`)
  Test:
    Package: common-telemetry
    Filter: otlp_trace_export_round_trip_via_blocking_client

## Constraints

- All comments and identifiers in new code must be English (project
  rule).
- No new YAML config keys (mechanism vs config rule, see
  `feedback_mechanism_vs_config.md`).
- Preserve `.no_proxy()` on the blocking client — do not regress
  PR 1931's fix during the construction-site rewrite.
- Do not enable any `experimental_*` feature on `opentelemetry_sdk`.
  The standard processors plus `reqwest::blocking::Client` is
  upstream-canonical (pattern 3).
- Do not introduce a custom `HttpClient` impl. Stay on
  `reqwest-blocking-client`.
- Do not introduce a nested `tokio::runtime::Runtime::new()` inside
  async code. The "manually-constructed runtime in `fn main()`"
  mechanism is fine because it is the *only* runtime, constructed
  before any async context exists.
- Pin the construction-site fix against the version already in
  `Cargo.toml` (opentelemetry_sdk 0.31). Do not bump the dependency to
  reach for `*_with_async_runtime::*` — that is explicitly out of
  scope.

## Out of Scope

- Bumping `opentelemetry`, `opentelemetry-otlp`, or `opentelemetry_sdk`
  versions.
- Adding new OTLP exporters (e.g., a different trace backend).
- Switching to `*_with_async_runtime::*` processors or any
  `experimental_*` feature.
- Refactoring `init_logging`'s overall structure beyond what is
  required to move the construction site off the async context.
- Changing the `.no_proxy()` policy itself.
- Touching the gateway supervisor's restart-budget logic — even though
  the symptom involved gateway giving up after three attempts, the
  gateway behavior is correct; the bug is in the child.
