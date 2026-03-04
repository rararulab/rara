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

//! SQLite-backed [`OutboxStore`] implementation.
//!
//! Stores [`OutboundEnvelope`]s durably so messages that cannot be delivered
//! immediately (e.g., user offline) survive restarts and can be retried by a
//! background drainer.
//!
//! The full [`OutboundEnvelope`] is serialized into the `target` JSON column
//! so that `drain_pending` can reconstruct the exact envelope without losing
//! any fields. The `payload` column stores just the payload portion for
//! potential ad-hoc querying. The `channel_type` TEXT column carries a
//! human-readable routing label for simple filtering.

use async_trait::async_trait;
use rara_kernel::io::{
    bus::OutboxStore,
    types::{BusError, MessageId, OutboundEnvelope, OutboundRouting},
};
use sqlx::SqlitePool;
use tracing::warn;

// -- DB row type (chrono at DB boundary) --------------------------------------

#[derive(sqlx::FromRow)]
struct OutboxRow {
    id:           String,
    #[allow(dead_code)]
    channel_type: String,
    target:       serde_json::Value,
    #[allow(dead_code)]
    payload:      serde_json::Value,
    #[allow(dead_code)]
    status:       i16,
    created_at:   chrono::DateTime<chrono::Utc>,
    #[allow(dead_code)]
    delivered_at: Option<chrono::DateTime<chrono::Utc>>,
}

// -- Conversion helpers -------------------------------------------------------

/// Extract a routing-strategy label for the `channel_type` TEXT column.
///
/// Allows simple queries like `WHERE channel_type = 'broadcast'` without
/// parsing JSONB.
fn routing_label(routing: &OutboundRouting) -> &'static str {
    match routing {
        OutboundRouting::BroadcastAll => "broadcast",
        OutboundRouting::BroadcastExcept { .. } => "broadcast",
        OutboundRouting::Targeted { .. } => "targeted",
    }
}

fn chrono_to_jiff(dt: chrono::DateTime<chrono::Utc>) -> jiff::Timestamp {
    jiff::Timestamp::from_second(dt.timestamp()).unwrap_or(jiff::Timestamp::UNIX_EPOCH)
}

fn jiff_to_chrono(ts: jiff::Timestamp) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::from_timestamp(ts.as_second(), 0).unwrap_or_default()
}

// -- PgOutboxStore ------------------------------------------------------------

/// SQLite-backed durable outbox for [`OutboundEnvelope`]s.
pub struct PgOutboxStore {
    pool: SqlitePool,
}

impl PgOutboxStore {
    pub fn new(pool: SqlitePool) -> Self { Self { pool } }
}

#[async_trait]
impl OutboxStore for PgOutboxStore {
    async fn append(&self, envelope: OutboundEnvelope) -> Result<(), BusError> {
        let id = envelope.id.0.clone();
        let channel_type = routing_label(&envelope.routing);

        // Store the *full* envelope in `target` so we can reconstruct it
        // exactly during `drain_pending`. The `payload` column stores just
        // the payload portion for potential ad-hoc queries.
        let target = serde_json::to_value(&envelope).map_err(|e| BusError::Internal {
            message: format!("outbox serialize envelope: {e}"),
        })?;
        let payload = serde_json::to_value(&envelope.payload).map_err(|e| BusError::Internal {
            message: format!("outbox serialize payload: {e}"),
        })?;
        let created_at = jiff_to_chrono(envelope.timestamp);

        sqlx::query(
            "INSERT INTO kernel_outbox (id, channel_type, target, payload, status, created_at) \
             VALUES (?1, ?2, ?3, ?4, 0, ?5)",
        )
        .bind(&id)
        .bind(channel_type)
        .bind(&target)
        .bind(&payload)
        .bind(created_at)
        .execute(&self.pool)
        .await
        .map_err(|e| BusError::Internal {
            message: format!("outbox append: {e}"),
        })?;

        Ok(())
    }

    async fn drain_pending(&self, max: usize) -> Vec<OutboundEnvelope> {
        let rows = sqlx::query_as::<_, OutboxRow>(
            "SELECT id, channel_type, target, payload, status, created_at, delivered_at FROM \
             kernel_outbox WHERE status = 0 ORDER BY created_at ASC LIMIT ?1",
        )
        .bind(max as i64)
        .fetch_all(&self.pool)
        .await;

        match rows {
            Ok(rows) => rows
                .into_iter()
                .filter_map(|row| {
                    // The `target` column holds the full serialized
                    // OutboundEnvelope — deserialize directly.
                    match serde_json::from_value::<OutboundEnvelope>(row.target) {
                        Ok(mut env) => {
                            // Prefer the DB timestamp to avoid jiff
                            // round-trip drift.
                            env.timestamp = chrono_to_jiff(row.created_at);
                            Some(env)
                        }
                        Err(e) => {
                            warn!(
                                id = %row.id,
                                error = %e,
                                "outbox: failed to deserialize envelope"
                            );
                            None
                        }
                    }
                })
                .collect(),
            Err(e) => {
                warn!(error = %e, "outbox: failed to drain pending");
                vec![]
            }
        }
    }

    async fn mark_delivered(&self, id: &MessageId) -> Result<(), BusError> {
        sqlx::query("UPDATE kernel_outbox SET status = 1, delivered_at = datetime('now') WHERE id = ?1")
            .bind(&id.0)
            .execute(&self.pool)
            .await
            .map_err(|e| BusError::Internal {
                message: format!("outbox mark_delivered: {e}"),
            })?;

        Ok(())
    }
}
