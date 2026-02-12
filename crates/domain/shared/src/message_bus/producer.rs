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

//! Typed producer for sending commands to their declared queues.

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

    /// Send a command to its declared queue. Returns PGMQ message ID.
    pub async fn send<C: Command>(&self, command: &C) -> Result<i64, MessageBusError> {
        let envelope = Envelope {
            id: Uuid::new_v4(),
            command: C::NAME.to_owned(),
            payload: serde_json::to_value(command).map_err(|source| {
                MessageBusError::Serialization { source }
            })?,
            created_at: Timestamp::now(),
            max_attempts: C::MAX_ATTEMPTS,
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

    /// Ensure a queue exists (idempotent).
    pub async fn ensure_queue(&self, queue_name: &str) -> Result<(), MessageBusError> {
        self.pgmq
            .create(queue_name)
            .await
            .map_err(|e| MessageBusError::Queue {
                message: format!("create queue '{queue_name}': {e}"),
            })
    }
}
