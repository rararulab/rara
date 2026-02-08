# Worker Abstraction Redesign

## Overview

重构 `rsketch_common_worker` 模块，优化 API 人体工程学、类型安全、功能扩展和架构简化。

## Design Decisions

| Dimension | Decision |
|-----------|----------|
| API Style | Builder pattern for worker configuration |
| Worker Trait | Keep `on_start`, `work`, `on_shutdown`; return `()` |
| Error Handling | Worker handles its own errors; framework doesn't care |
| WorkerContext | Generic state `S` + tracing integration |
| State Scope | Manager-level shared state (`Manager<S>`) |
| WorkerHandle | Trait-based capability (`Handle`, `Pausable`, `Notifiable`) |
| Metrics | Keep Prometheus global metrics |
| Trigger | Extend with Cron + combo triggers |
| Blocking | Builder configuration item |

---

## Part 1: Worker Trait

```rust
/// Core worker trait - only defines behavior, not configuration.
#[async_trait]
pub trait Worker: Send + 'static {
    /// Called once when worker starts. Must not fail.
    async fn on_start(&mut self, ctx: WorkerContext) { }

    /// Core work unit. Called according to trigger schedule.
    async fn work(&mut self, ctx: WorkerContext);

    /// Called once during shutdown. Must not fail.
    async fn on_shutdown(&mut self, ctx: WorkerContext) { }
}
```

Changes:
- Remove `name()`, `trigger()`, `is_blocking()` — moved to Builder
- Remove all `Result` return values — worker handles errors internally
- Remove `where Self: Sized` constraint
- `WorkerContext` passed by value (Clone, cheap)

---

## Part 2: WorkerContext

```rust
/// Context passed to worker on each execution.
/// Clone is cheap (all Arc internally).
#[derive(Clone)]
pub struct WorkerContext<S = ()> {
    state: Arc<S>,
    cancel_token: CancellationToken,
    notify: Arc<Notify>,
    worker_name: &'static str,
}

impl<S> WorkerContext<S> {
    /// Access shared state.
    pub fn state(&self) -> &S { &self.state }
    
    /// Check if cancellation requested.
    pub fn is_cancelled(&self) -> bool { self.cancel_token.is_cancelled() }
    
    /// Wait for cancellation signal.
    pub async fn cancelled(&self) { self.cancel_token.cancelled().await }
    
    /// Wait for notify signal (for Trigger::Notify workers).
    pub async fn notified(&self) { self.notify.notified().await }
    
    /// Get child token for sub-tasks.
    pub fn child_token(&self) -> CancellationToken { 
        self.cancel_token.child_token() 
    }
    
    /// Worker name for logging context.
    pub fn name(&self) -> &'static str { self.worker_name }
}
```

Tracing integration: framework wraps `work()` calls with `info_span!("worker", name = worker_name)`.

---

## Part 3: Trigger

```rust
/// Trigger mechanism for worker execution.
#[derive(Debug, Clone)]
pub enum Trigger {
    /// Execute once immediately on startup, then stop.
    Once,
    
    /// Execute only when explicitly notified via handle.
    Notify,
    
    /// Execute at fixed intervals.
    Interval(Duration),
    
    /// Execute on cron schedule.
    Cron(croner::Cron),
    
    /// Execute at intervals, but can also be triggered manually.
    /// Timer resets after manual trigger.
    IntervalOrNotify(Duration),
    
    /// Execute on cron schedule, but can also be triggered manually.
    CronOrNotify(croner::Cron),
}
```

Use `croner` crate for cron parsing.

---

## Part 4: Handle Traits

```rust
/// Base handle trait - all handles have a name.
pub trait Handle: Clone + Send + Sync {
    fn name(&self) -> &'static str;
}

/// Handle that can be paused and resumed.
pub trait Pausable: Handle {
    fn pause(&self);
    fn resume(&self);
    fn is_paused(&self) -> bool;
}

/// Handle that can be manually triggered.
pub trait Notifiable: Handle {
    fn notify(&self);
}
```

Concrete types:
- `OnceHandle` — implements `Handle`
- `NotifyHandle` — implements `Handle`, `Notifiable`
- `IntervalHandle` — implements `Handle`, `Pausable`
- `CronHandle` — implements `Handle`, `Pausable`
- `IntervalOrNotifyHandle` — implements `Handle`, `Pausable`, `Notifiable`
- `CronOrNotifyHandle` — implements `Handle`, `Pausable`, `Notifiable`

---

## Part 5: Builder Pattern

```rust
impl<S> Manager<S> {
    pub fn worker<W: Worker>(&mut self, worker: W) -> WorkerBuilder<S, W, TriggerNotSet> {
        WorkerBuilder::new(self, worker)
    }
}

// Type-state markers
pub struct TriggerNotSet;
pub struct TriggerOnce;
pub struct TriggerNotify;
pub struct TriggerInterval;
pub struct TriggerCron;
pub struct TriggerIntervalOrNotify;
pub struct TriggerCronOrNotify;
```

Common methods:
```rust
impl<'m, S, W: Worker, T> WorkerBuilder<'m, S, W, T> {
    pub fn name(mut self, name: &'static str) -> Self;
    pub fn blocking(mut self) -> Self;
}
```

Trigger methods (state transitions):
```rust
impl<'m, S, W: Worker> WorkerBuilder<'m, S, W, TriggerNotSet> {
    pub fn once(self) -> WorkerBuilder<'m, S, W, TriggerOnce>;
    pub fn on_notify(self) -> WorkerBuilder<'m, S, W, TriggerNotify>;
    pub fn interval(self, d: Duration) -> WorkerBuilder<'m, S, W, TriggerInterval>;
    pub fn cron(self, expr: &str) -> Result<WorkerBuilder<'m, S, W, TriggerCron>, CronParseError>;
    pub fn interval_or_notify(self, d: Duration) -> WorkerBuilder<'m, S, W, TriggerIntervalOrNotify>;
    pub fn cron_or_notify(self, expr: &str) -> Result<WorkerBuilder<'m, S, W, TriggerCronOrNotify>, CronParseError>;
}
```

Each trigger state has its own `spawn()` returning the correct handle type.

---

## Part 6: Manager

```rust
pub struct Manager<S = ()> {
    state: Arc<S>,
    cancel_token: CancellationToken,
    runtime: Option<Arc<Runtime>>,
    shutdown_timeout: Duration,
    joins: JoinSet<()>,
}

impl Manager<()> {
    pub fn new() -> Self;
    pub fn with_config(config: ManagerConfig) -> Self;
}

impl<S: Send + Sync + 'static> Manager<S> {
    pub fn with_state(state: S) -> Self;
    pub fn with_state_and_config(state: S, config: ManagerConfig) -> Self;
    pub fn worker<W: Worker>(&mut self, worker: W) -> WorkerBuilder<S, W, TriggerNotSet>;
    pub async fn shutdown(mut self);
}

#[derive(Debug, Clone, SmartDefault)]
pub struct ManagerConfig {
    pub runtime: Option<Arc<Runtime>>,
    #[default(Duration::from_secs(30))]
    pub shutdown_timeout: Duration,
}
```

---

## Part 7: TriggerDriver (Internal)

```rust
/// Internal trait for trigger execution strategy.
trait TriggerDriver: Send {
    /// Wait for next execution. Returns false if should stop.
    async fn wait_next(&mut self, ctx: &WorkerContext, notify: &Notify) -> bool;
}
```

Implementations: `OnceDriver`, `NotifyDriver`, `IntervalDriver`, `CronDriver`, `IntervalOrNotifyDriver`, `CronOrNotifyDriver`.

Unified execution loop:
```rust
async fn run_worker<S, W: Worker>(
    mut worker: W,
    ctx: WorkerContext<S>,
    paused: Arc<AtomicBool>,
    mut driver: impl TriggerDriver,
    name: &'static str,
) {
    let span = tracing::info_span!("worker", name);
    let _guard = span.enter();
    
    worker.on_start(ctx.clone()).await;
    
    while driver.wait_next(&ctx, &ctx.notify).await {
        if paused.load(Ordering::Acquire) {
            continue;
        }
        
        let start = Instant::now();
        worker.work(ctx.clone()).await;
        // record metrics
    }
    
    worker.on_shutdown(ctx.clone()).await;
}
```

---

## Part 8: Module Structure

```
crates/common/worker/src/
├── lib.rs              # Public API exports
├── worker.rs           # Worker trait
├── context.rs          # WorkerContext<S>
├── trigger.rs          # Trigger enum
├── handle.rs           # Handle traits + concrete types
├── builder.rs          # WorkerBuilder
├── manager.rs          # Manager<S> + ManagerConfig
├── driver.rs           # TriggerDriver trait + impls (pub(crate))
├── metrics.rs          # Prometheus metrics
└── err.rs              # CronParseError only
```

---

## Dependency Changes

```toml
[dependencies]
# New
croner = "2"
smart_default = "0.7"

# Keep
async-trait = "0.1"
tokio = { version = "1", features = ["sync", "time"] }
tokio-util = "0.7"
prometheus = "0.13"
tracing = "0.1"
lazy_static = "1"
```

---

## Migration Example

Before:
```rust
struct MyWorker;

#[async_trait]
impl Worker for MyWorker {
    fn name() -> &'static str { "my-worker" }
    fn trigger() -> Trigger { Trigger::Interval(Duration::from_secs(60)) }
    async fn work(&mut self, ctx: &WorkerContext) -> Result<()> {
        do_work()?;
        Ok(())
    }
}

let mut manager = Manager::start(WorkerConfig::builder().build())?;
let handle = manager.register(MyWorker);
```

After:
```rust
struct MyWorker;

#[async_trait]
impl Worker for MyWorker {
    async fn work(&mut self, ctx: WorkerContext<()>) {
        if let Err(e) = do_work() {
            tracing::error!(error = ?e, "work failed");
        }
    }
}

let mut manager = Manager::new();
let handle = manager
    .worker(MyWorker)
    .name("my-worker")
    .interval(Duration::from_secs(60))
    .spawn();
```

---

## Breaking Changes

1. `Worker` trait signature completely changed
2. `Manager::start()` → `Manager::new()` / `Manager::with_state()`
3. `manager.register(worker)` → `manager.worker(w).name(...).trigger(...).spawn()`
4. `WorkerHandle` split into multiple concrete types
