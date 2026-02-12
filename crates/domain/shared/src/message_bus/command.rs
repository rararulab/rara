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

//! Command trait, CommandHandler trait, Envelope, and DeadLetterEnvelope.

use std::future::Future;

use jiff::Timestamp;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use uuid::Uuid;

use crate::message_bus::error::MessageBusError;

/// Self-describing command stored in PGMQ.
pub trait Command: Serialize + DeserializeOwned + Send + 'static {
    /// Routing key used to dispatch messages (e.g., `"crawl_job"`).
    const NAME: &'static str;
    /// PGMQ queue name (e.g., `"task_queue"`).
    const QUEUE: &'static str;
    /// Maximum total delivery attempts before dead-lettering. Default: 3.
    const MAX_ATTEMPTS: i32 = 3;
}

/// Handler trait implemented by state holders (e.g., `AppState`).
///
/// - `Ok(())` — message is acked (archived).
/// - `Err(_)` — retried up to `Command::MAX_RETRIES`, then dead-lettered.
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
    pub max_attempts: i32,
}

/// Envelope written to dead-letter queues on terminal failure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeadLetterEnvelope {
    pub source_queue: String,
    pub reason: String,
    pub failed_at: Timestamp,
    pub envelope: Envelope,
}
