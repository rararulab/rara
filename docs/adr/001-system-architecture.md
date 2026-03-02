# ADR-001: System Architecture and Module Boundaries

- **Status**: Proposed
- **Date**: 2026-02-08
- **Issue**: [#4 - System Architecture and Module Boundary Design](https://github.com/rararulab/rara/issues/4)

## Context

Job 是一个 AI 驱动的求职自动化平台，核心功能包括：岗位发现、AI 匹配、简历优化、自动投递、面试准备、通知推送和数据分析。系统需要同时处理同步 HTTP 请求和异步后台任务（爬取、AI 推理、定时调度等），因此架构需要在单进程内融合 HTTP Server 和 Background Worker 两种运行形态。

### Design Drivers

1. **Single-binary deployment** -- 初期以单进程部署，降低运维复杂度
2. **Module isolation** -- 各业务域通过 workspace crate 物理隔离，编译期阻断循环依赖
3. **Async event flow** -- 模块间通过 Outbox Pattern 解耦，支持最终一致性
4. **Auditability** -- 所有 AI 决策和外发动作必须留审计日志
5. **Extensibility** -- 未来可水平拆分为多进程/多服务

---

## C4 Level 1: System Context

```
                          +-----------+
                          |   User    |
                          | (Candidate)|
                          +-----+-----+
                                |
                     HTTP API / Telegram Bot
                                |
                          +-----v-----+
                          |           |
     +---LinkedIn API---->|    Job    |<----Telegram API----+
     |   Indeed API       | Platform  |     Email (SMTP)    |
     |   etc.             |           |                     |
     +---AI Provider----->|  (Rust)   |                     |
         (OpenAI/Claude)  +-----------+
                                |
                           PostgreSQL
```

### External Systems

| System | Direction | Purpose |
|--------|-----------|---------|
| Job boards (LinkedIn, Indeed, etc.) | Inbound | Scrape/API-fetch job listings |
| AI Providers (OpenAI, Anthropic, etc.) | Outbound | Resume optimization, job matching, interview prep |
| Telegram | Outbound | Push notifications to user |
| Email (SMTP) | Outbound | Email notifications, application submissions |
| PostgreSQL | Bidirectional | Primary data store |
| Redis (optional) | Bidirectional | Deduplication cache, rate-limit counters |

---

## C4 Level 2: Container Diagram

所有容器运行在同一个 Rust 进程中，通过 `CancellationToken` 统一管理生命周期。

```
+------------------------------------------------------------------+
|                       Job Process (single binary)                 |
|                                                                   |
|  +--------------------+       +-----------------------------+     |
|  |    HTTP Server     |       |       Worker Engine         |     |
|  |    (axum)          |       |    (job-common-worker)      |     |
|  |                    |       |                             |     |
|  |  /api/v1/jobs      |       |  [Cron] JobDiscoveryWorker  |     |
|  |  /api/v1/resumes   |       |  [Cron] MatchingWorker      |     |
|  |  /api/v1/apps      |       |  [Notify] ApplicationWorker |     |
|  |  /api/v1/analytics |       |  [Interval] OutboxProcessor |     |
|  |  /api/v1/health    |       |  [Cron] AnalyticsWorker     |     |
|  +--------+-----------+       +--------+--------------------+     |
|           |                            |                          |
|           v                            v                          |
|  +----------------------------------------------------+          |
|  |              Domain Services Layer                  |          |
|  |                                                     |          |
|  |  job_source | ai | resume | applicant | interview   |          |
|  |  scheduler  | notify | analytics                    |          |
|  +----------------------------------------------------+          |
|           |                            |                          |
|           v                            v                          |
|  +----------------------------------------------------+          |
|  |              Infrastructure Layer                   |          |
|  |                                                     |          |
|  |  yunara-store (PG) | outbox | audit_log | telemetry |          |
|  +----------------------------------------------------+          |
+------------------------------------------------------------------+
         |              |                |
         v              v                v
    PostgreSQL     Redis (opt)      AI Providers
```

### Container Responsibilities

| Container | Technology | Responsibility |
|-----------|-----------|----------------|
| HTTP Server | axum | REST API endpoints, request validation, auth |
| gRPC Server | tonic | Internal/admin RPC (optional, already exists) |
| Worker Engine | job-common-worker | Background tasks: scraping, matching, outbox processing |
| Domain Services | Rust crates | Business logic, domain rules, orchestration |
| Infrastructure | sqlx, reqwest | DB access, HTTP clients, message outbox |

---

## Module Boundaries

### Domain Modules

八个核心业务模块，每个对应一个 workspace crate：

```
crates/domain/
  job-source/       -- Job discovery and normalization
  ai/               -- AI provider abstraction (matching, generation)
  resume/           -- Resume storage, versioning, optimization
  applicant/        -- Application lifecycle (draft -> submitted -> tracked)
  interview/        -- Interview prep content generation
  scheduler/        -- Scheduling and orchestration rules
  notify/           -- Notification delivery (Telegram, email, webhook)
  analytics/        -- Metrics aggregation, funnel analysis, reporting
```

### Module Dependency Rules

每个模块通过 trait 暴露公共接口，依赖关系严格单向。详细依赖矩阵见 [ADR-002](./002-module-dependencies.md)。

核心原则：

1. **No circular dependencies** -- 编译期由 Cargo workspace 保证
2. **Depend on abstractions** -- 模块间通过 trait 交互，不直接依赖具体实现
3. **Infrastructure sinks down** -- 业务模块不直接引用 `sqlx`、`reqwest` 等基础设施库
4. **Events for cross-cutting** -- 跨模块副作用通过 Outbox events 传递，不直接调用

### Dependency Direction (simplified)

```
scheduler  --->  job_source  --->  ai
    |                                |
    v                                v
applicant  <---  (events)  <---  resume
    |
    v
notify  <---  analytics
```

- `scheduler` orchestrates the pipeline: triggers `job_source` scraping, then `ai` matching
- `ai` is a pure service: receives input, returns output, no side effects on other modules
- `resume` manages resume data; `ai` reads resumes to generate match scores
- `applicant` tracks application state; triggered by events from `scheduler`
- `notify` delivers messages; triggered by events from `applicant` and `scheduler`
- `analytics` observes events from all modules; read-only, no reverse dependencies

---

## Event Flow: Outbox Pattern

### Why Outbox

模块间异步通信使用 Outbox Pattern 而非直接消息队列，原因：

1. **Transactional guarantee** -- 业务数据变更和事件发布在同一个 DB 事务中，不会丢事件
2. **No extra infra** -- 不需要 Kafka/RabbitMQ，PostgreSQL 即可承载
3. **Replay/debug** -- 事件持久化在 `outbox_events` 表，便于排查和重放
4. **Idempotency** -- 消费端通过 `event_id` 去重

### Outbox Table Schema

```sql
CREATE TABLE outbox_events (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    event_type  TEXT NOT NULL,           -- e.g. "job.discovered", "match.completed"
    payload     JSONB NOT NULL,          -- event-specific data
    module      TEXT NOT NULL,           -- source module name
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    processed_at TIMESTAMPTZ,           -- NULL until consumed
    attempts    INT NOT NULL DEFAULT 0,  -- retry count
    last_error  TEXT                     -- last processing error
);

CREATE INDEX idx_outbox_unprocessed ON outbox_events (created_at)
    WHERE processed_at IS NULL;
```

### Outbox Processing Flow

```
[Domain Module]                    [OutboxProcessor Worker]
      |                                     |
      | -- INSERT INTO outbox_events -->    |
      |    (within business tx)             |
      |                                     |
      |                            (Interval: 1s)
      |                                     |
      |                            SELECT unprocessed events
      |                            ORDER BY created_at
      |                            LIMIT batch_size
      |                                     |
      |                            Route event to handler
      |                            by event_type
      |                                     |
      |                            UPDATE processed_at
      |                            or INCREMENT attempts
```

### Event Types

| Event Type | Producer | Consumer(s) | Payload |
|-----------|----------|-------------|---------|
| `job.discovered` | job_source | scheduler, analytics | `{ job_id, source, title, url }` |
| `job.matched` | ai | scheduler, analytics | `{ job_id, resume_id, score, reasoning }` |
| `resume.optimized` | ai | applicant, notify | `{ resume_id, job_id, version }` |
| `application.submitted` | applicant | notify, analytics | `{ app_id, job_id, status }` |
| `application.status_changed` | applicant | notify, analytics | `{ app_id, old_status, new_status }` |
| `interview.prep_generated` | interview | notify | `{ job_id, prep_id }` |
| `notification.sent` | notify | analytics | `{ channel, target, event_ref }` |

---

## Key Sequence: Discover -> Match -> Notify -> Apply -> Track

```mermaid
sequenceDiagram
    participant S as Scheduler
    participant JS as JobSource
    participant AI as AI Module
    participant R as Resume
    participant N as Notify
    participant A as Applicant
    participant DB as PostgreSQL

    Note over S: Cron trigger (e.g. every 6h)

    S->>JS: discover_jobs(sources, criteria)
    JS->>JS: Scrape / API fetch
    JS->>DB: INSERT jobs + outbox(job.discovered)
    JS-->>S: Vec<JobSummary>

    Note over S: OutboxProcessor picks up job.discovered

    S->>AI: match_jobs(jobs, user_profile)
    AI->>R: get_active_resume(user_id)
    R-->>AI: Resume
    AI->>AI: LLM scoring + reasoning
    AI->>DB: INSERT match_results + outbox(job.matched)
    AI-->>S: Vec<MatchResult>

    Note over S: Filter by score threshold

    S->>N: notify_matches(high_score_matches)
    N->>N: Send Telegram / email
    N->>DB: INSERT notification_log + outbox(notification.sent)

    Note over S: User approves via Telegram / API

    S->>AI: optimize_resume(resume, job)
    AI->>DB: INSERT resume_version + outbox(resume.optimized)
    AI-->>S: OptimizedResume

    S->>A: submit_application(job, resume)
    A->>A: Fill form / send email
    A->>DB: INSERT application(status=submitted) + outbox(application.submitted)
    A-->>S: ApplicationId

    Note over A: Periodic status check

    A->>A: check_status(application)
    A->>DB: UPDATE status + outbox(application.status_changed)
    A->>N: notify_status_change(app)
```

### Sequence: Resume Optimization Detail

```mermaid
sequenceDiagram
    participant U as User
    participant API as HTTP API
    participant R as Resume
    participant AI as AI Module
    participant DB as PostgreSQL

    U->>API: POST /api/v1/resumes/{id}/optimize
    API->>R: get_resume(id)
    R->>DB: SELECT resume
    R-->>API: Resume

    API->>AI: optimize(resume, target_job)
    AI->>AI: Call LLM provider
    AI->>DB: INSERT audit_log(action=resume_optimize, input, output)
    AI-->>API: OptimizedContent

    API->>R: create_version(resume_id, content)
    R->>DB: INSERT resume_version
    R-->>API: ResumeVersion

    API-->>U: 200 { version_id, diff_summary }
```

---

## Audit Log

所有 AI 决策和外发动作（投递、通知）必须记录审计日志。

```sql
CREATE TABLE audit_log (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    timestamp   TIMESTAMPTZ NOT NULL DEFAULT now(),
    module      TEXT NOT NULL,           -- "ai", "applicant", "notify"
    action      TEXT NOT NULL,           -- "match_job", "optimize_resume", "submit_application"
    actor       TEXT NOT NULL,           -- "system" or user identifier
    input       JSONB,                   -- request/input snapshot
    output      JSONB,                   -- response/output snapshot
    metadata    JSONB                    -- extra context (model name, latency, tokens used, etc.)
);

CREATE INDEX idx_audit_log_module_action ON audit_log (module, action, timestamp);
```

### What Must Be Audited

| Category | Actions |
|----------|---------|
| AI Decisions | `match_job`, `optimize_resume`, `generate_interview_prep`, `score_application` |
| Outbound Actions | `submit_application`, `send_notification`, `send_email` |
| Data Mutations | `create_resume_version`, `change_application_status` |
| Configuration Changes | `update_matching_criteria`, `update_notification_preferences` |

---

## Deployment Topology

### Phase 1: Single Machine (Current Target)

```
+-------------------------------------------+
|  Single Host (dev / small-scale prod)     |
|                                           |
|  +-------------------------------------+ |
|  |  job (single binary)                | |
|  |  - HTTP :25555                       | |
|  |  - gRPC :50051                       | |
|  |  - Worker threads (tokio tasks)      | |
|  +-------------------------------------+ |
|                                           |
|  +------------------+  +---------------+  |
|  |   PostgreSQL     |  | Redis (opt)   |  |
|  |   :5432          |  | :6379         |  |
|  +------------------+  +---------------+  |
+-------------------------------------------+
```

- 单进程部署，HTTP + gRPC + Workers 共享 tokio runtime
- PostgreSQL 为唯一必需外部依赖
- Redis 可选，用于 job URL 去重缓存和速率限制计数器
- 通过 `docker compose` 一键启动

### Phase 2: Containerized

```yaml
# docker-compose.yml (conceptual)
services:
  job:
    build: .
    command: job server
    ports:
      - "25555:25555"
      - "50051:50051"
    environment:
      DATABASE_URL: postgres://job:job@postgres:5432/job
      REDIS_URL: redis://redis:6379
    depends_on:
      - postgres
      - redis

  postgres:
    image: postgres:17
    volumes:
      - pg_data:/var/lib/postgresql/data
    environment:
      POSTGRES_DB: job
      POSTGRES_USER: job
      POSTGRES_PASSWORD: job

  redis:
    image: redis:7-alpine
    # Optional: only needed for dedup/rate-limiting
```

### Phase 3: Horizontal Scaling (Future)

如果负载增长到单进程无法承受：

1. **Separate API and Workers** -- `job server` 只运行 HTTP/gRPC；`job worker` 只运行后台任务
2. **Multiple Worker instances** -- Outbox 表天然支持多消费者（`SELECT ... FOR UPDATE SKIP LOCKED`）
3. **Read replicas** -- Analytics 查询可路由到 PG read replica
4. **Dedicated AI queue** -- AI 调用延迟高，可独立为专用 worker pool

---

## Operational Considerations

### Health Checks

- `GET /health` -- 基础存活检查（已实现）
- `GET /api/v1/health` -- 详细健康信息，包括 DB 连通性、worker 状态
- gRPC health service（已实现）

### Observability

- **Tracing**: OpenTelemetry (已集成 `job-common-telemetry`)
- **Metrics**: Prometheus (worker metrics 已有基础)
- **Logging**: Structured JSON logs via `tracing-subscriber`

### Configuration

- 环境变量优先（12-factor），fallback 到配置文件
- 敏感信息（API keys, DB password）必须通过环境变量或 secret manager 注入

### Graceful Shutdown

- 已有 `CancellationToken` + signal handler 机制
- Worker Manager 的 `shutdown()` 确保所有 worker 有序停止
- Outbox events 未处理完的不会丢失（持久化在 DB 中）

---

## Decision

采用 **单进程 HTTP + Worker + Outbox** 架构，理由：

1. 项目初期，单二进制部署最简单，减少 DevOps 开销
2. Outbox Pattern 兼顾事务一致性和模块解耦，且不引入额外消息中间件
3. Worker 框架已经成熟（`job-common-worker`），支持多种触发模式
4. 八个业务模块通过 workspace crate 物理隔离，编译期保证无循环依赖
5. 未来可通过分离 `job server` / `job worker` 子命令实现水平扩展

## Consequences

- (+) 部署简单，单二进制 + PostgreSQL 即可运行
- (+) 事件不丢失，Outbox 与业务数据同事务写入
- (+) 模块边界清晰，可独立开发和测试
- (-) 单进程瓶颈：如果 AI 调用量极大，需要拆分 worker 进程
- (-) Outbox 轮询有 1-5s 延迟，不适合需要亚秒级响应的场景（当前业务不需要）
- (-) 需要维护 event type 注册表，新增模块需要注册事件类型
