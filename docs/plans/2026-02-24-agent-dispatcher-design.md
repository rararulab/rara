# Agent Dispatcher Design

**Issue**: #259
**Date**: 2026-02-24
**Status**: In Progress

## Problem

`ProactiveAgent` and `ScheduledAgent` run as independent workers with no coordination:
- Session write conflicts when both target `"agent:proactive"`
- Duplicate notifications in the same time window
- No priority, queue, or global visibility of running agents

## Solution: Unified AgentDispatcher

All background agent executions go through a central `AgentDispatcher` that provides:
1. **Session-parallel execution** — different sessions run concurrently, same session serialized
2. **Priority queue** — urgent tasks preempt low-priority ones
3. **Dedup** — same `dedup_key` won't be enqueued twice within a time window
4. **Observability** — Prometheus metrics + tracing spans + REST API + pluggable LogStore

ChatAgent is exempt (real-time interactive, uses its own session locking).

## Architecture

```
Workers (submit tasks)          AgentDispatcher (run loop)
┌─────────────────────┐        ┌──────────────────────────────┐
│ ProactiveAgentWorker │──tx──▶│  mpsc::Receiver              │
│ AgentSchedulerWorker │──tx──▶│    ├─ dedup check            │
│ (future workers)     │──tx──▶│    ├─ priority queue          │
└─────────────────────┘        │    ├─ session-parallel dispatch│
                               │    ├─ spawn execute_task()     │
                               │    ├─ log to LogStore          │
                               │    └─ emit metrics + spans     │
                               └──────────────────────────────┘
```

## Data Types

### `crates/agents/src/dispatcher/types.rs`

```rust
pub struct AgentTask {
    pub id: String,                     // ULID
    pub kind: AgentTaskKind,
    pub priority: Priority,
    pub session_key: String,
    pub message: String,
    pub history: Vec<ChatMessage>,
    pub dedup_key: Option<String>,
    pub created_at: jiff::Timestamp,
}

pub enum AgentTaskKind {
    Proactive,
    Scheduled { job_id: String },
    Pipeline,
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
pub enum Priority {
    Low = 0,      // proactive review
    Normal = 1,   // scheduled jobs
    High = 2,     // user-triggered
    Urgent = 3,   // manual notify trigger
}

pub enum TaskStatus {
    Queued,
    Running,
    Completed,
    Error,
    Cancelled,
    Deduped,
}

pub struct TaskRecord {
    pub id: String,
    pub kind: AgentTaskKind,
    pub session_key: String,
    pub priority: Priority,
    pub status: TaskStatus,
    pub submitted_at: jiff::Timestamp,
    pub started_at: Option<jiff::Timestamp>,
    pub finished_at: Option<jiff::Timestamp>,
    pub duration_ms: Option<u64>,
    pub error: Option<String>,
    pub iterations: Option<usize>,
    pub tool_calls: Option<usize>,
}
```

## Dispatcher Core

### `crates/agents/src/dispatcher/core.rs`

```rust
pub struct AgentDispatcher {
    tx: mpsc::Sender<DispatcherCommand>,
    // Read-only handles for status queries
    running: Arc<RwLock<HashMap<String, RunningTask>>>,
    queue_snapshot: Arc<RwLock<Vec<QueuedTaskInfo>>>,
    log_store: Arc<dyn DispatcherLogStore>,
}

enum DispatcherCommand {
    Submit {
        task: AgentTask,
        result_tx: oneshot::Sender<TaskResult>,
    },
    Cancel { task_id: String },
}

pub struct TaskResult {
    pub task_id: String,
    pub status: TaskStatus,
    pub output: Option<AgentOutput>,
    pub error: Option<String>,
}
```

**Run loop:**
1. Receive `Submit` command
2. Dedup check (by `dedup_key` — check running + queue)
3. Insert into `BinaryHeap<PrioritizedTask>`
4. `try_dispatch()`: pop tasks whose session is not busy, spawn each
5. On task finish: remove from running, record to LogStore, emit metrics, `try_dispatch()` again

**Session-parallel dispatch:**
- Maintain `busy_sessions: HashSet<String>` from running tasks
- Only dispatch tasks whose `session_key` is not in `busy_sessions`
- Different sessions can run concurrently

**`submit()` returns `oneshot::Receiver<TaskResult>`** — callers can await or fire-and-forget.

## LogStore Abstraction

### `crates/agents/src/dispatcher/log_store.rs`

```rust
#[async_trait]
pub trait DispatcherLogStore: Send + Sync + 'static {
    async fn append(&self, record: TaskRecord);
    async fn query(&self, filter: LogFilter) -> Vec<TaskRecord>;
    async fn stats(&self) -> DispatcherStats;
}

pub struct LogFilter {
    pub limit: usize,
    pub kind: Option<AgentTaskKind>,
    pub status: Option<TaskStatus>,
    pub since: Option<jiff::Timestamp>,
}

pub struct DispatcherStats {
    pub total_submitted: u64,
    pub total_completed: u64,
    pub total_errors: u64,
    pub total_deduped: u64,
    pub total_cancelled: u64,
    pub uptime_seconds: u64,
}

/// Default in-memory ring buffer implementation.
pub struct InMemoryLogStore {
    records: RwLock<VecDeque<TaskRecord>>,
    capacity: usize,                    // default 200
    counters: AtomicCounters,
    started_at: jiff::Timestamp,
}
```

## Metrics

### `crates/agents/src/dispatcher/metrics.rs`

Follow existing `LazyLock` pattern from `common_worker::metrics`:

```rust
// Counters
pub static DISPATCHER_TASKS_SUBMITTED:  LazyLock<IntCounterVec>  // [kind, priority]
pub static DISPATCHER_TASKS_COMPLETED:  LazyLock<IntCounterVec>  // [kind, status]
pub static DISPATCHER_TASKS_DEDUPED:    LazyLock<IntCounterVec>  // [kind]

// Gauges
pub static DISPATCHER_QUEUE_SIZE:       LazyLock<IntGaugeVec>    // [priority]
pub static DISPATCHER_RUNNING_TASKS:    LazyLock<IntGaugeVec>    // [kind, session]

// Histograms
pub static DISPATCHER_TASK_DURATION:    LazyLock<HistogramVec>   // [kind]
pub static DISPATCHER_QUEUE_WAIT:       LazyLock<HistogramVec>   // [kind]
```

Tracing spans on `execute_task()` with fields: `task_id`, `task_kind`, `session`, `priority`.

## REST API

### `crates/agents/src/dispatcher/router.rs`

```
GET  /api/dispatcher/status     → DispatcherStatus (running + queued + stats)
GET  /api/dispatcher/history    → Vec<TaskRecord>  (?limit=50&kind=&status=&since=)
POST /api/dispatcher/cancel/:id → { success: bool }
```

```rust
#[derive(Serialize)]
pub struct DispatcherStatus {
    pub running: Vec<RunningTaskInfo>,
    pub queued: Vec<QueuedTaskInfo>,
    pub stats: DispatcherStats,
}
```

## Worker Refactoring

### ProactiveAgentWorker (proactive.rs)

Before: directly calls `ProactiveAgent::run()`
After: collects activity summary, builds `AgentTask`, submits to dispatcher

```rust
// Key change in work():
let task = AgentTask::builder()
    .kind(AgentTaskKind::Proactive)
    .priority(Priority::Low)
    .session_key(PROACTIVE_SESSION_KEY)
    .message(activity_summary)
    .history(history)
    .dedup_key("proactive".to_owned())
    .build();
let result_rx = state.dispatcher.submit(task).await?;
// Fire-and-forget: worker doesn't need to wait for completion
```

The dispatcher's `execute_task` handles:
- Creating ProactiveAgent and running it
- Persisting session messages
- Logging outcome

### AgentSchedulerWorker (scheduled_agent.rs)

Before: iterates due jobs, calls `ScheduledAgent::run()` for each
After: submits each due job as a separate `AgentTask`

```rust
for job in &due_jobs {
    let task = AgentTask::builder()
        .kind(AgentTaskKind::Scheduled { job_id: job.id.clone() })
        .priority(Priority::Normal)
        .session_key(&job.session_key)
        .message(job.message.clone())
        .history(history)
        .dedup_key(format!("scheduled:{}", job.id))
        .build();
    state.dispatcher.submit(task).await.ok();
}
```

The dispatcher's `execute_task` handles:
- Creating ScheduledAgent and running it
- Persisting session messages
- Calling `scheduler.mark_executed()`

### AppState Changes

```rust
pub struct AppState {
    // ... existing fields ...
    pub dispatcher: Arc<AgentDispatcher>,   // NEW
}
```

### app/src/lib.rs Changes

```rust
// After orchestrator creation, before workers:
let dispatcher = AgentDispatcher::new(
    orchestrator.clone(),
    chat_service.clone(),
    agent_scheduler.clone(),
    Arc::new(InMemoryLogStore::new(200)),
);
let dispatcher = Arc::new(dispatcher);
dispatcher.spawn_run_loop();  // starts the mpsc recv loop as tokio task

// Merge dispatcher routes
router = router.merge(dispatcher.routes());

// Workers now receive dispatcher via AppState
```

## Notification Dedup (v2)

For v1, session-level serialization already prevents most duplicates.
Cross-session notification dedup (e.g., ProactiveAgent and ScheduledAgent both
sending "reminder X" to the same user) is deferred to v2. When needed, it can
be implemented as a time-windowed dedup layer on `NotifyClient`.

## Files to Create

| File | Description |
|------|-------------|
| `crates/agents/src/dispatcher/mod.rs` | Module declarations |
| `crates/agents/src/dispatcher/types.rs` | AgentTask, Priority, TaskRecord, etc. |
| `crates/agents/src/dispatcher/core.rs` | AgentDispatcher + run loop |
| `crates/agents/src/dispatcher/log_store.rs` | Trait + InMemoryLogStore |
| `crates/agents/src/dispatcher/metrics.rs` | Prometheus metrics |
| `crates/agents/src/dispatcher/error.rs` | DispatcherError (snafu) |
| `crates/agents/src/dispatcher/router.rs` | REST API routes |

## Files to Modify

| File | Change |
|------|--------|
| `crates/agents/src/lib.rs` | Add `pub mod dispatcher;` |
| `crates/agents/Cargo.toml` | Add deps (prometheus, axum, utoipa, etc.) |
| `crates/workers/src/proactive.rs` | Submit to dispatcher instead of direct run |
| `crates/workers/src/scheduled_agent.rs` | Submit to dispatcher instead of direct run |
| `crates/workers/src/worker_state.rs` | Add `dispatcher` field to AppState |
| `crates/app/src/lib.rs` | Init dispatcher, spawn loop, merge routes |

## Frontend (separate issue)

New "Agent Dispatcher" page:
- Stats cards (submitted, completed, errors, deduped)
- Running tasks table (kind, session, priority, elapsed)
- Queue table (kind, session, priority, waiting)
- History table with filters (kind, status, time range)
- Cancel button for running tasks

API types in `web/src/api/types.ts`, fetch functions in `web/src/api/client.ts`.
