// Copyright 2025 Rararulab
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

//! SQLite-backed [`FeedStore`] implementation.
//!
//! Persists [`FeedEvent`]s to the `feed_events` table and tracks per-subscriber
//! read cursors in `feed_read_cursors`. Both tables are created by the
//! `20260414125453_feed_events` migration.

use async_trait::async_trait;
use jiff::Timestamp;
use rara_kernel::{
    data_feed::{FeedEvent, FeedEventId, FeedFilter, FeedStore},
    session::SessionKey,
};
use snafu::ResultExt;
use sqlx::SqlitePool;
use tracing::instrument;

/// SQLite-backed feed event store.
///
/// Implements [`FeedStore`] using the `feed_events` and `feed_read_cursors`
/// tables. All operations use the shared connection pool.
pub struct SqliteFeedStore {
    pool: SqlitePool,
}

impl SqliteFeedStore {
    /// Create a new store backed by the given connection pool.
    pub fn new(pool: SqlitePool) -> Self { Self { pool } }
}

#[async_trait]
impl FeedStore for SqliteFeedStore {
    #[instrument(skip_all, fields(event_id = %event.id, source = %event.source_name))]
    async fn append(&self, event: &FeedEvent) -> rara_kernel::Result<()> {
        let id = event.id.to_string();
        let tags_json =
            serde_json::to_string(&event.tags).expect("tags serialisation should not fail");
        let payload_json =
            serde_json::to_string(&event.payload).expect("payload serialisation should not fail");
        let received_at = event.received_at.to_string();

        // INSERT OR IGNORE for idempotency on event.id.
        sqlx::query(
            "INSERT OR IGNORE INTO feed_events (id, source_name, event_type, tags, payload, \
             received_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )
        .bind(&id)
        .bind(&event.source_name)
        .bind(&event.event_type)
        .bind(&tags_json)
        .bind(&payload_json)
        .bind(&received_at)
        .execute(&self.pool)
        .await
        .whatever_context("feed_events insert failed")?;

        Ok(())
    }

    #[instrument(skip_all)]
    async fn query(&self, filter: FeedFilter) -> rara_kernel::Result<Vec<FeedEvent>> {
        let mut sql = String::from(
            "SELECT id, source_name, event_type, tags, payload, received_at FROM feed_events \
             WHERE 1=1",
        );
        let mut binds: Vec<String> = Vec::new();

        if let Some(ref source) = filter.source_name {
            sql.push_str(" AND source_name = ?");
            binds.push(source.clone());
        }

        if let Some(ref since) = filter.since {
            sql.push_str(" AND received_at >= ?");
            binds.push(since.to_string());
        }

        sql.push_str(" ORDER BY received_at ASC LIMIT ?");

        let limit = filter.limit.min(1000) as i64;

        let mut query = sqlx::query_as::<_, FeedEventRow>(&sql);
        for bind in &binds {
            query = query.bind(bind);
        }
        query = query.bind(limit);

        let rows: Vec<FeedEventRow> = query
            .fetch_all(&self.pool)
            .await
            .whatever_context("feed_events query failed")?;

        let mut events: Vec<FeedEvent> = Vec::with_capacity(rows.len());
        for row in rows {
            let event = row.into_feed_event()?;
            // Apply tag filter in-memory (simpler than dynamic SQL for array
            // containment on a JSON text column).
            if !filter.tags.is_empty() && !filter.tags.iter().all(|t| event.tags.contains(t)) {
                continue;
            }
            events.push(event);
        }

        Ok(events)
    }

    #[instrument(skip_all, fields(subscriber = %subscriber))]
    async fn mark_read(
        &self,
        subscriber: &SessionKey,
        up_to: FeedEventId,
    ) -> rara_kernel::Result<()> {
        let sub_id = subscriber.to_string();
        let event_id = up_to.to_string();

        let source: Option<(String,)> =
            sqlx::query_as("SELECT source_name FROM feed_events WHERE id = ?1")
                .bind(&event_id)
                .fetch_optional(&self.pool)
                .await
                .whatever_context("feed_events lookup failed")?;

        let source_name = source.map(|s| s.0).unwrap_or_else(|| "unknown".to_owned());

        sqlx::query(
            "INSERT INTO feed_read_cursors (subscriber_id, source_name, last_read_id, updated_at) \
             VALUES (?1, ?2, ?3, ?4) ON CONFLICT(subscriber_id, source_name) DO UPDATE SET \
             last_read_id = excluded.last_read_id, updated_at = excluded.updated_at",
        )
        .bind(&sub_id)
        .bind(&source_name)
        .bind(&event_id)
        .bind(Timestamp::now().to_string())
        .execute(&self.pool)
        .await
        .whatever_context("feed_read_cursors upsert failed")?;

        Ok(())
    }

    #[instrument(skip_all, fields(subscriber = %subscriber))]
    async fn unread_count(&self, subscriber: &SessionKey) -> rara_kernel::Result<usize> {
        let sub_id = subscriber.to_string();

        let count: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM feed_events e WHERE NOT EXISTS (SELECT 1 FROM feed_read_cursors \
             c WHERE c.subscriber_id = ?1 AND c.source_name = e.source_name AND c.last_read_id >= \
             e.id)",
        )
        .bind(&sub_id)
        .fetch_one(&self.pool)
        .await
        .whatever_context("unread_count query failed")?;

        Ok(count.0 as usize)
    }
}

// ---------------------------------------------------------------------------
// Internal row type
// ---------------------------------------------------------------------------

#[derive(sqlx::FromRow)]
struct FeedEventRow {
    id:          String,
    source_name: String,
    event_type:  String,
    tags:        String,
    payload:     String,
    received_at: String,
}

impl FeedEventRow {
    fn into_feed_event(self) -> rara_kernel::Result<FeedEvent> {
        let id = FeedEventId::deterministic(&self.id);
        let tags: Vec<String> = serde_json::from_str(&self.tags).unwrap_or_default();
        let payload: serde_json::Value =
            serde_json::from_str(&self.payload).unwrap_or(serde_json::Value::Null);
        let received_at: Timestamp =
            self.received_at
                .parse()
                .map_err(|e: jiff::Error| rara_kernel::KernelError::Other {
                    message: format!("invalid received_at timestamp: {e}").into(),
                })?;

        Ok(FeedEvent {
            id,
            source_name: self.source_name,
            event_type: self.event_type,
            tags,
            payload,
            received_at,
        })
    }
}
