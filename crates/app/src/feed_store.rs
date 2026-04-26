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
//! Persists [`FeedEvent`]s to the `data_feed_events` table. The table is
//! created by the init migration baseline.

use async_trait::async_trait;
use diesel::{ExpressionMethods, QueryDsl, Queryable, Selectable, SelectableHelper};
use diesel_async::RunQueryDsl;
use jiff::Timestamp;
use rara_kernel::data_feed::{FeedEvent, FeedEventId, FeedFilter, FeedStore};
use rara_model::schema::data_feed_events;
use snafu::ResultExt;
use tracing::instrument;
use yunara_store::diesel_pool::DieselSqlitePools;

/// SQLite-backed feed event store.
///
/// Implements [`FeedStore`] using the `data_feed_events` table. Reads use
/// the reader pool; appends use the single-writer pool.
pub struct SqliteFeedStore {
    pools: DieselSqlitePools,
}

impl SqliteFeedStore {
    /// Create a new store backed by the given pool bundle.
    pub fn new(pools: DieselSqlitePools) -> Self { Self { pools } }
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

        let mut conn = self
            .pools
            .writer
            .get()
            .await
            .whatever_context("data_feed_events pool acquire failed")?;

        // INSERT OR IGNORE for idempotency on event.id.
        diesel::insert_into(data_feed_events::table)
            .values((
                data_feed_events::id.eq(&id),
                data_feed_events::source_name.eq(&event.source_name),
                data_feed_events::event_type.eq(&event.event_type),
                data_feed_events::tags.eq(&tags_json),
                data_feed_events::payload.eq(&payload_json),
                data_feed_events::received_at.eq(&received_at),
            ))
            .on_conflict_do_nothing()
            .execute(&mut *conn)
            .await
            .whatever_context("data_feed_events insert failed")?;

        Ok(())
    }

    #[instrument(skip_all)]
    async fn query(&self, filter: FeedFilter) -> rara_kernel::Result<Vec<FeedEvent>> {
        let mut conn = self
            .pools
            .reader
            .get()
            .await
            .whatever_context("data_feed_events pool acquire failed")?;

        let mut q = data_feed_events::table.into_boxed();

        if let Some(ref source) = filter.source_name {
            q = q.filter(data_feed_events::source_name.eq(source));
        }
        if let Some(ref since) = filter.since {
            q = q.filter(data_feed_events::received_at.ge(since.to_string()));
        }

        let limit: i64 = (filter.limit.min(1000)) as i64;

        let rows: Vec<FeedEventRow> = q
            .select(FeedEventRow::as_select())
            .order(data_feed_events::received_at.asc())
            .limit(limit)
            .load(&mut *conn)
            .await
            .whatever_context("data_feed_events query failed")?;

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
}

// ---------------------------------------------------------------------------
// Internal row type
// ---------------------------------------------------------------------------

#[derive(Queryable, Selectable)]
#[diesel(table_name = data_feed_events)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
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
