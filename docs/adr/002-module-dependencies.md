# ADR-002: Module Dependency Rules and Crate Mapping

- **Status**: Proposed
- **Date**: 2026-02-08
- **Issue**: [#4 - System Architecture and Module Boundary Design](https://github.com/crrow/job/issues/4)
- **Supersedes**: None
- **Related**: [ADR-001 System Architecture](./001-system-architecture.md)

## Context

ADR-001 定义了八个业务模块和整体架构。本文档细化模块间的依赖规则、crate 组织结构，以及从现有 codebase 到目标结构的演进路径。

---

## Crate Organization

### Target Workspace Layout

```
job/
├── Cargo.toml                      # Workspace root
├── api/                            # Proto definitions + generated code
│   └── job-api
│
├── crates/
│   ├── app/                        # Application lifecycle, wiring
│   │   └── job-app
│   │
│   ├── cmd/                        # CLI binary (clap)
│   │   └── job-cli
│   │
│   ├── common/                     # Shared infrastructure
│   │   ├── base/                   # job-base: utilities, ReadableSize, etc.
│   │   ├── error/                  # job-error: error types, status codes
│   │   ├── runtime/               # job-common-runtime: tokio runtime
│   │   ├── telemetry/             # job-common-telemetry: tracing + otel
│   │   ├── worker/                # job-common-worker: worker framework
│   │   ├── yunara-store/          # job-yunara-store: PostgreSQL abstractions
│   │   ├── outbox/                # job-common-outbox (NEW): outbox pattern impl
│   │   ├── audit/                 # job-common-audit (NEW): audit log
│   │   └── utils/
│   │       └── downloader/        # HTTP downloader
│   │
│   ├── domain/                    # Business domain modules (NEW)
│   │   ├── job-source/            # Job discovery and normalization
│   │   ├── ai/                    # AI provider abstraction
│   │   ├── resume/                # Resume management
│   │   ├── applicant/             # Application lifecycle
│   │   ├── interview/             # Interview preparation
│   │   ├── scheduler/             # Pipeline orchestration
│   │   ├── notify/                # Notification delivery
│   │   └── analytics/             # Metrics and reporting
│   │
│   ├── server/                    # HTTP (axum) + gRPC (tonic)
│   │   └── job-server
│   │
│   └── paths/                     # Platform path utilities
│       └── job-paths
```

### New Crates to Create

| Crate | Path | Purpose |
|-------|------|---------|
| `job-common-outbox` | `crates/common/outbox/` | Outbox table operations, event publishing/consuming |
| `job-common-audit` | `crates/common/audit/` | Audit log writing, querying |
| `job-domain-job-source` | `crates/domain/job-source/` | Job board scraping, normalization |
| `job-domain-ai` | `crates/domain/ai/` | AI provider abstraction, prompt management |
| `job-domain-resume` | `crates/domain/resume/` | Resume CRUD, versioning |
| `job-domain-applicant` | `crates/domain/applicant/` | Application state machine |
| `job-domain-interview` | `crates/domain/interview/` | Interview prep generation |
| `job-domain-scheduler` | `crates/domain/scheduler/` | Pipeline orchestration, cron rules |
| `job-domain-notify` | `crates/domain/notify/` | Multi-channel notification delivery |
| `job-domain-analytics` | `crates/domain/analytics/` | Aggregation, funnel analysis |

---

## Dependency Rules

### Layer Hierarchy

```
Layer 4 (Entry):      job-cli, job-app
Layer 3 (Interface):  job-server (HTTP + gRPC)
Layer 2 (Domain):     job-source, ai, resume, applicant, interview, scheduler, notify, analytics
Layer 1 (Infra):      yunara-store, outbox, audit, worker, telemetry, runtime
Layer 0 (Foundation): base, error, paths
```

**Rule**: A crate may only depend on crates in the same layer or lower layers. Never upward.

### Dependency Matrix

行 = dependent，列 = dependency。`D` = direct dependency, `T` = trait-only (via interface), `-` = forbidden.

```
                   | base | error | paths | store | outbox | audit | worker | telemetry | runtime |
-------------------|------|-------|-------|-------|--------|-------|--------|-----------|---------|
job-source         |  D   |   D   |   -   |   D   |   D    |   -   |   -    |    D      |    -    |
ai                 |  D   |   D   |   -   |   D   |   D    |   D   |   -    |    D      |    -    |
resume             |  D   |   D   |   -   |   D   |   D    |   -   |   -    |    D      |    -    |
applicant          |  D   |   D   |   -   |   D   |   D    |   D   |   -    |    D      |    -    |
interview          |  D   |   D   |   -   |   D   |   D    |   -   |   -    |    D      |    -    |
scheduler          |  D   |   D   |   -   |   D   |   D    |   D   |   -    |    D      |    -    |
notify             |  D   |   D   |   -   |   D   |   D    |   D   |   -    |    D      |    -    |
analytics          |  D   |   D   |   -   |   D   |   -    |   -   |   -    |    D      |    -    |
```

### Domain Module Cross-Dependencies

域模块之间原则上不直接依赖。如果模块 A 需要模块 B 的数据，有两种合规路径：

1. **Trait abstraction** -- 模块 B 在自己的 crate 中定义 trait，模块 A 依赖该 trait，`job-app` 负责 wiring
2. **Event-driven** -- 模块 A 发布 outbox event，模块 B 通过 outbox processor 消费

例外：`scheduler` 作为编排模块，可依赖其他域模块的 **trait definitions**（不是具体实现）。

```
                   | job-source | ai | resume | applicant | interview | scheduler | notify | analytics |
-------------------|------------|-----|--------|-----------|-----------|-----------|--------|-----------|
job-source         |     -      |  -  |   -    |     -     |     -     |     -     |   -    |     -     |
ai                 |     -      |  -  |   T    |     -     |     -     |     -     |   -    |     -     |
resume             |     -      |  -  |   -    |     -     |     -     |     -     |   -    |     -     |
applicant          |     -      |  -  |   T    |     -     |     -     |     -     |   -    |     -     |
interview          |     -      |  T  |   -    |     -     |     -     |     -     |   -    |     -     |
scheduler          |     T      |  T  |   T    |     T     |     T     |     -     |   T    |     -     |
notify             |     -      |  -  |   -    |     -     |     -     |     -     |   -    |     -     |
analytics          |     -      |  -  |   -    |     -     |     -     |     -     |   -    |     -     |
```

**Key observations**:
- `scheduler` is the only module with broad cross-domain trait dependencies -- it orchestrates the pipeline
- `ai` needs `resume` traits to read resume data for matching/optimization
- `applicant` needs `resume` traits to attach resumes to applications
- `interview` needs `ai` traits to generate prep content
- `analytics` and `notify` have ZERO domain cross-dependencies -- they are purely event-driven

---

## Module Interface Design

每个域模块暴露的公共 API 分为三层：

### 1. Service Trait (核心接口)

```rust
// crates/domain/job-source/src/lib.rs

/// Core interface for job discovery operations.
#[async_trait]
pub trait JobSourceService: Send + Sync {
    /// Discover new jobs from configured sources.
    async fn discover(&self, criteria: &DiscoveryCriteria) -> Result<Vec<JobListing>>;

    /// Get a specific job by ID.
    async fn get_job(&self, id: &JobId) -> Result<Option<JobListing>>;

    /// Search jobs with filters.
    async fn search(&self, filter: &JobFilter) -> Result<Vec<JobListing>>;
}
```

### 2. Domain Types (数据模型)

```rust
// crates/domain/job-source/src/types.rs

pub struct JobId(Uuid);
pub struct JobListing {
    pub id:          JobId,
    pub source:      JobSource,
    pub title:       String,
    pub company:     String,
    pub url:         String,
    pub description: String,
    pub posted_at:   Option<DateTime<Utc>>,
    pub discovered_at: DateTime<Utc>,
    pub metadata:    serde_json::Value,
}

pub enum JobSource {
    LinkedIn,
    Indeed,
    Manual,
    // extensible
}
```

### 3. Events (出站事件)

```rust
// crates/domain/job-source/src/events.rs

pub const EVENT_JOB_DISCOVERED: &str = "job.discovered";

#[derive(Serialize, Deserialize)]
pub struct JobDiscoveredEvent {
    pub job_id:  Uuid,
    pub source:  String,
    pub title:   String,
    pub url:     String,
}
```

---

## Infrastructure Crate Details

### `job-common-outbox`

```rust
// Core outbox interface
pub struct OutboxEvent {
    pub id:         Uuid,
    pub event_type: String,
    pub payload:    serde_json::Value,
    pub module:     String,
    pub created_at: DateTime<Utc>,
}

#[async_trait]
pub trait OutboxPublisher: Send + Sync {
    /// Publish an event within the given transaction.
    async fn publish_in_tx(
        &self,
        tx: &mut PgTransaction<'_>,
        event_type: &str,
        payload: &impl Serialize,
        module: &str,
    ) -> Result<Uuid>;
}

#[async_trait]
pub trait OutboxConsumer: Send + Sync {
    /// Fetch a batch of unprocessed events.
    async fn fetch_batch(&self, limit: i64) -> Result<Vec<OutboxEvent>>;

    /// Mark an event as processed.
    async fn mark_processed(&self, event_id: Uuid) -> Result<()>;

    /// Record a processing failure.
    async fn record_failure(&self, event_id: Uuid, error: &str) -> Result<()>;
}
```

### `job-common-audit`

```rust
pub struct AuditEntry {
    pub module:   String,
    pub action:   String,
    pub actor:    String,
    pub input:    Option<serde_json::Value>,
    pub output:   Option<serde_json::Value>,
    pub metadata: Option<serde_json::Value>,
}

#[async_trait]
pub trait AuditLogger: Send + Sync {
    async fn log(&self, entry: AuditEntry) -> Result<()>;

    /// Convenience: log within an existing transaction.
    async fn log_in_tx(
        &self,
        tx: &mut PgTransaction<'_>,
        entry: AuditEntry,
    ) -> Result<()>;
}
```

---

## Wiring in `job-app`

`job-app` 是唯一知道所有具体实现的 crate，负责依赖注入：

```rust
// Conceptual wiring in job-app

pub struct AppState {
    pub db:             DBStore,
    pub outbox:         Arc<dyn OutboxPublisher>,
    pub audit:          Arc<dyn AuditLogger>,
    pub job_source:     Arc<dyn JobSourceService>,
    pub ai:             Arc<dyn AiService>,
    pub resume:         Arc<dyn ResumeService>,
    pub applicant:      Arc<dyn ApplicantService>,
    pub interview:      Arc<dyn InterviewService>,
    pub scheduler:      Arc<dyn SchedulerService>,
    pub notify:         Arc<dyn NotifyService>,
    pub analytics:      Arc<dyn AnalyticsService>,
}
```

`job-app` 的依赖：

```toml
[dependencies]
# All domain crates (for concrete implementations)
job-domain-job-source = { path = "../domain/job-source" }
job-domain-ai = { path = "../domain/ai" }
job-domain-resume = { path = "../domain/resume" }
job-domain-applicant = { path = "../domain/applicant" }
job-domain-interview = { path = "../domain/interview" }
job-domain-scheduler = { path = "../domain/scheduler" }
job-domain-notify = { path = "../domain/notify" }
job-domain-analytics = { path = "../domain/analytics" }

# Infrastructure
job-common-outbox = { path = "../common/outbox" }
job-common-audit = { path = "../common/audit" }
yunara-store = { path = "../common/yunara-store" }
job-common-worker = { path = "../common/worker" }

# Server
job-server = { path = "../server" }
```

---

## Worker Registration

Workers 在 `job-app` 中注册到 `Manager`：

```rust
// Conceptual worker setup
fn register_workers(manager: &mut Manager<AppState>, state: &AppState) {
    // Job discovery: every 6 hours
    manager
        .worker(JobDiscoveryWorker::new(state.job_source.clone()))
        .name("job-discovery")
        .cron("0 */6 * * *")
        .unwrap()
        .spawn();

    // Outbox processor: every 1 second
    manager
        .worker(OutboxProcessorWorker::new(
            state.outbox.clone(),
            build_event_router(state),
        ))
        .name("outbox-processor")
        .interval(Duration::from_secs(1))
        .spawn();

    // Analytics aggregation: daily at 2am
    manager
        .worker(AnalyticsAggregationWorker::new(state.analytics.clone()))
        .name("analytics-daily")
        .cron("0 2 * * *")
        .unwrap()
        .spawn();
}
```

---

## HTTP Route Registration

各域模块提供路由注册函数，在 `job-app` 中组装：

```rust
// Each domain module exposes routes
// crates/domain/job-source/src/routes.rs
pub fn job_source_routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/v1/jobs", get(list_jobs).post(trigger_discovery))
        .route("/api/v1/jobs/:id", get(get_job))
        .with_state(state)
}

// job-app wires them together
let route_handlers = vec![
    |r: Router| job_source_routes(state.clone())(r),
    |r: Router| resume_routes(state.clone())(r),
    |r: Router| applicant_routes(state.clone())(r),
    |r: Router| analytics_routes(state.clone())(r),
    health_routes,
];
```

---

## Migration Path

从当前 codebase 到目标结构的渐进式迁移：

### Step 1: Infrastructure Foundation
1. Create `job-common-outbox` crate with outbox table + publisher/consumer
2. Create `job-common-audit` crate with audit log table + logger
3. Add DB migrations for `outbox_events` and `audit_log` tables

### Step 2: First Domain Module
1. Create `crates/domain/job-source/` as first domain crate
2. Define `JobSourceService` trait + types + events
3. Implement basic scraping logic
4. Wire into `job-app`

### Step 3: AI Module
1. Create `crates/domain/ai/` with AI provider abstraction
2. Support OpenAI/Anthropic as interchangeable backends
3. Integrate audit logging for all AI calls

### Step 4: Remaining Modules
1. Resume -> Applicant -> Scheduler -> Notify -> Interview -> Analytics
2. Each module follows the same pattern: trait + types + events + impl

### Step 5: Outbox Wiring
1. Register OutboxProcessor worker
2. Build event router mapping event types to handlers
3. End-to-end test: discover -> match -> notify flow

---

## Decision

1. 域模块放在 `crates/domain/` 下，与 `crates/common/` 的基础设施层物理分离
2. 模块间通过 trait 抽象交互，`job-app` 作为 composition root 负责 wiring
3. 跨模块副作用通过 outbox events 传递，不直接调用
4. `scheduler` 是唯一允许跨域 trait 依赖的编排模块
5. 每个域模块暴露三层公共 API：Service Trait / Domain Types / Events
6. 基础设施 crate（outbox, audit）提供通用 trait，域模块按需使用

## Consequences

- (+) Cargo workspace 在编译期阻断循环依赖
- (+) 各模块可独立编译和测试
- (+) 新增域模块有明确的模板和约定
- (+) `job-app` 的 composition root 模式使得测试时可以轻松替换 mock 实现
- (-) 初始 crate 数量较多（约 20 个），workspace 配置有一定管理成本
- (-) Trait-based abstraction 对于简单场景可能过度设计，但考虑到长期可维护性是值得的
- (-) `scheduler` 的跨域依赖需要在 review 中特别关注，防止它退化为 God Module
