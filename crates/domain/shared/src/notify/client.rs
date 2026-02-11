// Copyright 2026 Crrow
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

//! Queue client for notifications.

use jiff::Timestamp;
use pgmq::PGMQueue;
use sqlx::PgPool;
use uuid::Uuid;

use crate::notify::{
    error::NotifyError,
    types::{
        DequeuedTelegramNotification, QueuedTelegramNotification, SendTelegramNotificationRequest,
    },
};

/// PGMQ queue name dedicated to telegram notification tasks.
pub const TELEGRAM_NOTIFY_QUEUE_NAME: &str = "notification_telegram_dispatch";

#[derive(Clone)]
pub struct NotifyClient {
    queue: PGMQueue,
}

impl NotifyClient {
    /// Create client from an existing postgres pool and ensure queue exists.
    pub async fn new(pool: PgPool) -> Result<Self, NotifyError> {
        let queue = PGMQueue::new_with_pool(pool).await;
        queue
            .create(TELEGRAM_NOTIFY_QUEUE_NAME)
            .await
            .map_err(|e| NotifyError::RepositoryError {
                message: format!("failed to create telegram notify queue: {e}"),
            })?;
        Ok(Self { queue })
    }

    /// Enqueue one telegram notification task.
    pub async fn send_telegram(
        &self,
        request: SendTelegramNotificationRequest,
    ) -> Result<QueuedTelegramNotification, NotifyError> {
        if request.body.trim().is_empty() {
            return Err(NotifyError::ValidationError {
                message: "body must not be empty".to_owned(),
            });
        }

        let payload = QueuedTelegramNotification {
            id:             Uuid::new_v4(),
            chat_id:        request.chat_id,
            subject:        request.subject,
            body:           request.body,
            priority:       request.priority,
            max_retries:    if request.max_retries <= 0 {
                3
            } else {
                request.max_retries
            },
            reference_type: request.reference_type,
            reference_id:   request.reference_id,
            metadata:       request.metadata,
            created_at:     Timestamp::now(),
        };

        self.queue
            .send(TELEGRAM_NOTIFY_QUEUE_NAME, &payload)
            .await
            .map_err(|e| NotifyError::RepositoryError {
                message: format!(
                    "failed to enqueue telegram notification {}: {e}",
                    payload.id
                ),
            })?;

        Ok(payload)
    }

    /// Read one telegram queue batch and set visibility timeout.
    pub async fn dequeue_telegram_batch(
        &self,
        limit: i32,
        vt_seconds: i32,
    ) -> Result<Vec<DequeuedTelegramNotification>, NotifyError> {
        let Some(messages) = self
            .queue
            .read_batch::<QueuedTelegramNotification>(
                TELEGRAM_NOTIFY_QUEUE_NAME,
                Some(vt_seconds),
                limit,
            )
            .await
            .map_err(|e| NotifyError::RepositoryError {
                message: format!("failed to dequeue telegram notify batch: {e}"),
            })?
        else {
            return Ok(Vec::new());
        };

        Ok(messages
            .into_iter()
            .map(|m| DequeuedTelegramNotification {
                msg_id:  m.msg_id,
                read_ct: m.read_ct,
                payload: m.message,
            })
            .collect())
    }

    /// Ack (archive) a processed telegram message.
    pub async fn ack_telegram(&self, msg_id: i64) -> Result<(), NotifyError> {
        self.queue
            .archive(TELEGRAM_NOTIFY_QUEUE_NAME, msg_id)
            .await
            .map_err(|e| NotifyError::RepositoryError {
                message: format!("failed to ack telegram notify message {msg_id}: {e}"),
            })?;
        Ok(())
    }
}
