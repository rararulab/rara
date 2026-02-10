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

//! PostgreSQL-backed implementation of
//! [`crate::repository::NotificationRepository`].

use std::fmt::Write;

use async_trait::async_trait;
use job_model::notify::NotificationLog;
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    error::NotifyError,
    types::{Notification, NotificationFilter, NotificationStatistics},
};

/// PostgreSQL implementation of the notification repository.
pub struct PgNotificationRepository {
    pool: PgPool,
}

impl PgNotificationRepository {
    /// Create a new repository backed by the given connection pool.
    #[must_use]
    pub fn new(pool: PgPool) -> Self { Self { pool } }
}

/// Map a `sqlx::Error` into a `NotifyError::RepositoryError`.
fn map_err(e: sqlx::Error) -> NotifyError {
    NotifyError::RepositoryError {
        message: e.to_string(),
    }
}

#[async_trait]
impl crate::repository::NotificationRepository for PgNotificationRepository {
    async fn save(&self, notification: &Notification) -> Result<Notification, NotifyError> {
        let store: NotificationLog = notification.clone().into();

        let row = sqlx::query_as::<_, NotificationLog>(
            r#"INSERT INTO notification_log
                   (id, channel, recipient, subject, body, status,
                    priority, retry_count, max_retries, error_message,
                    reference_type, reference_id, metadata, trace_id,
                    sent_at, created_at)
               VALUES
                   ($1, $2, $3, $4, $5, $6, $7,
                    $8, $9, $10, $11, $12, $13, $14, $15, $16)
               RETURNING *"#,
        )
        .bind(store.id)
        .bind(&store.channel)
        .bind(&store.recipient)
        .bind(&store.subject)
        .bind(&store.body)
        .bind(&store.status)
        .bind(&store.priority)
        .bind(store.retry_count)
        .bind(store.max_retries)
        .bind(&store.error_message)
        .bind(&store.reference_type)
        .bind(store.reference_id)
        .bind(&store.metadata)
        .bind(&store.trace_id)
        .bind(store.sent_at)
        .bind(store.created_at)
        .fetch_one(&self.pool)
        .await
        .map_err(map_err)?;

        Ok(row.into())
    }

    async fn find_by_id(&self, id: Uuid) -> Result<Option<Notification>, NotifyError> {
        let row =
            sqlx::query_as::<_, NotificationLog>("SELECT * FROM notification_log WHERE id = $1")
                .bind(id)
                .fetch_optional(&self.pool)
                .await
                .map_err(map_err)?;

        Ok(row.map(Into::into))
    }

    async fn find_all(
        &self,
        filter: &NotificationFilter,
    ) -> Result<Vec<Notification>, NotifyError> {
        let mut sql = String::from("SELECT * FROM notification_log WHERE 1=1");

        if let Some(ref channel) = filter.channel {
            let channel_code = *channel as u8 as i16;
            let _ = write!(sql, " AND channel = {channel_code}");
        }

        if let Some(ref status) = filter.status {
            let status_code = *status as u8 as i16;
            let _ = write!(sql, " AND status = {status_code}");
        }

        if let Some(ref recipient) = filter.recipient {
            let _ = write!(sql, " AND recipient = '{recipient}'");
        }

        if let Some(ref reference_type) = filter.reference_type {
            let _ = write!(sql, " AND reference_type = '{reference_type}'");
        }

        if let Some(ref reference_id) = filter.reference_id {
            let _ = write!(sql, " AND reference_id = '{reference_id}'");
        }

        if let Some(ref created_after) = filter.created_after {
            let _ = write!(sql, " AND created_at >= '{created_after}'");
        }

        if let Some(ref created_before) = filter.created_before {
            let _ = write!(sql, " AND created_at <= '{created_before}'");
        }

        sql.push_str(" ORDER BY created_at DESC");

        let rows = sqlx::query_as::<_, NotificationLog>(&sql)
            .fetch_all(&self.pool)
            .await
            .map_err(map_err)?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn update(&self, notification: &Notification) -> Result<Notification, NotifyError> {
        let store: NotificationLog = notification.clone().into();

        let row = sqlx::query_as::<_, NotificationLog>(
            r#"UPDATE notification_log
               SET channel = $2,
                   recipient = $3,
                   subject = $4,
                   body = $5,
                   status = $6,
                   priority = $7,
                   retry_count = $8,
                   max_retries = $9,
                   error_message = $10,
                   reference_type = $11,
                   reference_id = $12,
                   metadata = $13,
                   trace_id = $14,
                   sent_at = $15
               WHERE id = $1
               RETURNING *"#,
        )
        .bind(store.id)
        .bind(&store.channel)
        .bind(&store.recipient)
        .bind(&store.subject)
        .bind(&store.body)
        .bind(&store.status)
        .bind(&store.priority)
        .bind(store.retry_count)
        .bind(store.max_retries)
        .bind(&store.error_message)
        .bind(&store.reference_type)
        .bind(store.reference_id)
        .bind(&store.metadata)
        .bind(&store.trace_id)
        .bind(store.sent_at)
        .fetch_one(&self.pool)
        .await
        .map_err(map_err)?;

        Ok(row.into())
    }

    async fn find_pending(&self, limit: i64) -> Result<Vec<Notification>, NotifyError> {
        let pending_status = crate::types::NotificationStatus::Pending as u8 as i16;
        let retrying_status = crate::types::NotificationStatus::Retrying as u8 as i16;

        let rows = sqlx::query_as::<_, NotificationLog>(
            r#"SELECT * FROM notification_log
               WHERE status IN ($1, $2)
               ORDER BY priority DESC, created_at ASC
               LIMIT $3"#,
        )
        .bind(pending_status)
        .bind(retrying_status)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn mark_sent(&self, id: Uuid) -> Result<(), NotifyError> {
        let sent_status = crate::types::NotificationStatus::Sent as u8 as i16;
        let result = sqlx::query(
            r#"UPDATE notification_log
               SET status = $2, sent_at = now()
               WHERE id = $1"#,
        )
        .bind(id)
        .bind(sent_status)
        .execute(&self.pool)
        .await
        .map_err(map_err)?;

        if result.rows_affected() == 0 {
            return Err(NotifyError::NotFound { id });
        }
        Ok(())
    }

    async fn mark_failed(&self, id: Uuid, error: &str) -> Result<(), NotifyError> {
        let failed_status = crate::types::NotificationStatus::Failed as u8 as i16;
        let result = sqlx::query(
            r#"UPDATE notification_log
               SET status = $3, error_message = $2
               WHERE id = $1"#,
        )
        .bind(id)
        .bind(error)
        .bind(failed_status)
        .execute(&self.pool)
        .await
        .map_err(map_err)?;

        if result.rows_affected() == 0 {
            return Err(NotifyError::NotFound { id });
        }
        Ok(())
    }

    async fn increment_retry(&self, id: Uuid) -> Result<Notification, NotifyError> {
        let retrying_status = crate::types::NotificationStatus::Retrying as u8 as i16;
        let row = sqlx::query_as::<_, NotificationLog>(
            r#"UPDATE notification_log
               SET retry_count = retry_count + 1,
                   status = $2
               WHERE id = $1
               RETURNING *"#,
        )
        .bind(id)
        .bind(retrying_status)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_err)?;

        match row {
            Some(r) => Ok(r.into()),
            None => Err(NotifyError::NotFound { id }),
        }
    }

    async fn get_statistics(&self) -> Result<NotificationStatistics, NotifyError> {
        let pending_status = crate::types::NotificationStatus::Pending as u8 as i16;
        let sent_status = crate::types::NotificationStatus::Sent as u8 as i16;
        let failed_status = crate::types::NotificationStatus::Failed as u8 as i16;
        let retrying_status = crate::types::NotificationStatus::Retrying as u8 as i16;

        let row: (i64, i64, i64, i64, i64) = sqlx::query_as(
            r#"SELECT
                   COUNT(*) AS total,
                   COUNT(*) FILTER (WHERE status = $1) AS pending,
                   COUNT(*) FILTER (WHERE status = $2) AS sent,
                   COUNT(*) FILTER (WHERE status = $3) AS failed,
                   COUNT(*) FILTER (WHERE status = $4) AS retrying
               FROM notification_log"#,
        )
        .bind(pending_status)
        .bind(sent_status)
        .bind(failed_status)
        .bind(retrying_status)
        .fetch_one(&self.pool)
        .await
        .map_err(map_err)?;

        Ok(NotificationStatistics {
            total:    row.0,
            pending:  row.1,
            sent:     row.2,
            failed:   row.3,
            retrying: row.4,
        })
    }
}
