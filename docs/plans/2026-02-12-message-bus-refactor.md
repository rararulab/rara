# Message Bus Refactor: Coordinator + Command Trait

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace the current message_bus module with a type-safe, actor-style Coordinator that dispatches PGMQ messages to self-describing Command handlers via type erasure.

**Architecture:** `Coordinator<S>` holds an `Arc<S>` (e.g., AppState) and a registry of type-erased handlers keyed by command name. Each `Command` trait impl declares its queue, name, and max retries. `S: CommandHandler<C>` provides the handle logic. Producer sends typed commands. Worker loops poll per-queue and dispatch via command name routing.

**Tech Stack:** pgmq 0.31, sqlx (PgPool), tokio + tokio-util (JoinSet, CancellationToken), snafu, serde, jiff, tracing

---

## Overview

### Files to delete
- `crates/domain/shared/src/message_bus/types.rs`
- `crates/domain/shared/src/message_bus/consumer.rs`
- `crates/domain/shared/src/message_bus/worker.rs`

### Files to create/rewrite
- `crates/domain/shared/src/message_bus/mod.rs` — module declarations + re-exports
- `crates/domain/shared/src/message_bus/error.rs` — simplified MessageBusError (snafu)
- `crates/domain/shared/src/message_bus/command.rs` — Command trait, CommandHandler trait, Envelope, DeadLetterEnvelope
- `crates/domain/shared/src/message_bus/coordinator.rs` — Coordinator<S>, QueueConfig, worker loop
- `crates/domain/shared/src/message_bus/producer.rs` — typed Producer

### Cargo.toml changes
- Remove: `async-trait`, `backon`, `validator`
- Keep: `pgmq`, `snafu`, `serde`, `serde_json`, `jiff`, `uuid`, `sqlx`, `tokio`, `tokio-util`, `tracing`

---

### Task 1: Delete old files and update mod.rs

**Files:**
- Delete: `crates/domain/shared/src/message_bus/types.rs`
- Delete: `crates/domain/shared/src/message_bus/consumer.rs`
- Delete: `crates/domain/shared/src/message_bus/worker.rs`
- Modify: `crates/domain/shared/src/message_bus/mod.rs`

**Step 1: Delete old files**

```bash
rm crates/domain/shared/src/message_bus/types.rs
rm crates/domain/shared/src/message_bus/consumer.rs
rm crates/domain/shared/src/message_bus/worker.rs
```

**Step 2: Rewrite mod.rs**

```rust
pub mod command;
pub mod coordinator;
pub mod error;
pub mod producer;
```

**Step 3: Verify workspace compiles (expect errors — producer.rs still references old types)**

No commit yet — the crate won't compile until all files are rewritten.

---

### Task 2: Write error.rs

**Files:**
- Rewrite: `crates/domain/shared/src/message_bus/error.rs`

**Step 1: Write error.rs**

```rust
use snafu::Snafu;

#[derive(Debug, Snafu)]
pub enum MessageBusError {
    #[snafu(display("serialization error: {source}"))]
    Serialization { source: serde_json::Error },

    #[snafu(display("queue error: {message}"))]
    Queue { message: String },

    #[snafu(display("handler error: {message}"))]
    Handler { message: String },
}
```

---

### Task 3: Write command.rs — traits + envelope types

**Files:**
- Create: `crates/domain/shared/src/message_bus/command.rs`

**Step 1: Write command.rs**

```rust
use std::future::Future;

use jiff::Timestamp;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use uuid::Uuid;

use crate::message_bus::error::MessageBusError;

/// Self-describing command stored in PGMQ.
///
/// Each command type declares its routing name, target queue, and retry policy.
/// Domain crates define concrete command structs and implement this trait.
pub trait Command: Serialize + DeserializeOwned + Send + 'static {
    /// Routing key used to dispatch messages (e.g., `"crawl_job"`).
    const NAME: &'static str;
    /// PGMQ queue name (e.g., `"task_queue"`).
    const QUEUE: &'static str;
    /// Maximum delivery attempts before dead-lettering. Default: 3.
    const MAX_RETRIES: i32 = 3;
}

/// Handler trait implemented by state holders (e.g., `AppState`).
///
/// The coordinator calls `state.handle(command)` after deserializing the
/// raw PGMQ payload into the concrete command type.
///
/// - `Ok(())` → message is acked (archived).
/// - `Err(_)` → retried up to `Command::MAX_RETRIES`, then dead-lettered.
pub trait CommandHandler<C: Command>: Send + Sync + 'static {
    fn handle(&self, command: C) -> impl Future<Output = Result<(), MessageBusError>> + Send;
}

/// Wire-format envelope stored in PGMQ queues.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope {
    pub id: Uuid,
    /// Routing key matching [`Command::NAME`].
    pub command: String,
    /// Serialized command payload.
    pub payload: serde_json::Value,
    pub created_at: Timestamp,
    pub max_retries: i32,
}

/// Envelope written to dead-letter queues on terminal failure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeadLetterEnvelope {
    pub source_queue: String,
    pub reason: String,
    pub failed_at: Timestamp,
    pub envelope: Envelope,
}
```

---

### Task 4: Write coordinator.rs — type-erased dispatch + worker loops

**Files:**
- Create: `crates/domain/shared/src/message_bus/coordinator.rs`

**Step 1: Write coordinator.rs**

```rust
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use jiff::Timestamp;
use pgmq::PGMQueue;
use sqlx::PgPool;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::message_bus::command::{Command, CommandHandler, DeadLetterEnvelope, Envelope};
use crate::message_bus::error::MessageBusError;

// ── type erasure ────────────────────────────────────────────────────────

type ErasedHandler = Arc<
    dyn Fn(serde_json::Value) -> Pin<Box<dyn Future<Output = Result<(), MessageBusError>> + Send>>
        + Send
        + Sync,
>;

struct Registration {
    queue: &'static str,
    max_retries: i32,
    handler: ErasedHandler,
}

// ── queue config ────────────────────────────────────────────────────────

/// Per-queue polling configuration.
#[derive(Debug, Clone)]
pub struct QueueConfig {
    pub batch_size: i32,
    pub visibility_timeout_seconds: i32,
    pub poll_interval: Duration,
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self {
            batch_size: 10,
            visibility_timeout_seconds: 30,
            poll_interval: Duration::from_secs(1),
        }
    }
}

// ── coordinator ─────────────────────────────────────────────────────────

/// Central message bus coordinator.
///
/// Holds a shared state `S` (e.g., `AppState`) and a registry of
/// type-erased command handlers. Call [`register`] for each command type,
/// optionally configure per-queue settings, then [`run`] to start all
/// worker loops.
pub struct Coordinator<S> {
    state: Arc<S>,
    pgmq: PGMQueue,
    commands: HashMap<&'static str, Registration>,
    queue_configs: HashMap<&'static str, QueueConfig>,
}

impl<S: Send + Sync + 'static> Coordinator<S> {
    /// Create a new coordinator backed by the given state and PgPool.
    pub async fn new(state: S, pool: PgPool) -> Self {
        let pgmq = PGMQueue::new_with_pool(pool).await;
        Self {
            state: Arc::new(state),
            pgmq,
            commands: HashMap::new(),
            queue_configs: HashMap::new(),
        }
    }

    /// Register a command type.
    ///
    /// The compiler enforces `S: CommandHandler<C>` — the state must
    /// implement a handler for this command.
    pub fn register<C: Command>(&mut self) -> &mut Self
    where
        S: CommandHandler<C>,
    {
        let state = self.state.clone();
        let handler: ErasedHandler = Arc::new(move |raw: serde_json::Value| {
            let state = state.clone();
            Box::pin(async move {
                let cmd: C =
                    serde_json::from_value(raw).map_err(|source| MessageBusError::Serialization {
                        source,
                    })?;
                state.handle(cmd).await
            })
        });
        self.commands.insert(
            C::NAME,
            Registration {
                queue: C::QUEUE,
                max_retries: C::MAX_RETRIES,
                handler,
            },
        );
        self
    }

    /// Override default queue configuration for a specific queue.
    pub fn queue_config(&mut self, queue: &'static str, config: QueueConfig) -> &mut Self {
        self.queue_configs.insert(queue, config);
        self
    }

    /// Start all worker loops and block until `cancel` is triggered.
    pub async fn run(self, cancel: CancellationToken) -> Result<(), MessageBusError> {
        let Self {
            pgmq,
            commands,
            queue_configs,
            ..
        } = self;

        // Group handlers by queue.
        let mut queues: HashMap<&str, HashMap<String, (ErasedHandler, i32)>> = HashMap::new();
        for (name, reg) in commands {
            queues
                .entry(reg.queue)
                .or_default()
                .insert(name.to_owned(), (reg.handler, reg.max_retries));
        }

        // Create all queues + DLQs.
        for &queue_name in queues.keys() {
            pgmq.create(queue_name)
                .await
                .map_err(|e| MessageBusError::Queue {
                    message: format!("create queue '{queue_name}': {e}"),
                })?;
            let dlq = format!("{queue_name}_dlq");
            pgmq.create(&dlq)
                .await
                .map_err(|e| MessageBusError::Queue {
                    message: format!("create dlq '{dlq}': {e}"),
                })?;
        }

        let mut join_set = JoinSet::new();

        for (queue_name, handlers) in queues {
            let pgmq = pgmq.clone();
            let token = cancel.clone();
            let config = queue_configs
                .get(queue_name)
                .cloned()
                .unwrap_or_default();
            let queue_name = queue_name.to_owned();

            join_set.spawn(async move {
                queue_worker_loop(pgmq, queue_name, handlers, config, token).await;
            });
        }

        while let Some(result) = join_set.join_next().await {
            if let Err(e) = result {
                error!("worker task panicked: {e}");
            }
        }

        info!("all message bus workers stopped");
        Ok(())
    }
}

// ── worker loop ─────────────────────────────────────────────────────────

async fn queue_worker_loop(
    pgmq: PGMQueue,
    queue: String,
    handlers: HashMap<String, (ErasedHandler, i32)>,
    config: QueueConfig,
    cancel: CancellationToken,
) {
    let dlq = format!("{queue}_dlq");
    info!(
        queue = %queue,
        commands = ?handlers.keys().collect::<Vec<_>>(),
        "message bus worker started"
    );

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                info!(queue = %queue, "message bus worker shutting down");
                break;
            }
            result = poll_and_process(&pgmq, &queue, &dlq, &handlers, &config) => {
                if let Err(e) = result {
                    error!(queue = %queue, error = %e, "poll cycle failed");
                    tokio::time::sleep(config.poll_interval).await;
                }
            }
        }
    }
}

async fn poll_and_process(
    pgmq: &PGMQueue,
    queue: &str,
    dlq: &str,
    handlers: &HashMap<String, (ErasedHandler, i32)>,
    config: &QueueConfig,
) -> Result<(), MessageBusError> {
    let batch = pgmq
        .read_batch::<Envelope>(queue, Some(config.visibility_timeout_seconds), config.batch_size)
        .await
        .map_err(|e| MessageBusError::Queue {
            message: format!("dequeue from '{queue}': {e}"),
        })?;

    let Some(messages) = batch else {
        tokio::time::sleep(config.poll_interval).await;
        return Ok(());
    };

    if messages.is_empty() {
        tokio::time::sleep(config.poll_interval).await;
        return Ok(());
    }

    for msg in messages {
        let envelope = msg.message;
        let msg_id = msg.msg_id;
        let read_ct = msg.read_ct;

        let Some((handler, max_retries)) = handlers.get(&envelope.command) else {
            warn!(
                queue = %queue,
                command = %envelope.command,
                "unknown command, dead-lettering"
            );
            dead_letter(pgmq, queue, dlq, msg_id, envelope, "unknown_command").await;
            continue;
        };

        match handler(envelope.payload.clone()).await {
            Ok(()) => {
                if let Err(e) = pgmq.archive(queue, msg_id).await {
                    error!(queue = %queue, msg_id, error = %e, "ack failed");
                }
            }
            Err(e) => {
                warn!(
                    queue = %queue,
                    command = %envelope.command,
                    msg_id,
                    read_ct,
                    max_retries,
                    error = %e,
                    "handler failed"
                );
                if read_ct >= *max_retries {
                    dead_letter(pgmq, queue, dlq, msg_id, envelope, &e.to_string()).await;
                }
                // else: leave unacked, VT will make it reappear
            }
        }
    }

    Ok(())
}

async fn dead_letter(
    pgmq: &PGMQueue,
    queue: &str,
    dlq: &str,
    msg_id: i64,
    envelope: Envelope,
    reason: &str,
) {
    let dl = DeadLetterEnvelope {
        source_queue: queue.to_owned(),
        reason: reason.to_owned(),
        failed_at: Timestamp::now(),
        envelope,
    };
    // Send to DLQ first, then ack source — safer than ack-first.
    if let Err(e) = pgmq.send(dlq, &dl).await {
        error!(queue = %queue, dlq = %dlq, msg_id, error = %e, "dead-letter send failed");
        return;
    }
    if let Err(e) = pgmq.archive(queue, msg_id).await {
        error!(queue = %queue, msg_id, error = %e, "dead-letter ack failed");
    }
}
```

---

### Task 5: Rewrite producer.rs — typed command sending

**Files:**
- Rewrite: `crates/domain/shared/src/message_bus/producer.rs`

**Step 1: Write producer.rs**

```rust
use jiff::Timestamp;
use pgmq::PGMQueue;
use sqlx::PgPool;
use uuid::Uuid;

use crate::message_bus::command::{Command, Envelope};
use crate::message_bus::error::MessageBusError;

/// Typed producer for sending commands to their declared queues.
#[derive(Clone)]
pub struct Producer {
    pgmq: PGMQueue,
}

impl Producer {
    pub async fn new(pool: PgPool) -> Self {
        let pgmq = PGMQueue::new_with_pool(pool).await;
        Self { pgmq }
    }

    /// Send a command to its declared queue.
    ///
    /// Returns the PGMQ message ID on success.
    pub async fn send<C: Command>(&self, command: &C) -> Result<i64, MessageBusError> {
        let envelope = Envelope {
            id: Uuid::new_v4(),
            command: C::NAME.to_owned(),
            payload: serde_json::to_value(command).map_err(|source| {
                MessageBusError::Serialization { source }
            })?,
            created_at: Timestamp::now(),
            max_retries: C::MAX_RETRIES,
        };

        let msg_id = self
            .pgmq
            .send(C::QUEUE, &envelope)
            .await
            .map_err(|e| MessageBusError::Queue {
                message: format!("send to '{}': {e}", C::QUEUE),
            })?;

        Ok(msg_id)
    }

    /// Ensure a queue exists (idempotent). Only needed if sending
    /// before the Coordinator has started.
    pub async fn ensure_queue(&self, queue_name: &str) -> Result<(), MessageBusError> {
        self.pgmq
            .create(queue_name)
            .await
            .map_err(|e| MessageBusError::Queue {
                message: format!("create queue '{queue_name}': {e}"),
            })
    }
}
```

---

### Task 6: Clean up Cargo.toml — remove unused deps

**Files:**
- Modify: `crates/domain/shared/Cargo.toml`

**Step 1: Remove unused dependencies**

Remove these lines:
- `async-trait.workspace = true`
- `backon.workspace = true`
- `validator = { workspace = true, features = ["derive"] }`

**Step 2: Verify build**

```bash
cargo check -p job-domain-shared
```

Expected: compiles with no errors.

**Step 3: Commit**

```bash
git add -A && git commit -m "refactor(message-bus): rewrite as Coordinator + Command trait

- Replace producer/consumer/worker split with Coordinator<S> + typed dispatch
- Command trait: self-describing (NAME, QUEUE, MAX_RETRIES)
- CommandHandler<C>: state holders implement per-command handling
- Type-erased registry for multi-command per-queue dispatch
- Producer: typed send<C: Command> API
- Remove async-trait, backon, validator deps"
```

---

### Task 7 (optional): Integration test with testcontainers + pgmq

**Files:**
- Create: `crates/domain/shared/src/message_bus/tests.rs` (or inline `#[cfg(test)]` in coordinator.rs)

**Note:** This requires a Docker image with the `pgmq` PostgreSQL extension (e.g., `quay.io/tembo/pgmq-pg17`). If the team's testcontainers setup doesn't include pgmq, skip this task and test via the app's end-to-end flow instead.

The test would:
1. Start a pgmq-enabled Postgres container
2. Create a `Coordinator<TestState>` + `Producer`
3. Send a command via Producer
4. Run Coordinator briefly, verify handler was called
5. Verify message was acked (no longer in queue)
