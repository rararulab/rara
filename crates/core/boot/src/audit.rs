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
        timestamp:  chrono_to_jiff(row.timestamp),
        agent_id:   AgentId(row.agent_id),
        session_id: SessionId::new(row.session_id),
        user_id:    UserId(row.user_id),
        event_type,
        details:    row.details,
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
                "INSERT INTO kernel_audit_events \
                 (timestamp, agent_id, session_id, user_id, event_type, event_data, details) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7)",
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
            "SELECT id, timestamp, agent_id, session_id, user_id, \
             event_type, event_data, details, created_at \
             FROM kernel_audit_events WHERE 1=1",
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use rara_kernel::audit::{AuditEventType, MemoryOp};
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::postgres::Postgres;

    async fn setup_db() -> (PgPool, testcontainers::ContainerAsync<Postgres>) {
        let container = Postgres::default().start().await.unwrap();
        let url = format!(
            "postgres://postgres:postgres@127.0.0.1:{}/postgres",
            container.get_host_port_ipv4(5432).await.unwrap()
        );
        let pool = PgPool::connect(&url).await.unwrap();
        sqlx::migrate!("../../rara-model/migrations")
            .run(&pool)
            .await
            .unwrap();
        (pool, container)
    }

    fn make_event(agent_id: AgentId, user: &str, event_type: AuditEventType) -> AuditEvent {
        AuditEvent {
            timestamp:  jiff::Timestamp::now(),
            agent_id,
            session_id: SessionId::new("test-session"),
            user_id:    UserId(user.to_string()),
            event_type,
            details:    serde_json::Value::Null,
        }
    }

    /// Helper: record and wait for the background task to complete.
    async fn record_and_wait(log: &PgAuditLog, event: AuditEvent) {
        log.record(event).await;
        // Give the spawned task time to flush.
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }

    #[tokio::test]
    async fn test_record_and_query_all() {
        let (pool, _container) = setup_db().await;
        let log = PgAuditLog::new(pool);
        let aid = AgentId::new();

        record_and_wait(
            &log,
            make_event(
                aid,
                "alice",
                AuditEventType::ProcessSpawned {
                    manifest_name: "scout".to_string(),
                    parent_id:     None,
                },
            ),
        )
        .await;

        record_and_wait(
            &log,
            make_event(
                aid,
                "alice",
                AuditEventType::ProcessCompleted {
                    result: "done".to_string(),
                },
            ),
        )
        .await;

        let all = log
            .query(AuditFilter {
                limit: 100,
                ..Default::default()
            })
            .await;
        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn test_filter_by_agent_id() {
        let (pool, _container) = setup_db().await;
        let log = PgAuditLog::new(pool);
        let a1 = AgentId::new();
        let a2 = AgentId::new();

        record_and_wait(
            &log,
            make_event(
                a1,
                "alice",
                AuditEventType::ProcessSpawned {
                    manifest_name: "scout".to_string(),
                    parent_id:     None,
                },
            ),
        )
        .await;

        record_and_wait(
            &log,
            make_event(
                a2,
                "bob",
                AuditEventType::ProcessSpawned {
                    manifest_name: "worker".to_string(),
                    parent_id:     None,
                },
            ),
        )
        .await;

        let filtered = log
            .query(AuditFilter {
                agent_id: Some(a1),
                limit:    100,
                ..Default::default()
            })
            .await;
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].agent_id, a1);
    }

    #[tokio::test]
    async fn test_filter_by_user_id() {
        let (pool, _container) = setup_db().await;
        let log = PgAuditLog::new(pool);

        record_and_wait(
            &log,
            make_event(
                AgentId::new(),
                "alice",
                AuditEventType::ToolCall {
                    tool_name: "bash".to_string(),
                    approved:  true,
                },
            ),
        )
        .await;

        record_and_wait(
            &log,
            make_event(
                AgentId::new(),
                "bob",
                AuditEventType::ToolCall {
                    tool_name: "grep".to_string(),
                    approved:  true,
                },
            ),
        )
        .await;

        let filtered = log
            .query(AuditFilter {
                user_id: Some(UserId("alice".to_string())),
                limit:   100,
                ..Default::default()
            })
            .await;
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].user_id.0, "alice");
    }

    #[tokio::test]
    async fn test_filter_by_event_type() {
        let (pool, _container) = setup_db().await;
        let log = PgAuditLog::new(pool);
        let aid = AgentId::new();

        record_and_wait(
            &log,
            make_event(
                aid,
                "alice",
                AuditEventType::ProcessSpawned {
                    manifest_name: "scout".to_string(),
                    parent_id:     None,
                },
            ),
        )
        .await;

        record_and_wait(
            &log,
            make_event(
                aid,
                "alice",
                AuditEventType::ToolCall {
                    tool_name: "bash".to_string(),
                    approved:  true,
                },
            ),
        )
        .await;

        record_and_wait(
            &log,
            make_event(
                aid,
                "alice",
                AuditEventType::ProcessCompleted {
                    result: "done".to_string(),
                },
            ),
        )
        .await;

        let filtered = log
            .query(AuditFilter {
                event_type: Some("ToolCall".to_string()),
                limit:      100,
                ..Default::default()
            })
            .await;
        assert_eq!(filtered.len(), 1);
    }

    #[tokio::test]
    async fn test_query_with_limit() {
        let (pool, _container) = setup_db().await;
        let log = PgAuditLog::new(pool);
        let aid = AgentId::new();

        for _ in 0..10 {
            record_and_wait(
                &log,
                make_event(
                    aid,
                    "alice",
                    AuditEventType::ProcessSpawned {
                        manifest_name: "test".to_string(),
                        parent_id:     None,
                    },
                ),
            )
            .await;
        }

        let limited = log
            .query(AuditFilter {
                limit: 3,
                ..Default::default()
            })
            .await;
        assert_eq!(limited.len(), 3);
    }

    #[tokio::test]
    async fn test_roundtrip_all_event_types() {
        let (pool, _container) = setup_db().await;
        let log = PgAuditLog::new(pool);
        let aid = AgentId::new();
        let aid2 = AgentId::new();

        let events = vec![
            AuditEventType::ProcessSpawned {
                manifest_name: "scout".to_string(),
                parent_id:     Some(aid2),
            },
            AuditEventType::ProcessCompleted {
                result: "ok".to_string(),
            },
            AuditEventType::ProcessFailed {
                error: "boom".to_string(),
            },
            AuditEventType::ProcessKilled { by: aid2 },
            AuditEventType::ToolCall {
                tool_name: "bash".to_string(),
                approved:  false,
            },
            AuditEventType::LlmCall {
                model:       "gpt-4".to_string(),
                tokens_in:   100,
                tokens_out:  50,
                duration_ms: 1234,
            },
            AuditEventType::MemoryAccess {
                operation: MemoryOp::Store,
                key:       "foo".to_string(),
            },
            AuditEventType::SignalSent {
                target_id: aid2,
                signal:    "SIGTERM".to_string(),
            },
        ];

        for et in &events {
            record_and_wait(&log, make_event(aid, "alice", et.clone())).await;
        }

        let all = log
            .query(AuditFilter {
                limit: 100,
                ..Default::default()
            })
            .await;
        assert_eq!(all.len(), events.len());

        // Verify each event type roundtrips correctly.
        for (original, loaded) in events.iter().zip(all.iter()) {
            let orig_name = event_type_name(original);
            let loaded_name = event_type_name(&loaded.event_type);
            assert_eq!(orig_name, loaded_name);
        }
    }

    #[tokio::test]
    async fn test_combined_filters() {
        let (pool, _container) = setup_db().await;
        let log = PgAuditLog::new(pool);
        let target_agent = AgentId::new();
        let other_agent = AgentId::new();

        // target agent, alice, ToolCall
        record_and_wait(
            &log,
            make_event(
                target_agent,
                "alice",
                AuditEventType::ToolCall {
                    tool_name: "bash".to_string(),
                    approved:  true,
                },
            ),
        )
        .await;

        // target agent, alice, ProcessSpawned
        record_and_wait(
            &log,
            make_event(
                target_agent,
                "alice",
                AuditEventType::ProcessSpawned {
                    manifest_name: "x".to_string(),
                    parent_id:     None,
                },
            ),
        )
        .await;

        // other agent, alice, ToolCall
        record_and_wait(
            &log,
            make_event(
                other_agent,
                "alice",
                AuditEventType::ToolCall {
                    tool_name: "grep".to_string(),
                    approved:  true,
                },
            ),
        )
        .await;

        // target agent, bob, ToolCall
        record_and_wait(
            &log,
            make_event(
                target_agent,
                "bob",
                AuditEventType::ToolCall {
                    tool_name: "read".to_string(),
                    approved:  true,
                },
            ),
        )
        .await;

        let filtered = log
            .query(AuditFilter {
                agent_id:   Some(target_agent),
                user_id:    Some(UserId("alice".to_string())),
                event_type: Some("ToolCall".to_string()),
                limit:      100,
                ..Default::default()
            })
            .await;
        assert_eq!(filtered.len(), 1);
        if let AuditEventType::ToolCall { ref tool_name, .. } = filtered[0].event_type {
            assert_eq!(tool_name, "bash");
        } else {
            panic!("expected ToolCall");
        }
    }

    #[tokio::test]
    async fn test_details_json_roundtrip() {
        let (pool, _container) = setup_db().await;
        let log = PgAuditLog::new(pool);
        let aid = AgentId::new();

        let details = serde_json::json!({
            "args": ["ls", "-la"],
            "cwd": "/tmp",
            "exit_code": 0,
        });

        let event = AuditEvent {
            timestamp:  jiff::Timestamp::now(),
            agent_id:   aid,
            session_id: SessionId::new("test-session"),
            user_id:    UserId("alice".to_string()),
            event_type: AuditEventType::ToolCall {
                tool_name: "bash".to_string(),
                approved:  true,
            },
            details:    details.clone(),
        };

        record_and_wait(&log, event).await;

        let all = log
            .query(AuditFilter {
                limit: 100,
                ..Default::default()
            })
            .await;
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].details, details);
    }
}
