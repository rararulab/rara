# Observability 全链路接入 — 设计文档

**Date**: 2026-03-02
**Status**: Draft

## 问题

当前 rara 的可观测性有以下缺失：

1. **无 `/metrics` 端点** — Prometheus 已部署但无法 scrape 应用指标
2. **核心路径缺少 `#[instrument]`** — kernel event_loop、agent_turn、process spawn 等关键路径无结构化 tracing
3. **OTLP 默认禁用** — 需要手动配置 Langfuse 或 OTLP endpoint 才能发送 traces
4. **无 Grafana 仪表板** — 数据源已配置（Tempo/Quickwit/Prometheus）但无 rara 专用仪表板

## 现有基础

- `common-telemetry` crate：完整的 OTLP SDK（exporter, sampler, context propagation）
- `worker/metrics.rs`：12 个 Prometheus 指标（LazyLock 静态注册）
- Helm 部署：Grafana + Prometheus + Tempo + Quickwit + Alloy + Langfuse
- Alloy 配置：OTLP receiver → traces→Tempo, metrics→Prometheus, logs→Quickwit
- HTTP TraceLayer：已有基础的 `http_request` span（method + path + status + latency）
- 126 个 `#[instrument]` 分布在 17 个文件（主要在 backend-admin 服务层）

## 方案设计

### 1. Prometheus `/metrics` 端点

在 HTTP server 中添加 `/metrics` 路由，暴露 prometheus crate 的 `TextEncoder` 输出。

```rust
// crates/server/src/http.rs — 新增 metrics_handler
async fn metrics_handler() -> impl IntoResponse {
    use prometheus::{Encoder, TextEncoder};
    let encoder = TextEncoder::new();
    let mut buffer = Vec::new();
    let metric_families = prometheus::gather();
    encoder.encode(&metric_families, &mut buffer).unwrap();
    (
        StatusCode::OK,
        [("content-type", "text/plain; version=0.0.4; charset=utf-8")],
        buffer,
    )
}
```

添加到 `health_routes()` 中：
```rust
pub fn health_routes(router: Router) -> Router {
    router
        .route("/api/v1/health", get(api_health_handler))
        .route("/api/health", get(api_health_handler))
        .route("/metrics", get(metrics_handler))
}
```

### 2. 核心路径 `#[instrument]` 补全

需要覆盖的核心路径（按优先级）：

**kernel event_loop.rs**:
- `handle_event` — 顶层事件分发
- `handle_user_message` — 用户消息入口
- `handle_spawn_agent` — 进程创建
- `handle_turn_completed` — turn 完成处理
- `handle_child_completed` — 子进程完成
- `handle_signal` — 信号处理
- `handle_deliver` — 消息投递

**kernel agent_turn.rs**:
- `run_inline_agent_loop` — LLM 循环主入口
- 每个 iteration 的 span（包含 tool call 信息）

**kernel process/ 相关**:
- `ProcessTable::insert` / `remove` / `lookup`
- `AgentRegistry::get` / `register`

**I/O pipeline**:
- `IngressPipeline::process` — 入站消息处理
- `EndpointRegistry::deliver` — 出站消息投递
- `StreamHub::create` / `close` — 流管理

**使用规范**:
```rust
#[instrument(skip(self, runtimes), fields(agent_id = %agent_id))]
```
- 总是 skip self、大型结构（runtimes, pool 等）
- 在 fields 中记录关键标识符（agent_id, session_id, message_id）
- 错误用 `err` 参数自动记录

### 3. Kernel 运行时 Prometheus 指标

在 kernel 中新增 Prometheus 指标模块：

```rust
// crates/core/kernel/src/metrics.rs
pub static PROCESS_SPAWNED: LazyLock<IntCounterVec>     // agent_name
pub static PROCESS_COMPLETED: LazyLock<IntCounterVec>   // agent_name, exit_reason
pub static PROCESS_ACTIVE: LazyLock<IntGaugeVec>        // agent_name
pub static TURN_TOTAL: LazyLock<IntCounterVec>          // agent_name, model
pub static TURN_DURATION_SECONDS: LazyLock<HistogramVec> // agent_name, model
pub static TURN_TOOL_CALLS: LazyLock<IntCounterVec>     // agent_name, tool_name
pub static TURN_TOKENS_INPUT: LazyLock<IntCounterVec>   // model
pub static TURN_TOKENS_OUTPUT: LazyLock<IntCounterVec>  // model
pub static EVENT_QUEUE_SIZE: LazyLock<IntGaugeVec>      // shard_id
pub static EVENT_PROCESSED: LazyLock<IntCounterVec>     // event_type, shard_id
pub static SYSCALL_TOTAL: LazyLock<IntCounterVec>       // syscall_type
pub static MESSAGE_INBOUND: LazyLock<IntCounterVec>     // channel_type
pub static MESSAGE_OUTBOUND: LazyLock<IntCounterVec>    // channel_type
```

在 event_loop 相应位置 increment 这些计数器。

### 4. Grafana 仪表板（JSON provisioning）

通过 Helm ConfigMap 预配置仪表板。创建 3 个仪表板：

**Dashboard 1: Rara Overview（黄金信号）**
- HTTP 请求速率（by path, method）
- HTTP 错误率（4xx/5xx by path）
- HTTP 延迟分位数（p50/p95/p99）
- 活跃 Agent 进程数
- 消息吞吐量（inbound/outbound by channel）

**Dashboard 2: Agent 拓扑**
- 进程 spawn/complete 速率（by agent_name）
- Turn 执行时间（by agent_name, model）
- Tool 调用分布（by tool_name）
- LLM token 消耗（input/output by model）
- Event queue 深度（by shard）

**Dashboard 3: Worker 运行统计**
- Worker 执行速率 + 错误率
- Worker 执行耗时直方图
- Worker active/paused 状态

仪表板 JSON 放在 `deploy/helm/rara-infra/dashboards/` 目录，通过 Grafana sidecar 自动加载。

### 5. OTLP 默认连接

修改 `AppConfig` 的默认行为：当检测到 k8s 环境（`KUBERNETES_SERVICE_HOST` env）时，自动设置 OTLP endpoint 为 `http://rara-infra-alloy:4318/v1/traces`。

本地开发保持默认禁用（避免连接失败噪音）。

## 文件变更清单

| 文件 | 变更 |
|------|------|
| `crates/server/src/http.rs` | 添加 `/metrics` handler |
| `crates/server/Cargo.toml` | 添加 `prometheus` 依赖 |
| `crates/core/kernel/src/metrics.rs` | 新建 - kernel Prometheus 指标 |
| `crates/core/kernel/src/lib.rs` | 声明 metrics 模块 |
| `crates/core/kernel/src/event_loop.rs` | 添加 `#[instrument]` + metrics increment |
| `crates/core/kernel/src/agent_turn.rs` | 添加 `#[instrument]` + turn metrics |
| `crates/core/kernel/src/io/ingress.rs` | 添加 `#[instrument]` |
| `crates/core/kernel/src/io/egress.rs` | 添加 `#[instrument]` |
| `crates/core/kernel/src/io/stream.rs` | 添加 `#[instrument]` |
| `crates/core/kernel/src/process/mod.rs` | 添加 `#[instrument]` |
| `crates/core/kernel/src/process/agent_registry.rs` | 添加 `#[instrument]` |
| `crates/cmd/src/main.rs` | k8s 自动 OTLP 检测 |
| `deploy/helm/rara-infra/dashboards/rara-overview.json` | 新建 - 黄金信号仪表板 |
| `deploy/helm/rara-infra/dashboards/rara-agents.json` | 新建 - Agent 拓扑仪表板 |
| `deploy/helm/rara-infra/dashboards/rara-workers.json` | 新建 - Worker 仪表板 |
| `deploy/helm/rara-infra/values.yaml` | Grafana sidecar dashboard 配置 |

## 验证方式

1. `cargo check` 通过
2. 本地启动后 `curl localhost:25555/metrics` 返回 Prometheus 格式文本
3. 使用 Playwright 访问本地 Grafana（端口转发），验证 3 个仪表板加载正常
4. 发送一条聊天消息后，验证 Tempo 中可以看到完整的 trace 链路
