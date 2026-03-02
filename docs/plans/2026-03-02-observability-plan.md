# Observability 全链路接入 Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Connect rara to its deployed Grafana stack by adding a Prometheus `/metrics` endpoint, kernel Prometheus metrics, `#[instrument]` tracing on core paths, auto-OTLP in k8s, and 3 pre-provisioned Grafana dashboards.

**Architecture:** Add `/metrics` HTTP handler using the `prometheus` crate's TextEncoder. Create a `metrics.rs` module in kernel with LazyLock counters/gauges/histograms. Add `#[instrument]` to kernel event handlers, agent turn, and I/O pipeline. Provision Grafana dashboards via Helm ConfigMap with `grafana_dashboard: "1"` label.

**Tech Stack:** prometheus 0.14, tracing `#[instrument]`, Grafana JSON dashboard, Helm ConfigMap, kube-prometheus-stack sidecar

---

### Task 1: Add `/metrics` Prometheus endpoint

**Files:**
- Modify: `crates/server/Cargo.toml` — add `prometheus` dependency
- Modify: `crates/server/src/http.rs` — add `metrics_handler` + route

**Step 1: Add prometheus dependency to server crate**

In `crates/server/Cargo.toml`, add to `[dependencies]`:
```toml
prometheus = { workspace = true }
```

**Step 2: Add metrics handler and route**

In `crates/server/src/http.rs`, add the handler function (after `api_health_handler`):

```rust
/// Prometheus metrics endpoint — returns all registered metrics in text format.
async fn metrics_handler() -> impl IntoResponse {
    use prometheus::{Encoder, TextEncoder};
    let encoder = TextEncoder::new();
    let mut buffer = Vec::new();
    let metric_families = prometheus::gather();
    encoder.encode(&metric_families, &mut buffer).unwrap();
    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "text/plain; version=0.0.4; charset=utf-8")],
        buffer,
    )
}
```

Update `health_routes()` to include the metrics route:

```rust
pub fn health_routes(router: Router) -> Router {
    router
        .route("/api/v1/health", get(api_health_handler))
        .route("/api/health", get(api_health_handler))
        .route("/metrics", get(metrics_handler))
}
```

**Step 3: Verify compilation**

Run: `cargo check -p rara-server`
Expected: compiles without errors

**Step 4: Run existing server tests**

Run: `cargo test -p rara-server`
Expected: all existing tests pass (health_check tests still work)

**Step 5: Commit**

```bash
git add crates/server/Cargo.toml crates/server/src/http.rs
git commit -m "feat(server): add /metrics Prometheus endpoint (#449)"
```

---

### Task 2: Create kernel Prometheus metrics module

**Files:**
- Create: `crates/core/kernel/src/metrics.rs`
- Modify: `crates/core/kernel/src/lib.rs` — add `pub mod metrics;`

**Step 1: Create metrics module**

Create `crates/core/kernel/src/metrics.rs`:

```rust
//! Prometheus metrics for the kernel.
//!
//! All metrics use `LazyLock` for zero-cost static registration with the
//! global prometheus registry.

use std::sync::LazyLock;

use prometheus::{
    HistogramVec, IntCounterVec, IntGaugeVec,
    register_histogram_vec, register_int_counter_vec, register_int_gauge_vec,
};

// -- Process lifecycle -------------------------------------------------------

pub static PROCESS_SPAWNED: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "kernel_process_spawned_total",
        "Total agent processes spawned",
        &["agent_name"]
    )
    .unwrap()
});

pub static PROCESS_COMPLETED: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "kernel_process_completed_total",
        "Total agent processes completed",
        &["agent_name", "exit_state"]
    )
    .unwrap()
});

pub static PROCESS_ACTIVE: LazyLock<IntGaugeVec> = LazyLock::new(|| {
    register_int_gauge_vec!(
        "kernel_process_active",
        "Currently active agent processes",
        &["agent_name"]
    )
    .unwrap()
});

// -- LLM turn metrics --------------------------------------------------------

pub static TURN_TOTAL: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "kernel_turn_total",
        "Total LLM turns executed",
        &["agent_name", "model"]
    )
    .unwrap()
});

pub static TURN_DURATION_SECONDS: LazyLock<HistogramVec> = LazyLock::new(|| {
    register_histogram_vec!(
        "kernel_turn_duration_seconds",
        "LLM turn execution duration in seconds",
        &["agent_name", "model"],
        vec![0.5, 1.0, 2.0, 5.0, 10.0, 20.0, 30.0, 60.0, 120.0]
    )
    .unwrap()
});

pub static TURN_TOOL_CALLS: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "kernel_turn_tool_calls_total",
        "Total tool calls made during turns",
        &["agent_name", "tool_name"]
    )
    .unwrap()
});

pub static TURN_TOKENS_INPUT: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "kernel_turn_tokens_input_total",
        "Total input tokens consumed",
        &["model"]
    )
    .unwrap()
});

pub static TURN_TOKENS_OUTPUT: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "kernel_turn_tokens_output_total",
        "Total output tokens produced",
        &["model"]
    )
    .unwrap()
});

// -- Event processing --------------------------------------------------------

pub static EVENT_PROCESSED: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "kernel_event_processed_total",
        "Total events processed",
        &["event_type"]
    )
    .unwrap()
});

pub static SYSCALL_TOTAL: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "kernel_syscall_total",
        "Total syscalls processed",
        &["syscall_type"]
    )
    .unwrap()
});

// -- I/O pipeline ------------------------------------------------------------

pub static MESSAGE_INBOUND: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "kernel_message_inbound_total",
        "Total inbound messages received",
        &["channel_type"]
    )
    .unwrap()
});

pub static MESSAGE_OUTBOUND: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "kernel_message_outbound_total",
        "Total outbound messages delivered",
        &["channel_type"]
    )
    .unwrap()
});
```

**Step 2: Register module in lib.rs**

In `crates/core/kernel/src/lib.rs`, add after `pub mod memory;`:
```rust
pub mod metrics;
```

**Step 3: Verify compilation**

Run: `cargo check -p rara-kernel`
Expected: compiles without errors

**Step 4: Commit**

```bash
git add crates/core/kernel/src/metrics.rs crates/core/kernel/src/lib.rs
git commit -m "feat(kernel): add Prometheus metrics module (#449)"
```

---

### Task 3: Instrument event_loop.rs + emit metrics

**Files:**
- Modify: `crates/core/kernel/src/event_loop.rs`

**Step 1: Add metrics imports and instrument handle_event**

At the top of event_loop.rs, add to imports:
```rust
use crate::metrics;
```

In `handle_event()` (line 170), add metrics increment at the start of each match arm. Add an `event_type` variable and increment after dispatch:

```rust
pub(crate) async fn handle_event(&self, event: KernelEvent, runtimes: &RuntimeTable) {
    let event_type = event.variant_name();
    metrics::EVENT_PROCESSED
        .with_label_values(&[event_type])
        .inc();

    match event {
        // ... existing arms unchanged ...
    }
}
```

Note: `KernelEvent` already has a `variant_name()` method (used in handle_syscall at line 236).

**Step 2: Add metrics to handle_syscall**

In `handle_syscall()` (line 235), add after `let syscall_type = syscall.variant_name();`:
```rust
metrics::SYSCALL_TOTAL
    .with_label_values(&[syscall_type])
    .inc();
```

**Step 3: Add instrument to handle_spawn_agent + emit process metrics**

In `handle_spawn_agent()` (line 1285), add after the process is inserted into ProcessTable (find the `inner.process_table.insert(process);` line):

```rust
metrics::PROCESS_SPAWNED
    .with_label_values(&[&manifest.name])
    .inc();
metrics::PROCESS_ACTIVE
    .with_label_values(&[&manifest.name])
    .inc();
```

**Step 4: Add metrics to handle_turn_completed**

In `handle_turn_completed()`, in the `Ok(turn)` success arm (around line 1093), after existing metrics recording add:
```rust
metrics::TURN_TOTAL
    .with_label_values(&[&agent_name, &turn.model])
    .inc();
```

For the agent_name, extract it from process_table before the match:
```rust
let agent_name = self
    .inner()
    .process_table
    .get(agent_id)
    .map(|p| p.manifest_name.clone())
    .unwrap_or_else(|| "unknown".to_string());
```

**Step 5: Add metrics to cleanup_process (process completion)**

Find `cleanup_process` and add before runtime removal:
```rust
if let Some(process) = self.inner().process_table.get(agent_id) {
    metrics::PROCESS_ACTIVE
        .with_label_values(&[&process.manifest_name])
        .dec();
    metrics::PROCESS_COMPLETED
        .with_label_values(&[&process.manifest_name, &process.state.to_string()])
        .inc();
}
```

**Step 6: Verify compilation**

Run: `cargo check -p rara-kernel`
Expected: compiles without errors

**Step 7: Run kernel tests**

Run: `cargo test -p rara-kernel -- --test-threads=4`
Expected: all tests pass

**Step 8: Commit**

```bash
git add crates/core/kernel/src/event_loop.rs
git commit -m "feat(kernel): emit Prometheus metrics in event loop (#449)"
```

---

### Task 4: Instrument agent_turn.rs

**Files:**
- Modify: `crates/core/kernel/src/agent_turn.rs`

**Step 1: Add #[instrument] to run_inline_agent_loop**

The function is `pub(crate) async fn run_inline_agent_loop(...)` at line 121. Add `#[instrument]` above it:

```rust
#[tracing::instrument(
    skip(handle, history, stream_handle, turn_cancel),
    fields(
        agent_id = %handle.agent_id(),
        session_id = %handle.session_id(),
    )
)]
pub(crate) async fn run_inline_agent_loop(
```

**Step 2: Add iteration-level spans inside the loop**

Inside the main iteration loop (the `for iteration in 0..max_iterations` loop), wrap each iteration in a span:

```rust
let iter_span = tracing::info_span!(
    "llm_iteration",
    iteration = iteration,
    tool_calls = tracing::field::Empty,
);
let _iter_guard = iter_span.enter();
```

After tool calls are collected, record:
```rust
iter_span.record("tool_calls", pending_tool_calls.len());
```

**Step 3: Add tool execution span**

Around the tool execution section (parallel tool calls), add a span:
```rust
let tool_span = tracing::debug_span!("tool_execute", tool_name = %tc.name, tool_id = %tc.id);
let _tool_guard = tool_span.enter();
```

**Step 4: Verify compilation**

Run: `cargo check -p rara-kernel`
Expected: compiles without errors

**Step 5: Commit**

```bash
git add crates/core/kernel/src/agent_turn.rs
git commit -m "feat(kernel): instrument agent_turn with tracing spans (#449)"
```

---

### Task 5: Instrument I/O pipeline (ingress + egress + stream)

**Files:**
- Modify: `crates/core/kernel/src/io/ingress.rs`
- Modify: `crates/core/kernel/src/io/egress.rs`
- Modify: `crates/core/kernel/src/io/stream.rs`

**Step 1: Instrument IngressPipeline::ingest**

In `crates/core/kernel/src/io/ingress.rs`, add to the `ingest()` method (line 138):

```rust
#[tracing::instrument(
    skip(self, raw),
    fields(
        channel = ?raw.channel_type,
        platform_user = %raw.platform_user_id,
    )
)]
pub async fn ingest(&self, raw: RawPlatformMessage) -> Result<(), IngestError> {
```

Also add after successful publishing:
```rust
crate::metrics::MESSAGE_INBOUND
    .with_label_values(&[&format!("{:?}", raw.channel_type)])
    .inc();
```

Note: The `raw.channel_type` may need to be captured before it's consumed. Read the function to check if `raw` is moved. If so, capture channel type first:
```rust
let channel_label = format!("{:?}", raw.channel_type);
// ... existing logic ...
crate::metrics::MESSAGE_INBOUND
    .with_label_values(&[&channel_label])
    .inc();
```

**Step 2: Instrument Egress::deliver**

In `crates/core/kernel/src/io/egress.rs`, add to `deliver()` (line 237):

```rust
#[tracing::instrument(
    skip(adapters, endpoints, envelope),
    fields(
        user_id = %envelope.user.0,
        session_id = %envelope.session_id.0,
        payload_type = ?std::mem::discriminant(&envelope.payload),
    )
)]
pub async fn deliver(
```

After successful delivery to each endpoint, increment:
```rust
crate::metrics::MESSAGE_OUTBOUND
    .with_label_values(&[&format!("{:?}", endpoint.channel_type)])
    .inc();
```

**Step 3: Instrument StreamHub::open and close**

In `crates/core/kernel/src/io/stream.rs`:

For `open()` (line 154):
```rust
#[tracing::instrument(skip(self), fields(stream_id = tracing::field::Empty))]
pub fn open(&self, session_id: SessionId) -> StreamHandle {
```
After creating the stream_id, record it:
```rust
tracing::Span::current().record("stream_id", &stream_id.0.as_str());
```

For `close()` (line 169):
```rust
#[tracing::instrument(skip(self))]
pub fn close(&self, stream_id: &StreamId) {
```

**Step 4: Verify compilation**

Run: `cargo check -p rara-kernel`
Expected: compiles without errors

**Step 5: Commit**

```bash
git add crates/core/kernel/src/io/ingress.rs crates/core/kernel/src/io/egress.rs crates/core/kernel/src/io/stream.rs
git commit -m "feat(kernel): instrument I/O pipeline with tracing spans (#449)"
```

---

### Task 6: Instrument ProcessTable and AgentRegistry

**Files:**
- Modify: `crates/core/kernel/src/process/mod.rs`
- Modify: `crates/core/kernel/src/process/agent_registry.rs`

**Step 1: Instrument ProcessTable key methods**

In `crates/core/kernel/src/process/mod.rs`:

For `insert()` (line 612):
```rust
#[tracing::instrument(skip(self, process), fields(agent_id = %process.agent_id, agent_name = %process.manifest_name))]
pub fn insert(&self, process: AgentProcess) {
```

For `remove()` (line 680):
```rust
#[tracing::instrument(skip(self))]
pub fn remove(&self, id: AgentId) -> Option<AgentProcess> {
```

For `set_state()` (line 638):
```rust
#[tracing::instrument(skip(self), fields(new_state = %state))]
pub fn set_state(&self, id: AgentId, state: ProcessState) -> Result<()> {
```

**Step 2: Instrument AgentRegistry key methods**

In `crates/core/kernel/src/process/agent_registry.rs`:

For `get()` (line 46):
```rust
#[tracing::instrument(skip(self))]
pub fn get(&self, name: &str) -> Option<AgentManifest> {
```

For `register()` (line 62):
```rust
#[tracing::instrument(skip(self, manifest), fields(agent_name = %manifest.name))]
pub fn register(&self, manifest: AgentManifest) -> Result<()> {
```

**Step 3: Verify compilation**

Run: `cargo check -p rara-kernel`
Expected: compiles without errors

**Step 4: Run all kernel tests**

Run: `cargo test -p rara-kernel -- --test-threads=4`
Expected: all tests pass

**Step 5: Commit**

```bash
git add crates/core/kernel/src/process/mod.rs crates/core/kernel/src/process/agent_registry.rs
git commit -m "feat(kernel): instrument ProcessTable and AgentRegistry (#449)"
```

---

### Task 7: Auto-enable OTLP in Kubernetes environment

**Files:**
- Modify: `crates/cmd/src/main.rs`

**Step 1: Add k8s auto-detection to ServerArgs::run**

In `crates/cmd/src/main.rs`, in `ServerArgs::run()`, modify the OTLP fallback chain (around line 69-91).

Replace the final `else` branch:
```rust
} else if std::env::var("KUBERNETES_SERVICE_HOST").is_ok() {
    // Running in Kubernetes — auto-connect to Alloy OTLP collector.
    use common_telemetry::logging::{LoggingOptions, OtlpExportProtocol};
    tracing::info!("Kubernetes detected — auto-enabling OTLP tracing to Alloy");
    LoggingOptions {
        enable_otlp_tracing: true,
        otlp_endpoint: Some("http://rara-infra-alloy:4318/v1/traces".to_string()),
        otlp_export_protocol: Some(OtlpExportProtocol::Http),
        log_format: common_telemetry::logging::LogFormat::Json,
        ..Default::default()
    }
} else {
    common_telemetry::logging::LoggingOptions::default()
};
```

**Step 2: Verify compilation**

Run: `cargo check -p rara-cmd`
Expected: compiles without errors

**Step 3: Commit**

```bash
git add crates/cmd/src/main.rs
git commit -m "feat(cmd): auto-enable OTLP tracing in Kubernetes environment (#449)"
```

---

### Task 8: Grafana dashboard provisioning infrastructure

**Files:**
- Modify: `deploy/helm/rara-infra/values.yaml` — enable dashboard sidecar
- Create: `deploy/helm/rara-infra/templates/dashboards/configmaps.yaml` — ConfigMap template

**Step 1: Enable dashboard sidecar in values.yaml**

In `deploy/helm/rara-infra/values.yaml`, change the `sidecar` section under `kube-prometheus-stack.grafana`:

```yaml
    sidecar:
      datasources:
        enabled: true
      dashboards:
        enabled: true
        label: grafana_dashboard
        labelValue: "1"
        searchNamespace: ALL
```

**Step 2: Create ConfigMap template for dashboards**

Create directory and file `deploy/helm/rara-infra/templates/dashboards/configmaps.yaml`:

```yaml
{{- if (index .Values "kube-prometheus-stack" "grafana" "sidecar" "dashboards" "enabled") }}
{{- range $path, $_ := .Files.Glob "dashboards/*.json" }}
---
apiVersion: v1
kind: ConfigMap
metadata:
  name: {{ include "rara-infra.fullname" $ }}-dashboard-{{ base $path | trimSuffix ".json" }}
  namespace: {{ $.Release.Namespace }}
  labels:
    {{- include "rara-infra.labels" $ | nindent 4 }}
    grafana_dashboard: "1"
data:
  {{ base $path }}: |-
{{ $.Files.Get $path | indent 4 }}
{{- end }}
{{- end }}
```

**Step 3: Commit**

```bash
mkdir -p deploy/helm/rara-infra/templates/dashboards
git add deploy/helm/rara-infra/values.yaml deploy/helm/rara-infra/templates/dashboards/configmaps.yaml
git commit -m "feat(helm): enable Grafana dashboard sidecar provisioning (#449)"
```

---

### Task 9: Create Grafana dashboard JSON — Rara Overview (Golden Signals)

**Files:**
- Create: `deploy/helm/rara-infra/dashboards/rara-overview.json`

**Step 1: Create the overview dashboard**

Create `deploy/helm/rara-infra/dashboards/rara-overview.json` with these panels:

1. **HTTP Request Rate** — `rate(http_server_request_duration_seconds_count[5m])` by path, method
2. **HTTP Error Rate** — `rate(http_server_request_duration_seconds_count{http_response_status_code=~"4..|5.."}[5m])`
3. **HTTP Latency p50/p95/p99** — `histogram_quantile(0.95, rate(http_server_request_duration_seconds_bucket[5m]))`
4. **Active Agent Processes** — `kernel_process_active` by agent_name
5. **Inbound Message Rate** — `rate(kernel_message_inbound_total[5m])` by channel_type
6. **Outbound Message Rate** — `rate(kernel_message_outbound_total[5m])` by channel_type
7. **Event Processing Rate** — `rate(kernel_event_processed_total[5m])` by event_type
8. **Syscall Rate** — `rate(kernel_syscall_total[5m])` by syscall_type

The JSON should follow standard Grafana dashboard JSON format with:
- `"uid": "rara-overview"`
- `"title": "Rara Overview"`
- `"tags": ["rara"]`
- Prometheus datasource `"uid": "prometheus"`
- Time range: last 1 hour, refresh 10s
- Rows: 2x4 grid layout

**Step 2: Commit**

```bash
git add deploy/helm/rara-infra/dashboards/rara-overview.json
git commit -m "feat(helm): add Rara Overview Grafana dashboard (#449)"
```

---

### Task 10: Create Grafana dashboard JSON — Agent Topology

**Files:**
- Create: `deploy/helm/rara-infra/dashboards/rara-agents.json`

**Step 1: Create the agents dashboard**

Panels:
1. **Process Spawn Rate** — `rate(kernel_process_spawned_total[5m])` by agent_name
2. **Process Completion Rate** — `rate(kernel_process_completed_total[5m])` by agent_name, exit_state
3. **Turn Execution Time** — `histogram_quantile(0.95, rate(kernel_turn_duration_seconds_bucket[5m]))` by agent_name
4. **Turn Count Rate** — `rate(kernel_turn_total[5m])` by agent_name, model
5. **Tool Call Distribution** — `topk(10, sum by (tool_name) (rate(kernel_turn_tool_calls_total[5m])))`
6. **Input Tokens Rate** — `rate(kernel_turn_tokens_input_total[5m])` by model
7. **Output Tokens Rate** — `rate(kernel_turn_tokens_output_total[5m])` by model
8. **Cumulative Tokens** — `sum(kernel_turn_tokens_input_total) + sum(kernel_turn_tokens_output_total)`

Dashboard metadata:
- `"uid": "rara-agents"`
- `"title": "Rara Agents"`
- `"tags": ["rara", "agents"]`

**Step 2: Commit**

```bash
git add deploy/helm/rara-infra/dashboards/rara-agents.json
git commit -m "feat(helm): add Rara Agents Grafana dashboard (#449)"
```

---

### Task 11: Create Grafana dashboard JSON — Workers

**Files:**
- Create: `deploy/helm/rara-infra/dashboards/rara-workers.json`

**Step 1: Create the workers dashboard**

Uses existing worker metrics from `crates/common/worker/src/metrics.rs`:

Panels:
1. **Worker Execution Rate** — `rate(worker_executions_total[5m])` by worker
2. **Worker Error Rate** — `rate(worker_errors_total[5m])` by worker
3. **Worker Execution Duration** — `histogram_quantile(0.95, rate(worker_execution_duration_seconds_bucket[5m]))` by worker
4. **Worker Active Status** — `worker_active` by worker (stat panel: 1=active, 0=stopped)
5. **Worker Start/Stop Events** — `rate(worker_started_total[5m])`, `rate(worker_stopped_total[5m])`
6. **Worker Pause/Resume** — `rate(worker_paused_total[5m])`, `rate(worker_resumed_total[5m])`

Dashboard metadata:
- `"uid": "rara-workers"`
- `"title": "Rara Workers"`
- `"tags": ["rara", "workers"]`

**Step 2: Commit**

```bash
git add deploy/helm/rara-infra/dashboards/rara-workers.json
git commit -m "feat(helm): add Rara Workers Grafana dashboard (#449)"
```

---

### Task 12: Final verification and cleanup

**Step 1: Full cargo check**

Run: `cargo check`
Expected: entire workspace compiles

**Step 2: Run all kernel tests**

Run: `cargo test -p rara-kernel -- --test-threads=4`
Expected: all tests pass

**Step 3: Run server tests**

Run: `cargo test -p rara-server`
Expected: all tests pass

**Step 4: Check frontend build (unchanged, but verify)**

Run: `cd web && npm run build`
Expected: builds successfully

**Step 5: Helm template validation**

Run: `cd deploy/helm && helm template rara-infra ./rara-infra --debug 2>&1 | grep -c "grafana_dashboard"`
Expected: output shows 3 (one per dashboard ConfigMap)

**Step 6: Final commit if any fixes needed**

If any fixes were required, commit them.

---

## Key Reference Files

| File | Purpose |
|------|---------|
| `crates/common/worker/src/metrics.rs` | Existing Prometheus metrics pattern (LazyLock) |
| `crates/core/kernel/src/unified_event.rs` | `KernelEvent::variant_name()` method |
| `crates/core/kernel/src/event_loop.rs:170` | `handle_event()` dispatch |
| `crates/core/kernel/src/event_loop.rs:235` | `handle_syscall()` |
| `crates/core/kernel/src/event_loop.rs:615` | `handle_user_message()` — already has info_span |
| `crates/core/kernel/src/event_loop.rs:824` | `start_llm_turn()` — already has info_span |
| `crates/core/kernel/src/event_loop.rs:1050` | `handle_turn_completed()` — already has info_span |
| `crates/core/kernel/src/event_loop.rs:1285` | `handle_spawn_agent()` |
| `crates/core/kernel/src/agent_turn.rs:121` | `run_inline_agent_loop()` |
| `crates/core/kernel/src/io/ingress.rs:138` | `IngressPipeline::ingest()` |
| `crates/core/kernel/src/io/egress.rs:237` | `Egress::deliver()` |
| `crates/core/kernel/src/io/stream.rs:154` | `StreamHub::open()` |
| `deploy/helm/rara-infra/values.yaml` | Helm values for Grafana config |
| `deploy/helm/rara-infra/templates/_helpers.tpl` | Helm template helpers |

## Notes for implementer

- `KernelEvent` already has `variant_name()` returning `&'static str` — use this for event_type labels
- `Syscall` already has `variant_name()` — used in existing debug_span in handle_syscall
- Many handler functions already have manual `info_span!` / `debug_span!` — don't duplicate, add `#[instrument]` only where there's no existing span, OR add metrics increments inside existing spans
- `AgentProcess` has `manifest_name: String` field — use this for agent_name labels
- The `prometheus` crate is already a workspace dependency at version 0.14
- Dashboard JSON files can be large — keep them minimal with only the panels listed
- Use `"datasource": {"type": "prometheus", "uid": "prometheus"}` in dashboard JSON — this is the default Prometheus datasource UID in kube-prometheus-stack
