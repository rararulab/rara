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

//! PostgreSQL-backed [`AuditLog`] implementation.
//!
//! Persists audit events to the `kernel_audit_events` table so they survive
//! process restarts. The [`PgAuditLog::record`] method fires-and-forgets
//! via `tokio::spawn` to avoid blocking the hot path.

use async_trait::async_trait;
use rara_kernel::{
    audit::{AuditEvent, AuditEventType, AuditFilter, AuditLog, event_type_name},
    process::{AgentId, SessionId, principal::UserId},
};
use sqlx::PgPool;
use tracing::error;

// -- DB row type (chrono at DB boundary) --------------------------------------

#[derive(sqlx::FromRow)]
struct AuditRow {
    #[allow(dead_code)]
    id:         uuid::Uuid,
    timestamp:  chrono::DateTime<chrono::Utc>,
    agent_id:   uuid::Uuid,
    session_id: String,
    user_id:    String,
    event_type: String,
    event_data: serde_json::Value,
    details:    serde_json::Value,
    #[allow(dead_code)]
    created_at: chrono::DateTime<chrono::Utc>,
}

// -- Conversion helpers -------------------------------------------------------

fn chrono_to_jiff(dt: chrono::DateTime<chrono::Utc>) -> jiff::Timestamp {
    jiff::Timestamp::from_second(dt.timestamp()).unwrap_or(jiff::Timestamp::UNIX_EPOCH)
}

fn jiff_to_chrono(ts: jiff::Timestamp) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::from_timestamp(ts.as_second(), 0).unwrap_or_default()
}

fn row_to_event(row: AuditRow) -> Option<AuditEvent> {
    // Reconstruct AuditEventType from event_type name + event_data JSON.
    // We store the serde-tagged representation: {"VariantName": { ...fields }}
    let tagged = serde_json::json!({ &row.event_type: row.event_data });
    let event_type: AuditEventType = serde_json::from_value(tagged).ok()?;

    Some(AuditEvent {
        timestamp: chrono_to_jiff(row.timestamp),
        agent_id: AgentId(row.agent_id),
        session_id: SessionId::new(row.session_id),
        user_id: UserId(row.user_id),
        event_type,
        details: row.details,
    })
}

// -- PgAuditLog ---------------------------------------------------------------

/// PostgreSQL-backed audit log.
///
/// Writes are fire-and-forget (spawned as background tasks). Reads
/// support filtering by agent, user, event type, timestamp, and limit.
pub struct PgAuditLog {
    pool: PgPool,
}

impl PgAuditLog {
    /// Create a new `PgAuditLog` backed by the given connection pool.
    pub fn new(pool: PgPool) -> Self { Self { pool } }
}

#[async_trait]
impl AuditLog for PgAuditLog {
    async fn record(&self, event: AuditEvent) {
        let pool = self.pool.clone();
        tokio::spawn(async move {
            // Serialize the event_type variant into name + data.
            let type_name = event_type_name(&event.event_type).to_string();

            // Serialize the full AuditEventType to get the tagged JSON,
            // then extract just the inner fields for storage.
            let event_data = match serde_json::to_value(&event.event_type) {
                Ok(serde_json::Value::Object(mut map)) => {
                    // serde serializes externally tagged enums as {"VariantName": {fields}}.
                    map.remove(&type_name)
                        .unwrap_or(serde_json::Value::Object(serde_json::Map::new()))
                }
                Ok(v) => v,
                Err(e) => {
                    error!(error = %e, "audit: failed to serialize event_type");
                    return;
                }
            };

            let timestamp = jiff_to_chrono(event.timestamp);

            if let Err(e) = sqlx::query(
                "INSERT INTO kernel_audit_events (timestamp, agent_id, session_id, user_id, \
                 event_type, event_data, details) VALUES ($1, $2, $3, $4, $5, $6, $7)",
            )
            .bind(timestamp)
            .bind(event.agent_id.0)
            .bind(event.session_id.as_str())
            .bind(&event.user_id.0)
            .bind(&type_name)
            .bind(&event_data)
            .bind(&event.details)
            .execute(&pool)
            .await
            {
                error!(error = %e, "audit: failed to insert event");
            }
        });
    }

    async fn query(&self, filter: AuditFilter) -> Vec<AuditEvent> {
        // Build dynamic query with optional WHERE clauses.
        let mut sql = String::from(
            "SELECT id, timestamp, agent_id, session_id, user_id, event_type, event_data, \
             details, created_at FROM kernel_audit_events WHERE 1=1",
        );
        let mut param_idx = 0u32;
        // We'll track which params to bind later.
        let agent_id_val = filter.agent_id.map(|a| a.0);
        let user_id_val = filter.user_id.map(|u| u.0);
        let event_type_val = filter.event_type.clone();
        let since_val = filter.since.map(jiff_to_chrono);

        if agent_id_val.is_some() {
            param_idx += 1;
            sql.push_str(&format!(" AND agent_id = ${param_idx}"));
        }
        if user_id_val.is_some() {
            param_idx += 1;
            sql.push_str(&format!(" AND user_id = ${param_idx}"));
        }
        if event_type_val.is_some() {
            param_idx += 1;
            sql.push_str(&format!(" AND event_type = ${param_idx}"));
        }
        if since_val.is_some() {
            param_idx += 1;
            sql.push_str(&format!(" AND timestamp >= ${param_idx}"));
        }

        sql.push_str(" ORDER BY timestamp ASC");

        let limit = if filter.limit == 0 {
            i64::MAX
        } else {
            filter.limit as i64
        };
        param_idx += 1;
        sql.push_str(&format!(" LIMIT ${param_idx}"));

        let mut query = sqlx::query_as::<_, AuditRow>(&sql);

        if let Some(ref aid) = agent_id_val {
            query = query.bind(aid);
        }
        if let Some(ref uid) = user_id_val {
            query = query.bind(uid);
        }
        if let Some(ref et) = event_type_val {
            query = query.bind(et);
        }
        if let Some(ref since) = since_val {
            query = query.bind(since);
        }
        query = query.bind(limit);

        match query.fetch_all(&self.pool).await {
            Ok(rows) => rows.into_iter().filter_map(row_to_event).collect(),
            Err(e) => {
                error!(error = %e, "audit: failed to query events");
                vec![]
            }
        }
    }
}
