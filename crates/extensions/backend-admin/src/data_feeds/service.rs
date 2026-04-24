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

//! Persistence layer for data feed CRUD and event queries.
//!
//! [`DataFeedSvc`] operates directly on the `data_feeds` and `data_feed_events`
//! SQLite tables. It does not depend on [`DataFeedRegistry`] — the registry
//! manages in-memory runtime state while this service manages persistence.
//!
//! [`DataFeedRegistry`]: rara_kernel::data_feed::DataFeedRegistry

use jiff::Timestamp;
use rara_kernel::data_feed::{
    AuthConfig, DataFeedConfig, FeedEvent, FeedEventId, FeedStatus, FeedType,
};
use sqlx::SqlitePool;
use tracing::instrument;

/// Service for data feed persistence operations.
///
/// Holds a SQLite connection pool and provides CRUD on the `data_feeds`
/// table plus paginated queries on the `data_feed_events` table.
#[derive(Clone)]
pub struct DataFeedSvc {
    pool: SqlitePool,
}

impl DataFeedSvc {
    /// Create a new service backed by the given pool.
    pub fn new(pool: SqlitePool) -> Self { Self { pool } }

    // -- Feed config CRUD ---------------------------------------------------

    /// List all registered data feed configurations.
    #[instrument(skip_all)]
    pub async fn list_feeds(&self) -> anyhow::Result<Vec<DataFeedConfig>> {
        let rows: Vec<FeedRow> = sqlx::query_as(
            "SELECT id, name, feed_type, tags, transport, auth, enabled, status, last_error, \
             created_at, updated_at FROM data_feeds ORDER BY created_at DESC",
        )
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(FeedRow::into_config).collect()
    }

    /// Get a single feed by ID.
    #[instrument(skip(self))]
    pub async fn get_feed(&self, id: &str) -> anyhow::Result<Option<DataFeedConfig>> {
        let row: Option<FeedRow> = sqlx::query_as(
            "SELECT id, name, feed_type, tags, transport, auth, enabled, status, last_error, \
             created_at, updated_at FROM data_feeds WHERE id = ?1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        row.map(FeedRow::into_config).transpose()
    }

    /// Insert a new feed configuration.
    #[instrument(skip_all)]
    pub async fn create_feed(&self, config: &DataFeedConfig) -> anyhow::Result<()> {
        let tags_json = serde_json::to_string(&config.tags)?;
        let transport_json = serde_json::to_string(&config.transport)?;
        let auth_json = config
            .auth
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;

        sqlx::query(
            "INSERT INTO data_feeds (id, name, feed_type, tags, transport, auth, enabled, status, \
             last_error, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, \
             ?11)",
        )
        .bind(&config.id)
        .bind(&config.name)
        .bind(config.feed_type.to_string())
        .bind(&tags_json)
        .bind(&transport_json)
        .bind(&auth_json)
        .bind(config.enabled)
        .bind(config.status.to_string())
        .bind(&config.last_error)
        .bind(config.created_at.to_string())
        .bind(config.updated_at.to_string())
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Update an existing feed configuration.
    ///
    /// Returns `true` if a row was updated.
    #[instrument(skip_all)]
    pub async fn update_feed(&self, config: &DataFeedConfig) -> anyhow::Result<bool> {
        let tags_json = serde_json::to_string(&config.tags)?;
        let transport_json = serde_json::to_string(&config.transport)?;
        let auth_json = config
            .auth
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        let now = Timestamp::now().to_string();

        let result = sqlx::query(
            "UPDATE data_feeds SET name = ?1, feed_type = ?2, tags = ?3, transport = ?4, auth = \
             ?5, enabled = ?6, status = ?7, last_error = ?8, updated_at = ?9 WHERE id = ?10",
        )
        .bind(&config.name)
        .bind(config.feed_type.to_string())
        .bind(&tags_json)
        .bind(&transport_json)
        .bind(&auth_json)
        .bind(config.enabled)
        .bind(config.status.to_string())
        .bind(&config.last_error)
        .bind(&now)
        .bind(&config.id)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Delete a feed by ID. Returns `true` if a row was deleted.
    #[instrument(skip(self))]
    pub async fn delete_feed(&self, id: &str) -> anyhow::Result<bool> {
        let result = sqlx::query("DELETE FROM data_feeds WHERE id = ?1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Update the runtime `status` and `last_error` columns for a feed,
    /// looked up by unique name.
    ///
    /// Called via the [`StatusReporter`] wiring so the `data_feeds` table
    /// reflects the actual runtime state (running / idle / error) rather
    /// than the stale value written at creation time.
    ///
    /// [`StatusReporter`]: rara_kernel::data_feed::StatusReporter
    #[instrument(skip(self))]
    pub async fn update_status(
        &self,
        name: &str,
        status: FeedStatus,
        last_error: Option<String>,
    ) -> anyhow::Result<bool> {
        let now = Timestamp::now().to_string();
        let result = sqlx::query(
            "UPDATE data_feeds SET status = ?1, last_error = ?2, updated_at = ?3 WHERE name = ?4",
        )
        .bind(status.to_string())
        .bind(&last_error)
        .bind(&now)
        .bind(name)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Toggle the enabled flag for a feed. Returns `true` if updated.
    #[instrument(skip(self))]
    pub async fn toggle_feed(&self, id: &str) -> anyhow::Result<bool> {
        let now = Timestamp::now().to_string();
        let result = sqlx::query(
            "UPDATE data_feeds SET enabled = NOT enabled, updated_at = ?1 WHERE id = ?2",
        )
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    // -- Event queries ------------------------------------------------------

    /// Query events for a specific feed, with pagination.
    #[instrument(skip(self))]
    pub async fn query_events(
        &self,
        source_name: &str,
        since: Option<Timestamp>,
        limit: i64,
        offset: i64,
    ) -> anyhow::Result<EventPage> {
        // Count total matching events for pagination metadata.
        let total: (i64,) = if let Some(ref ts) = since {
            sqlx::query_as(
                "SELECT COUNT(*) FROM data_feed_events WHERE source_name = ?1 AND received_at >= \
                 ?2",
            )
            .bind(source_name)
            .bind(ts.to_string())
            .fetch_one(&self.pool)
            .await?
        } else {
            sqlx::query_as("SELECT COUNT(*) FROM data_feed_events WHERE source_name = ?1")
                .bind(source_name)
                .fetch_one(&self.pool)
                .await?
        };

        let rows: Vec<EventRow> = if let Some(ref ts) = since {
            sqlx::query_as(
                "SELECT id, source_name, event_type, tags, payload, received_at FROM \
                 data_feed_events WHERE source_name = ?1 AND received_at >= ?2 ORDER BY \
                 received_at DESC LIMIT ?3 OFFSET ?4",
            )
            .bind(source_name)
            .bind(ts.to_string())
            .bind(limit)
            .bind(offset)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as(
                "SELECT id, source_name, event_type, tags, payload, received_at FROM \
                 data_feed_events WHERE source_name = ?1 ORDER BY received_at DESC LIMIT ?2 \
                 OFFSET ?3",
            )
            .bind(source_name)
            .bind(limit)
            .bind(offset)
            .fetch_all(&self.pool)
            .await?
        };

        let events: Vec<FeedEvent> = rows
            .into_iter()
            .map(EventRow::into_event)
            .collect::<anyhow::Result<Vec<_>>>()?;

        let has_more = (offset + limit) < total.0;

        Ok(EventPage {
            events,
            total: total.0,
            has_more,
        })
    }

    /// Get a single event by ID within a feed.
    #[instrument(skip(self))]
    pub async fn get_event(
        &self,
        source_name: &str,
        event_id: &str,
    ) -> anyhow::Result<Option<FeedEvent>> {
        let row: Option<EventRow> = sqlx::query_as(
            "SELECT id, source_name, event_type, tags, payload, received_at FROM data_feed_events \
             WHERE id = ?1 AND source_name = ?2",
        )
        .bind(event_id)
        .bind(source_name)
        .fetch_optional(&self.pool)
        .await?;

        row.map(EventRow::into_event).transpose()
    }
}

/// Paginated event query result.
#[derive(Debug, serde::Serialize)]
pub struct EventPage {
    /// Events on this page.
    pub events:   Vec<FeedEvent>,
    /// Total number of matching events.
    pub total:    i64,
    /// Whether more events exist beyond this page.
    pub has_more: bool,
}

// ---------------------------------------------------------------------------
// Internal row types for SQLite <-> domain mapping
// ---------------------------------------------------------------------------

#[derive(sqlx::FromRow)]
struct FeedRow {
    id:         String,
    name:       String,
    feed_type:  String,
    tags:       String,
    transport:  String,
    auth:       Option<String>,
    enabled:    bool,
    status:     String,
    last_error: Option<String>,
    created_at: String,
    updated_at: String,
}

impl FeedRow {
    fn into_config(self) -> anyhow::Result<DataFeedConfig> {
        let feed_type: FeedType =
            serde_json::from_value(serde_json::Value::String(self.feed_type))?;
        let status: FeedStatus = serde_json::from_value(serde_json::Value::String(self.status))?;
        let tags: Vec<String> = serde_json::from_str(&self.tags)?;
        let transport: serde_json::Value = serde_json::from_str(&self.transport)?;
        let auth: Option<AuthConfig> =
            self.auth.as_deref().map(serde_json::from_str).transpose()?;
        let created_at: Timestamp = self.created_at.parse()?;
        let updated_at: Timestamp = self.updated_at.parse()?;

        Ok(DataFeedConfig::builder()
            .id(self.id)
            .name(self.name)
            .feed_type(feed_type)
            .tags(tags)
            .transport(transport)
            .maybe_auth(auth)
            .enabled(self.enabled)
            .status(status)
            .maybe_last_error(self.last_error)
            .created_at(created_at)
            .updated_at(updated_at)
            .build())
    }
}

#[derive(sqlx::FromRow)]
struct EventRow {
    id:          String,
    source_name: String,
    event_type:  String,
    tags:        String,
    payload:     String,
    received_at: String,
}

impl EventRow {
    fn into_event(self) -> anyhow::Result<FeedEvent> {
        let id = FeedEventId::try_from_raw(&self.id)
            .map_err(|e| anyhow::anyhow!("invalid event id: {e}"))?;
        let tags: Vec<String> = serde_json::from_str(&self.tags)?;
        let payload: serde_json::Value = serde_json::from_str(&self.payload)?;
        let received_at: Timestamp = self.received_at.parse()?;

        Ok(FeedEvent::builder()
            .id(id)
            .source_name(self.source_name)
            .event_type(self.event_type)
            .tags(tags)
            .payload(payload)
            .received_at(received_at)
            .build())
    }
}
