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

use diesel::{ExpressionMethods, QueryDsl, Queryable, Selectable, SelectableHelper};
use diesel_async::RunQueryDsl;
use jiff::Timestamp;
use rara_kernel::data_feed::{
    AuthConfig, DataFeedConfig, FeedEvent, FeedEventId, FeedStatus, FeedType,
};
use rara_model::schema::{data_feed_events, data_feeds};
use snafu::ResultExt;
use tracing::instrument;
use yunara_store::diesel_pool::DieselSqlitePool;

use super::error::{DataFeedSvcError, EncodeJsonSnafu, PoolAcquireSnafu, QuerySnafu, Result};

/// Service for data feed persistence operations.
///
/// Holds a diesel-async SQLite pool and provides CRUD on the `data_feeds`
/// table plus paginated queries on the `data_feed_events` table.
#[derive(Clone)]
pub struct DataFeedSvc {
    pool: DieselSqlitePool,
}

impl DataFeedSvc {
    /// Create a new service backed by the given pool.
    pub fn new(pool: DieselSqlitePool) -> Self { Self { pool } }

    // -- Feed config CRUD ---------------------------------------------------

    /// List all registered data feed configurations.
    #[instrument(skip_all)]
    pub async fn list_feeds(&self) -> Result<Vec<DataFeedConfig>> {
        let mut conn = self.pool.get().await.context(PoolAcquireSnafu)?;
        let rows: Vec<FeedRow> = data_feeds::table
            .select(FeedRow::as_select())
            .order(data_feeds::created_at.desc())
            .load(&mut *conn)
            .await
            .context(QuerySnafu)?;

        rows.into_iter().map(FeedRow::into_config).collect()
    }

    /// Get a single feed by ID.
    #[instrument(skip(self))]
    pub async fn get_feed(&self, id: &str) -> Result<Option<DataFeedConfig>> {
        use diesel::OptionalExtension;

        let mut conn = self.pool.get().await.context(PoolAcquireSnafu)?;
        let row: Option<FeedRow> = data_feeds::table
            .filter(data_feeds::id.eq(id))
            .select(FeedRow::as_select())
            .first(&mut *conn)
            .await
            .optional()
            .context(QuerySnafu)?;

        row.map(FeedRow::into_config).transpose()
    }

    /// Insert a new feed configuration.
    #[instrument(skip_all)]
    pub async fn create_feed(&self, config: &DataFeedConfig) -> Result<()> {
        let tags_json = serde_json::to_string(&config.tags).context(EncodeJsonSnafu)?;
        let transport_json = serde_json::to_string(&config.transport).context(EncodeJsonSnafu)?;
        let auth_json = config
            .auth
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .context(EncodeJsonSnafu)?;

        let mut conn = self.pool.get().await.context(PoolAcquireSnafu)?;
        diesel::insert_into(data_feeds::table)
            .values((
                data_feeds::id.eq(&config.id),
                data_feeds::name.eq(&config.name),
                data_feeds::feed_type.eq(config.feed_type.to_string()),
                data_feeds::tags.eq(&tags_json),
                data_feeds::transport.eq(&transport_json),
                data_feeds::auth.eq(&auth_json),
                data_feeds::enabled.eq(i32::from(config.enabled)),
                data_feeds::status.eq(config.status.to_string()),
                data_feeds::last_error.eq(&config.last_error),
                data_feeds::created_at.eq(config.created_at.to_string()),
                data_feeds::updated_at.eq(config.updated_at.to_string()),
            ))
            .execute(&mut *conn)
            .await
            .context(QuerySnafu)?;

        Ok(())
    }

    /// Update an existing feed configuration.
    ///
    /// Returns `true` if a row was updated.
    #[instrument(skip_all)]
    pub async fn update_feed(&self, config: &DataFeedConfig) -> Result<bool> {
        let tags_json = serde_json::to_string(&config.tags).context(EncodeJsonSnafu)?;
        let transport_json = serde_json::to_string(&config.transport).context(EncodeJsonSnafu)?;
        let auth_json = config
            .auth
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .context(EncodeJsonSnafu)?;
        let now = Timestamp::now().to_string();

        let mut conn = self.pool.get().await.context(PoolAcquireSnafu)?;
        let affected = diesel::update(data_feeds::table.filter(data_feeds::id.eq(&config.id)))
            .set((
                data_feeds::name.eq(&config.name),
                data_feeds::feed_type.eq(config.feed_type.to_string()),
                data_feeds::tags.eq(&tags_json),
                data_feeds::transport.eq(&transport_json),
                data_feeds::auth.eq(&auth_json),
                data_feeds::enabled.eq(i32::from(config.enabled)),
                data_feeds::status.eq(config.status.to_string()),
                data_feeds::last_error.eq(&config.last_error),
                data_feeds::updated_at.eq(&now),
            ))
            .execute(&mut *conn)
            .await
            .context(QuerySnafu)?;

        Ok(affected > 0)
    }

    /// Delete a feed by ID. Returns `true` if a row was deleted.
    #[instrument(skip(self))]
    pub async fn delete_feed(&self, id: &str) -> Result<bool> {
        let mut conn = self.pool.get().await.context(PoolAcquireSnafu)?;
        let affected = diesel::delete(data_feeds::table.filter(data_feeds::id.eq(id)))
            .execute(&mut *conn)
            .await
            .context(QuerySnafu)?;
        Ok(affected > 0)
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
    ) -> Result<bool> {
        let now = Timestamp::now().to_string();
        let mut conn = self.pool.get().await.context(PoolAcquireSnafu)?;
        let affected = diesel::update(data_feeds::table.filter(data_feeds::name.eq(name)))
            .set((
                data_feeds::status.eq(status.to_string()),
                data_feeds::last_error.eq(&last_error),
                data_feeds::updated_at.eq(&now),
            ))
            .execute(&mut *conn)
            .await
            .context(QuerySnafu)?;
        Ok(affected > 0)
    }

    /// Toggle the enabled flag for a feed. Returns `true` if updated.
    #[instrument(skip(self))]
    pub async fn toggle_feed(&self, id: &str) -> Result<bool> {
        use diesel::{dsl::sql, sql_types::Integer};

        let now = Timestamp::now().to_string();
        let mut conn = self.pool.get().await.context(PoolAcquireSnafu)?;
        // `NOT enabled` on a stored integer column has no clean DSL — we use
        // the sanctioned `sql::<Integer>` fragment per
        // docs/guides/db-diesel-migration.md.
        let affected = diesel::update(data_feeds::table.filter(data_feeds::id.eq(id)))
            .set((
                data_feeds::enabled.eq(sql::<Integer>("NOT enabled")),
                data_feeds::updated_at.eq(&now),
            ))
            .execute(&mut *conn)
            .await
            .context(QuerySnafu)?;
        Ok(affected > 0)
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
    ) -> Result<EventPage> {
        let mut conn = self.pool.get().await.context(PoolAcquireSnafu)?;

        // Count total matching events for pagination metadata.
        let mut count_q = data_feed_events::table
            .filter(data_feed_events::source_name.eq(source_name))
            .into_boxed();
        if let Some(ref ts) = since {
            count_q = count_q.filter(data_feed_events::received_at.ge(ts.to_string()));
        }
        let total: i64 = count_q
            .count()
            .get_result(&mut *conn)
            .await
            .context(QuerySnafu)?;

        let mut rows_q = data_feed_events::table
            .filter(data_feed_events::source_name.eq(source_name))
            .into_boxed();
        if let Some(ref ts) = since {
            rows_q = rows_q.filter(data_feed_events::received_at.ge(ts.to_string()));
        }
        let rows: Vec<EventRow> = rows_q
            .select(EventRow::as_select())
            .order(data_feed_events::received_at.desc())
            .limit(limit)
            .offset(offset)
            .load(&mut *conn)
            .await
            .context(QuerySnafu)?;

        let events: Vec<FeedEvent> = rows
            .into_iter()
            .map(EventRow::into_event)
            .collect::<Result<Vec<_>>>()?;

        let has_more = (offset + limit) < total;

        Ok(EventPage {
            events,
            total,
            has_more,
        })
    }

    /// Get a single event by ID within a feed.
    #[instrument(skip(self))]
    pub async fn get_event(&self, source_name: &str, event_id: &str) -> Result<Option<FeedEvent>> {
        use diesel::OptionalExtension;

        let mut conn = self.pool.get().await.context(PoolAcquireSnafu)?;
        let row: Option<EventRow> = data_feed_events::table
            .filter(data_feed_events::id.eq(event_id))
            .filter(data_feed_events::source_name.eq(source_name))
            .select(EventRow::as_select())
            .first(&mut *conn)
            .await
            .optional()
            .context(QuerySnafu)?;

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

#[derive(Queryable, Selectable)]
#[diesel(table_name = data_feeds)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
struct FeedRow {
    id:         String,
    name:       String,
    feed_type:  String,
    tags:       String,
    transport:  String,
    auth:       Option<String>,
    enabled:    i32,
    status:     String,
    last_error: Option<String>,
    created_at: String,
    updated_at: String,
}

impl FeedRow {
    fn into_config(self) -> Result<DataFeedConfig> {
        let decode = |msg: String| DataFeedSvcError::DecodeRow { message: msg };
        let feed_type: FeedType = serde_json::from_value(serde_json::Value::String(self.feed_type))
            .map_err(|e| decode(format!("feed_type: {e}")))?;
        let status: FeedStatus = serde_json::from_value(serde_json::Value::String(self.status))
            .map_err(|e| decode(format!("status: {e}")))?;
        let tags: Vec<String> =
            serde_json::from_str(&self.tags).map_err(|e| decode(format!("tags: {e}")))?;
        let transport: serde_json::Value =
            serde_json::from_str(&self.transport).map_err(|e| decode(format!("transport: {e}")))?;
        let auth: Option<AuthConfig> = self
            .auth
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .map_err(|e| decode(format!("auth: {e}")))?;
        let created_at: Timestamp = self
            .created_at
            .parse()
            .map_err(|e| decode(format!("created_at: {e}")))?;
        let updated_at: Timestamp = self
            .updated_at
            .parse()
            .map_err(|e| decode(format!("updated_at: {e}")))?;

        Ok(DataFeedConfig::builder()
            .id(self.id)
            .name(self.name)
            .feed_type(feed_type)
            .tags(tags)
            .transport(transport)
            .maybe_auth(auth)
            .enabled(self.enabled != 0)
            .status(status)
            .maybe_last_error(self.last_error)
            .created_at(created_at)
            .updated_at(updated_at)
            .build())
    }
}

#[derive(Queryable, Selectable)]
#[diesel(table_name = data_feed_events)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
struct EventRow {
    id:          String,
    source_name: String,
    event_type:  String,
    tags:        String,
    payload:     String,
    received_at: String,
}

impl EventRow {
    fn into_event(self) -> Result<FeedEvent> {
        let decode = |msg: String| DataFeedSvcError::DecodeRow { message: msg };
        let id = FeedEventId::try_from_raw(&self.id)
            .map_err(|e| decode(format!("invalid event id: {e}")))?;
        let tags: Vec<String> =
            serde_json::from_str(&self.tags).map_err(|e| decode(format!("tags: {e}")))?;
        let payload: serde_json::Value =
            serde_json::from_str(&self.payload).map_err(|e| decode(format!("payload: {e}")))?;
        let received_at: Timestamp = self
            .received_at
            .parse()
            .map_err(|e| decode(format!("received_at: {e}")))?;

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
