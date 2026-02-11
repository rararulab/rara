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

//! Queue client for notifications.

use jiff::Timestamp;
use pgmq::PGMQueue;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::notify::{
    error::NotifyError,
    types::{
        DequeuedTelegramNotification, NotificationQueueMessage, NotificationQueueOverview,
        QueueMessageState, QueuedTelegramNotification, SendTelegramNotificationRequest,
    },
};

/// PGMQ queue name dedicated to telegram notification tasks.
pub const TELEGRAM_NOTIFY_QUEUE_NAME: &str = "notification_telegram_dispatch";
const MAX_LIMIT: i64 = 200;

#[derive(Clone)]
pub struct NotifyClient {
    queue: PGMQueue,
}

impl NotifyClient {
    /// Create client from an existing postgres pool and ensure queue exists.
    pub async fn new<P: Into<PgPool>>(pool: P) -> Result<Self, NotifyError> {
        let queue = PGMQueue::new_with_pool(pool.into()).await;
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

    /// Read queue-level counters for telegram notification dispatch.
    pub async fn telegram_overview(&self) -> Result<NotificationQueueOverview, NotifyError> {
        let queue_table = queue_table_name(TELEGRAM_NOTIFY_QUEUE_NAME)?;
        let archive_table = archive_table_name(TELEGRAM_NOTIFY_QUEUE_NAME)?;
        let ready_count = self
            .count_rows_if_exists(&queue_table, Some("vt <= now()"))
            .await?;
        let inflight_count = self
            .count_rows_if_exists(&queue_table, Some("vt > now()"))
            .await?;
        let archived_count = self.count_rows_if_exists(&archive_table, None).await?;
        Ok(NotificationQueueOverview {
            queue_name: TELEGRAM_NOTIFY_QUEUE_NAME.to_owned(),
            ready_count,
            inflight_count,
            archived_count,
        })
    }

    /// List queue messages by state for observability.
    pub async fn list_telegram_messages(
        &self,
        state: QueueMessageState,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<NotificationQueueMessage>, NotifyError> {
        let limit = limit.clamp(1_i64, MAX_LIMIT);
        let offset = offset.max(0_i64);
        let queue_table = queue_table_name(TELEGRAM_NOTIFY_QUEUE_NAME)?;
        let archive_table = archive_table_name(TELEGRAM_NOTIFY_QUEUE_NAME)?;

        let items = match state {
            QueueMessageState::Ready => {
                let rows = sqlx::query(&format!(
                    "SELECT msg_id, read_ct, enqueued_at, vt, message FROM {queue_table} WHERE vt \
                     <= now() ORDER BY msg_id DESC LIMIT $1 OFFSET $2"
                ))
                .bind(limit)
                .bind(offset)
                .fetch_all(&self.queue.connection)
                .await
                .map_err(sql_to_notify_err)?;

                rows.into_iter()
                    .map(|row| queue_row_to_message(row, QueueMessageState::Ready))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(sql_to_notify_err)?
            }
            QueueMessageState::Inflight => {
                let rows = sqlx::query(&format!(
                    "SELECT msg_id, read_ct, enqueued_at, vt, message FROM {queue_table} WHERE vt \
                     > now() ORDER BY msg_id DESC LIMIT $1 OFFSET $2"
                ))
                .bind(limit)
                .bind(offset)
                .fetch_all(&self.queue.connection)
                .await
                .map_err(sql_to_notify_err)?;

                rows.into_iter()
                    .map(|row| queue_row_to_message(row, QueueMessageState::Inflight))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(sql_to_notify_err)?
            }
            QueueMessageState::Archived => {
                if !self.table_exists(&archive_table).await? {
                    return Ok(Vec::new());
                }
                let rows = sqlx::query(&format!(
                    "SELECT msg_id, read_ct, enqueued_at, vt, archived_at, message FROM \
                     {archive_table} ORDER BY msg_id DESC LIMIT $1 OFFSET $2"
                ))
                .bind(limit)
                .bind(offset)
                .fetch_all(&self.queue.connection)
                .await
                .map_err(sql_to_notify_err)?;

                rows.into_iter()
                    .map(archive_row_to_message)
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(sql_to_notify_err)?
            }
        };

        Ok(items)
    }

    async fn table_exists(&self, table_name: &str) -> Result<bool, NotifyError> {
        sqlx::query_scalar::<_, bool>("SELECT to_regclass($1) IS NOT NULL")
            .bind(table_name)
            .fetch_one(&self.queue.connection)
            .await
            .map_err(sql_to_notify_err)
    }

    async fn count_rows_if_exists(
        &self,
        table_name: &str,
        where_clause: Option<&str>,
    ) -> Result<i64, NotifyError> {
        if !self.table_exists(table_name).await? {
            return Ok(0);
        }

        let query = match where_clause {
            Some(predicate) => format!("SELECT COUNT(*) FROM {table_name} WHERE {predicate}"),
            None => format!("SELECT COUNT(*) FROM {table_name}"),
        };
        sqlx::query_scalar::<_, i64>(&query)
            .fetch_one(&self.queue.connection)
            .await
            .map_err(sql_to_notify_err)
    }
}

fn queue_table_name(queue_name: &str) -> Result<String, NotifyError> {
    validate_queue_name(queue_name)?;
    Ok(format!("pgmq.q_{queue_name}"))
}

fn archive_table_name(queue_name: &str) -> Result<String, NotifyError> {
    validate_queue_name(queue_name)?;
    Ok(format!("pgmq.a_{queue_name}"))
}

fn validate_queue_name(queue_name: &str) -> Result<(), NotifyError> {
    let is_valid = !queue_name.is_empty()
        && queue_name
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_');
    if is_valid {
        Ok(())
    } else {
        Err(NotifyError::ValidationError {
            message: format!("invalid queue name: {queue_name}"),
        })
    }
}

fn sql_to_notify_err(err: sqlx::Error) -> NotifyError {
    NotifyError::RepositoryError {
        message: format!("notification queue query failed: {err}"),
    }
}

fn queue_row_to_message(
    row: sqlx::postgres::PgRow,
    state: QueueMessageState,
) -> Result<NotificationQueueMessage, sqlx::Error> {
    Ok(NotificationQueueMessage {
        state,
        msg_id: row.try_get("msg_id")?,
        read_ct: row.try_get("read_ct")?,
        enqueued_at: row.try_get("enqueued_at")?,
        vt: row.try_get("vt")?,
        archived_at: None,
        payload: row.try_get("message")?,
    })
}

fn archive_row_to_message(
    row: sqlx::postgres::PgRow,
) -> Result<NotificationQueueMessage, sqlx::Error> {
    Ok(NotificationQueueMessage {
        state:       QueueMessageState::Archived,
        msg_id:      row.try_get("msg_id")?,
        read_ct:     row.try_get("read_ct")?,
        enqueued_at: row.try_get("enqueued_at")?,
        vt:          row.try_get("vt")?,
        archived_at: row.try_get("archived_at")?,
        payload:     row.try_get("message")?,
    })
}
