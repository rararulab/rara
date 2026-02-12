// Copyright 2025 Crrow
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//      http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Coordinator that groups command handlers by queue and runs worker loops.

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
    max_attempts: i32,
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
/// type-erased command handlers. Call [`Coordinator::register`] for each
/// command type, optionally configure per-queue settings, then
/// [`Coordinator::run`] to start all worker loops.
pub struct Coordinator<S> {
    state: Arc<S>,
    pgmq: PGMQueue,
    commands: HashMap<&'static str, Registration>,
    queue_configs: HashMap<&'static str, QueueConfig>,
}

impl<S: Send + Sync + 'static> Coordinator<S> {
    /// Create a new coordinator backed by the given shared state and PgPool.
    pub async fn new(state: Arc<S>, pool: PgPool) -> Self {
        let pgmq = PGMQueue::new_with_pool(pool).await;
        Self {
            state,
            pgmq,
            commands: HashMap::new(),
            queue_configs: HashMap::new(),
        }
    }

    /// Register a command type. Panics if a command with the same NAME
    /// is already registered.
    ///
    /// The compiler enforces `S: CommandHandler<C>` — the state must
    /// implement a handler for this command.
    pub fn register<C: Command>(&mut self) -> &mut Self
    where
        S: CommandHandler<C>,
    {
        assert!(
            !self.commands.contains_key(C::NAME),
            "duplicate command registration: {:?}",
            C::NAME
        );
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
                max_attempts: C::MAX_ATTEMPTS,
                handler,
            },
        );
        self
    }

    pub fn queue_config(&mut self, queue: &'static str, config: QueueConfig) -> &mut Self {
        self.queue_configs.insert(queue, config);
        self
    }

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
                .insert(name.to_owned(), (reg.handler, reg.max_attempts));
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

        let Some((handler, max_attempts)) = handlers.get(&envelope.command) else {
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
                    max_attempts,
                    error = %e,
                    "handler failed"
                );
                if read_ct >= *max_attempts {
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
    if let Err(e) = pgmq.send(dlq, &dl).await {
        error!(queue = %queue, dlq = %dlq, msg_id, error = %e, "dead-letter send failed");
        return;
    }
    if let Err(e) = pgmq.archive(queue, msg_id).await {
        error!(queue = %queue, msg_id, error = %e, "dead-letter ack failed");
    }
}
