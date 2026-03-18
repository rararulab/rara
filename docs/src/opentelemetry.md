# Observability

Rara uses OpenTelemetry for all telemetry signals — **traces** and **metrics** are pushed via OTLP to a local collector. There is no pull-based `/metrics` endpoint; everything is push-based, suitable for local-first deployments without sidecars.

## Architecture

```
┌─────────┐   OTLP (HTTP/gRPC)   ┌────────────────┐
│  Rara   │ ───────────────────▶  │  OTel Collector │
│         │   traces + metrics    │  (or Alloy)     │
└─────────┘   every 30s (metrics) └────────┬───────┘
                                           │
                          ┌────────────────┼────────────────┐
                          ▼                ▼                ▼
                    ┌──────────┐   ┌────────────┐   ┌───────────┐
                    │  Tempo   │   │   Mimir /   │   │  Grafana  │
                    │ (traces) │   │ Prometheus  │   │           │
                    └──────────┘   │ (metrics)   │   └───────────┘
                                   └────────────┘
```

## Configuration

All telemetry is configured in `config.yaml` under the `telemetry` section:

```yaml
telemetry:
  otlp_endpoint: "http://localhost:4318"   # OTLP collector endpoint
  otlp_protocol: "http"                    # "http" or "grpc"
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `otlp_endpoint` | `string` | `null` (disabled) | OTLP collector endpoint. Set this to enable telemetry. |
| `otlp_protocol` | `string` | `"http"` | Transport protocol: `"http"` (port 4318) or `"grpc"` (port 4317). |

When `otlp_endpoint` is set, Rara pushes both traces and metrics to the same endpoint:

- **HTTP**: traces → `{endpoint}/v1/traces`, metrics → `{endpoint}/v1/metrics`
- **gRPC**: both signals use the same gRPC endpoint

When `otlp_endpoint` is `null`, telemetry is disabled (logs still go to local files).

## Signals

### Traces

Distributed tracing with span context propagation (W3C Trace Context). Key instrumented operations:

- `run_agent_loop` — full agent execution
- `start_llm_turn` — individual LLM turn with iterations
- Per-iteration spans with `first_token_ms`, `stream_ms`, model info
- Tool execution spans with duration and success/failure

### Metrics

All metrics are pushed via OTLP periodic exporter (30-second flush interval). Metric names use dot notation per OTel convention.

#### Kernel Metrics (meter: `rara-kernel`)

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `kernel.session.created` | Counter | `agent_name` | Sessions created |
| `kernel.session.suspended` | Counter | `agent_name`, `exit_state` | Sessions suspended |
| `kernel.session.active` | UpDownCounter | `agent_name` | Currently active sessions |
| `kernel.turn.total` | Counter | `agent_name`, `model` | Total LLM turns |
| `kernel.turn.duration` | Histogram (s) | `agent_name`, `model` | LLM turn duration |
| `kernel.turn.tool_calls` | Counter | `agent_name`, `tool_name` | Tool calls per tool |
| `kernel.turn.tokens.input` | Counter | `model` | Input tokens consumed |
| `kernel.turn.tokens.output` | Counter | `model` | Output tokens consumed |
| `kernel.tool.duration` | Histogram (s) | `agent_name`, `tool_name` | Per-tool execution duration |
| `kernel.event.processed` | Counter | `event_type` | Events processed |
| `kernel.syscall.total` | Counter | `syscall_type` | Syscalls executed |
| `kernel.message.inbound` | Counter | `channel_type` | Inbound messages |
| `kernel.message.outbound` | Counter | `channel_type` | Outbound messages |

#### Worker Metrics (meter: `rara-worker`)

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `worker.started` | Counter | `worker` | Worker starts |
| `worker.stopped` | Counter | `worker` | Worker stops |
| `worker.active` | UpDownCounter | `worker` | Currently active workers |
| `worker.errors` | Counter | `worker` | Total worker errors |
| `worker.start_errors` | Counter | `worker` | Start failures |
| `worker.shutdown_errors` | Counter | `worker` | Shutdown failures |
| `worker.executions` | Counter | `worker` | Execution cycles |
| `worker.execution_errors` | Counter | `worker` | Execution failures |
| `worker.execution.duration` | Histogram (s) | `worker` | Execution cycle duration |
| `worker.paused` | Counter | `worker` | Pause events |
| `worker.resumed` | Counter | `worker` | Resume events |

#### HTTP Server Metrics (meter: `rara-server`)

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `http.server.request.duration` | Histogram (s) | `method`, `route`, `status` | HTTP request duration |

### Tape (Structured Event Log)

In addition to OTel signals, Rara persists a complete event log to local JSONL files (the "tape"). Each tape entry includes typed metadata:

- **LLM calls**: model, token usage, latency (`stream_ms`, `first_token_ms`), stop reason
- **Tool calls**: per-tool `duration_ms`, success/failure, error messages
- **Messages**: full request/response content

Tape files are the single source of truth for conversation replay and post-hoc token analysis. See `crates/kernel/src/memory/` for details.

## Quick Start: Local Setup with Grafana Alloy

The simplest local setup uses [Grafana Alloy](https://grafana.com/docs/alloy/latest/) as the OTLP receiver:

### 1. Install Alloy

```bash
brew install grafana/grafana/alloy    # macOS
```

### 2. Configure Alloy

Create `alloy-config.alloy`:

```hcl
otelcol.receiver.otlp "default" {
  http {
    endpoint = "0.0.0.0:4318"
  }
  grpc {
    endpoint = "0.0.0.0:4317"
  }

  output {
    traces  = [otelcol.exporter.otlphttp.tempo.input]
    metrics = [otelcol.exporter.prometheus.default.input]
  }
}

otelcol.exporter.otlphttp "tempo" {
  client {
    endpoint = "http://localhost:3200"
  }
}

otelcol.exporter.prometheus "default" {
  forward_to = [prometheus.remote_write.mimir.receiver]
}

prometheus.remote_write "mimir" {
  endpoint {
    url = "http://localhost:9009/api/v1/push"
  }
}
```

### 3. Configure Rara

```yaml
# config.yaml
telemetry:
  otlp_endpoint: "http://localhost:4318"
  otlp_protocol: "http"
```

### 4. Start

```bash
alloy run alloy-config.alloy &
rara server
```

Traces and metrics will flow to your local Grafana stack. Import the dashboard from `deploy/grafana/rara-overview.json` for pre-built visualizations.
