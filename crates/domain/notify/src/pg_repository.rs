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
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    db_models,
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
        let store: db_models::NotificationLog = notification.clone().into();

        let row = sqlx::query_as::<_, db_models::NotificationLog>(
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
        let row = sqlx::query_as::<_, db_models::NotificationLog>(
            "SELECT * FROM notification_log WHERE id = $1",
        )
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

        let rows = sqlx::query_as::<_, db_models::NotificationLog>(&sql)
            .fetch_all(&self.pool)
            .await
            .map_err(map_err)?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn update(&self, notification: &Notification) -> Result<Notification, NotifyError> {
        let store: db_models::NotificationLog = notification.clone().into();

        let row = sqlx::query_as::<_, db_models::NotificationLog>(
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

        let rows = sqlx::query_as::<_, db_models::NotificationLog>(
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
        let row = sqlx::query_as::<_, db_models::NotificationLog>(
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use jiff::Timestamp;
    use sqlx::postgres::PgPoolOptions;
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::postgres::Postgres;
    use uuid::Uuid;

    use super::*;
    use crate::{
        repository::NotificationRepository,
        types::{NotificationChannel, NotificationPriority, NotificationStatus},
    };

    async fn connect_pool(url: &str) -> sqlx::PgPool {
        let mut last_err: Option<sqlx::Error> = None;
        for _ in 0..30 {
            match PgPoolOptions::new()
                .max_connections(5)
                .acquire_timeout(Duration::from_secs(10))
                .connect(url)
                .await
            {
                Ok(pool) => return pool,
                Err(e) => {
                    last_err = Some(e);
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
            }
        }
        panic!("failed to connect to postgres: {last_err:?}");
    }

    async fn setup_pool() -> (sqlx::PgPool, testcontainers::ContainerAsync<Postgres>) {
        let container = Postgres::default().start().await.unwrap();
        let host = container.get_host().await.unwrap();
        let port = container.get_host_port_ipv4(5432).await.unwrap();
        let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
        let pool = connect_pool(&url).await;

        // Ensure gen_random_uuid() is available (older PG images need pgcrypto).
        sqlx::raw_sql("CREATE EXTENSION IF NOT EXISTS \"pgcrypto\"")
            .execute(&pool)
            .await
            .unwrap();

        // Run all migrations in order using raw_sql (simple query protocol)
        // which supports multi-statement execution.
        let migrations: &[&str] = &[
            include_str!("../../../common/yunara-store/migrations/20260127000000_init.sql"),
            include_str!(
                "../../../common/yunara-store/migrations/20260208000000_domain_models.sql"
            ),
            include_str!(
                "../../../common/yunara-store/migrations/20260209000000_resume_version_mgmt.sql"
            ),
            include_str!(
                "../../../common/yunara-store/migrations/20260210000000_schema_alignment.sql"
            ),
            include_str!(
                "../../../common/yunara-store/migrations/20260211000000_notify_priority.sql"
            ),
        ];

        for sql in migrations {
            sqlx::raw_sql(sql).execute(&pool).await.unwrap();
        }

        // The scheduler migration references set_updated_at() but the
        // function was created as trigger_set_updated_at() in the domain
        // migration. Fix the reference before executing.
        let scheduler_sql = include_str!(
            "../../../common/yunara-store/migrations/20260211000001_scheduler_tables.sql"
        )
        .replace(
            "FUNCTION set_updated_at()",
            "FUNCTION trigger_set_updated_at()",
        );
        sqlx::raw_sql(&scheduler_sql).execute(&pool).await.unwrap();

        // Convert domain enum columns to SMALLINT codes.
        let domain_int_migrations: &[&str] = &[
            include_str!(
                "../../../common/yunara-store/migrations/20260212000000_application_int_enums.sql"
            ),
            include_str!(
                "../../../common/yunara-store/migrations/20260212000001_interview_int_enums.sql"
            ),
            include_str!(
                "../../../common/yunara-store/migrations/20260212000002_notify_int_enums.sql"
            ),
            include_str!(
                "../../../common/yunara-store/migrations/20260212000003_resume_int_enums.sql"
            ),
            include_str!(
                "../../../common/yunara-store/migrations/20260212000004_scheduler_int_enums.sql"
            ),
        ];
        for sql in domain_int_migrations {
            sqlx::raw_sql(sql).execute(&pool).await.unwrap();
        }

        (pool, container)
    }

    fn make_notification() -> Notification {
        Notification {
            id:             Uuid::new_v4(),
            channel:        NotificationChannel::Telegram,
            recipient:      "user123".into(),
            subject:        Some("Test Subject".into()),
            body:           "Hello, this is a test notification.".into(),
            status:         NotificationStatus::Pending,
            priority:       NotificationPriority::Normal,
            retry_count:    0,
            max_retries:    3,
            error_message:  None,
            reference_type: Some("application".into()),
            reference_id:   Some(Uuid::new_v4()),
            metadata:       None,
            trace_id:       None,
            sent_at:        None,
            created_at:     Timestamp::now(),
        }
    }

    #[tokio::test]
    async fn test_save_and_find_by_id() {
        let (pool, _container) = setup_pool().await;
        let repo = PgNotificationRepository::new(pool);

        let notification = make_notification();
        let saved = repo.save(&notification).await.unwrap();
        assert_eq!(saved.id, notification.id);
        assert_eq!(saved.channel, NotificationChannel::Telegram);
        assert_eq!(saved.recipient, "user123");

        let found = repo.find_by_id(saved.id).await.unwrap();
        assert!(found.is_some());
        let found = found.unwrap();
        assert_eq!(found.id, saved.id);
        assert_eq!(found.body, "Hello, this is a test notification.");
    }

    #[tokio::test]
    async fn test_find_by_id_not_found() {
        let (pool, _container) = setup_pool().await;
        let repo = PgNotificationRepository::new(pool);

        let found = repo.find_by_id(Uuid::new_v4()).await.unwrap();
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn test_find_all_with_filters() {
        let (pool, _container) = setup_pool().await;
        let repo = PgNotificationRepository::new(pool);

        let mut n1 = make_notification();
        n1.channel = NotificationChannel::Telegram;
        n1.recipient = "alice".into();
        repo.save(&n1).await.unwrap();

        let mut n2 = make_notification();
        n2.channel = NotificationChannel::Email;
        n2.recipient = "bob".into();
        repo.save(&n2).await.unwrap();

        // Filter by channel
        let filter = NotificationFilter {
            channel: Some(NotificationChannel::Telegram),
            ..Default::default()
        };
        let results = repo.find_all(&filter).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].recipient, "alice");

        // Filter by recipient
        let filter = NotificationFilter {
            recipient: Some("bob".into()),
            ..Default::default()
        };
        let results = repo.find_all(&filter).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].channel, NotificationChannel::Email);

        // No filter - get all
        let all = repo.find_all(&NotificationFilter::default()).await.unwrap();
        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn test_update() {
        let (pool, _container) = setup_pool().await;
        let repo = PgNotificationRepository::new(pool);

        let notification = make_notification();
        let saved = repo.save(&notification).await.unwrap();

        let mut updated = saved.clone();
        updated.subject = Some("Updated Subject".into());
        updated.priority = NotificationPriority::Urgent;

        let result = repo.update(&updated).await.unwrap();
        assert_eq!(result.subject, Some("Updated Subject".into()));
        assert_eq!(result.priority, NotificationPriority::Urgent);
    }

    #[tokio::test]
    async fn test_mark_sent() {
        let (pool, _container) = setup_pool().await;
        let repo = PgNotificationRepository::new(pool);

        let notification = make_notification();
        let saved = repo.save(&notification).await.unwrap();

        repo.mark_sent(saved.id).await.unwrap();

        let found = repo.find_by_id(saved.id).await.unwrap().unwrap();
        assert_eq!(found.status, NotificationStatus::Sent);
        assert!(found.sent_at.is_some());
    }

    #[tokio::test]
    async fn test_mark_sent_not_found() {
        let (pool, _container) = setup_pool().await;
        let repo = PgNotificationRepository::new(pool);

        let result = repo.mark_sent(Uuid::new_v4()).await;
        assert!(matches!(result, Err(NotifyError::NotFound { .. })));
    }

    #[tokio::test]
    async fn test_mark_failed() {
        let (pool, _container) = setup_pool().await;
        let repo = PgNotificationRepository::new(pool);

        let notification = make_notification();
        let saved = repo.save(&notification).await.unwrap();

        repo.mark_failed(saved.id, "connection refused")
            .await
            .unwrap();

        let found = repo.find_by_id(saved.id).await.unwrap().unwrap();
        assert_eq!(found.status, NotificationStatus::Failed);
        assert_eq!(found.error_message, Some("connection refused".into()));
    }

    #[tokio::test]
    async fn test_mark_failed_not_found() {
        let (pool, _container) = setup_pool().await;
        let repo = PgNotificationRepository::new(pool);

        let result = repo.mark_failed(Uuid::new_v4(), "error").await;
        assert!(matches!(result, Err(NotifyError::NotFound { .. })));
    }

    #[tokio::test]
    async fn test_increment_retry() {
        let (pool, _container) = setup_pool().await;
        let repo = PgNotificationRepository::new(pool);

        let notification = make_notification();
        let saved = repo.save(&notification).await.unwrap();
        assert_eq!(saved.retry_count, 0);

        let retried = repo.increment_retry(saved.id).await.unwrap();
        assert_eq!(retried.retry_count, 1);
        assert_eq!(retried.status, NotificationStatus::Retrying);

        let retried2 = repo.increment_retry(saved.id).await.unwrap();
        assert_eq!(retried2.retry_count, 2);
    }

    #[tokio::test]
    async fn test_increment_retry_not_found() {
        let (pool, _container) = setup_pool().await;
        let repo = PgNotificationRepository::new(pool);

        let result = repo.increment_retry(Uuid::new_v4()).await;
        assert!(matches!(result, Err(NotifyError::NotFound { .. })));
    }

    #[tokio::test]
    async fn test_find_pending() {
        let (pool, _container) = setup_pool().await;
        let repo = PgNotificationRepository::new(pool);

        // Save a pending notification
        let n1 = make_notification();
        repo.save(&n1).await.unwrap();

        // Save another and mark it sent
        let n2 = make_notification();
        let saved2 = repo.save(&n2).await.unwrap();
        repo.mark_sent(saved2.id).await.unwrap();

        // Save a retrying one
        let n3 = make_notification();
        let saved3 = repo.save(&n3).await.unwrap();
        repo.increment_retry(saved3.id).await.unwrap();

        let pending = repo.find_pending(10).await.unwrap();
        assert_eq!(pending.len(), 2); // n1 (pending) + n3 (retrying)
    }

    #[tokio::test]
    async fn test_find_pending_respects_priority_order() {
        let (pool, _container) = setup_pool().await;
        let repo = PgNotificationRepository::new(pool);

        // Save low-priority first
        let mut low = make_notification();
        low.priority = NotificationPriority::Low;
        repo.save(&low).await.unwrap();

        // Save urgent second
        let mut urgent = make_notification();
        urgent.priority = NotificationPriority::Urgent;
        repo.save(&urgent).await.unwrap();

        let pending = repo.find_pending(10).await.unwrap();
        assert_eq!(pending.len(), 2);
        assert_eq!(pending[0].priority, NotificationPriority::Urgent);
        assert_eq!(pending[1].priority, NotificationPriority::Low);
    }

    #[tokio::test]
    async fn test_get_statistics() {
        let (pool, _container) = setup_pool().await;
        let repo = PgNotificationRepository::new(pool);

        // Empty stats
        let stats = repo.get_statistics().await.unwrap();
        assert_eq!(stats.total, 0);

        // Add notifications with different statuses
        let n1 = make_notification();
        repo.save(&n1).await.unwrap();

        let n2 = make_notification();
        let saved2 = repo.save(&n2).await.unwrap();
        repo.mark_sent(saved2.id).await.unwrap();

        let n3 = make_notification();
        let saved3 = repo.save(&n3).await.unwrap();
        repo.mark_failed(saved3.id, "error").await.unwrap();

        let n4 = make_notification();
        let saved4 = repo.save(&n4).await.unwrap();
        repo.increment_retry(saved4.id).await.unwrap();

        let stats = repo.get_statistics().await.unwrap();
        assert_eq!(stats.total, 4);
        assert_eq!(stats.pending, 1);
        assert_eq!(stats.sent, 1);
        assert_eq!(stats.failed, 1);
        assert_eq!(stats.retrying, 1);
    }
}
