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

//! Structured audit logging for agent behavior tracking.
//!
//! Every significant agent action (spawn, tool call, LLM call, memory access,
//! signal, completion, failure) is recorded as an [`AuditEvent`].
//!
//! The [`AuditLog`] trait defines a pluggable backend.
//! [`InMemoryAuditLog`] provides a bounded, lock-free default implementation
//! suitable for development and testing.

use std::{collections::VecDeque, sync::Arc};

use async_trait::async_trait;
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::process::{AgentId, SessionId, principal::UserId};

// ---------------------------------------------------------------------------
// AuditEvent
// ---------------------------------------------------------------------------

/// A single audit event recording an agent action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    /// When this event occurred.
    pub timestamp:  Timestamp,
    /// The agent that produced the event.
    pub agent_id:   AgentId,
    /// The session the agent belongs to.
    pub session_id: SessionId,
    /// The user identity running the agent.
    pub user_id:    UserId,
    /// What kind of action this event represents.
    pub event_type: AuditEventType,
    /// Arbitrary structured details (tool args, error messages, etc.).
    pub details:    serde_json::Value,
}

/// Discriminated event types for agent actions.
#[derive(Debug, Clone, Serialize, Deserialize, strum::IntoStaticStr)]
pub enum AuditEventType {
    /// An agent process was spawned.
    ProcessSpawned {
        manifest_name: String,
        parent_id:     Option<AgentId>,
    },
    /// An agent process completed successfully.
    ProcessCompleted { result: String },
    /// An agent process failed.
    ProcessFailed { error: String },
    /// An agent process was killed.
    ProcessKilled { by: AgentId },
    /// A tool was called.
    ToolCall { tool_name: String, approved: bool },
    /// An LLM call was made.
    LlmCall {
        model:       String,
        tokens_in:   u64,
        tokens_out:  u64,
        duration_ms: u64,
    },
    /// A memory operation was performed.
    MemoryAccess {
        operation: MemoryOp,
        key:       String,
    },
    /// A signal was sent to another process.
    SignalSent {
        target_id: AgentId,
        signal:    String,
    },
}

/// Memory operation type.
#[derive(Debug, Clone, Serialize, Deserialize, strum::Display)]
pub enum MemoryOp {
    /// Store a value in agent-local memory.
    Store,
    /// Recall a value from agent-local memory.
    Recall,
    /// Store a value in shared (cross-agent) memory.
    SharedStore,
    /// Recall a value from shared (cross-agent) memory.
    SharedRecall,
}

// ---------------------------------------------------------------------------
// AuditFilter
// ---------------------------------------------------------------------------

/// Filter criteria for querying audit events.
#[derive(Debug, Default, Clone)]
pub struct AuditFilter {
    /// Only return events from this agent.
    pub agent_id:   Option<AgentId>,
    /// Only return events from this user.
    pub user_id:    Option<UserId>,
    /// Only return events matching this event type name (e.g.
    /// "ProcessSpawned").
    pub event_type: Option<String>,
    /// Only return events after this timestamp.
    pub since:      Option<Timestamp>,
    /// Maximum number of events to return.
    pub limit:      usize,
}

// ---------------------------------------------------------------------------
// AuditLog trait
// ---------------------------------------------------------------------------

/// Pluggable audit log backend.
///
/// Implementations must be `Send + Sync` so they can be shared across
/// tokio tasks via `Arc<dyn AuditLog>`.
#[async_trait]
pub trait AuditLog: Send + Sync {
    /// Record a single audit event.
    ///
    /// Implementations should be non-blocking on the hot path.
    async fn record(&self, event: AuditEvent);

    /// Query events matching the given filter.
    async fn query(&self, filter: AuditFilter) -> Vec<AuditEvent>;
}

// ---------------------------------------------------------------------------
// InMemoryAuditLog
// ---------------------------------------------------------------------------

/// Default in-memory audit log with a bounded event buffer.
///
/// Stores up to `capacity` events in a `VecDeque`. When the buffer is full,
/// the oldest event is dropped. Thread-safe via `RwLock`.
pub struct InMemoryAuditLog {
    events:   RwLock<VecDeque<AuditEvent>>,
    capacity: usize,
}

impl InMemoryAuditLog {
    /// Create a new in-memory audit log with the given capacity.
    pub fn new(capacity: usize) -> Self {
        Self {
            events: RwLock::new(VecDeque::with_capacity(capacity.min(10_000))),
            capacity,
        }
    }
}

impl Default for InMemoryAuditLog {
    fn default() -> Self { Self::new(10_000) }
}

#[async_trait]
impl AuditLog for InMemoryAuditLog {
    async fn record(&self, event: AuditEvent) {
        let mut events = self.events.write().await;
        if events.len() >= self.capacity {
            events.pop_front();
        }
        events.push_back(event);
    }

    async fn query(&self, filter: AuditFilter) -> Vec<AuditEvent> {
        let events = self.events.read().await;
        let limit = if filter.limit == 0 {
            usize::MAX
        } else {
            filter.limit
        };

        events
            .iter()
            .filter(|e| {
                if let Some(ref id) = filter.agent_id {
                    if e.agent_id != *id {
                        return false;
                    }
                }
                if let Some(ref uid) = filter.user_id {
                    if e.user_id != *uid {
                        return false;
                    }
                }
                if let Some(ref et) = filter.event_type {
                    if !event_type_name(&e.event_type).eq_ignore_ascii_case(et) {
                        return false;
                    }
                }
                if let Some(ref since) = filter.since {
                    if e.timestamp < *since {
                        return false;
                    }
                }
                true
            })
            .take(limit)
            .cloned()
            .collect()
    }
}

/// Extract the variant name from an `AuditEventType` for filtering.
pub fn event_type_name(et: &AuditEventType) -> &'static str { et.into() }

// ---------------------------------------------------------------------------
// Helper: fire-and-forget recording
// ---------------------------------------------------------------------------

/// Record an audit event without blocking the caller.
///
/// Spawns a lightweight tokio task so the hot path is never blocked by
/// the audit log write. If no tokio runtime is available (e.g. in
/// synchronous test contexts), the event is silently dropped.
pub fn record_async(audit_log: &Arc<dyn AuditLog>, event: AuditEvent) {
    let log = Arc::clone(audit_log);
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        handle.spawn(async move {
            log.record(event).await;
        });
    }
}

// ---------------------------------------------------------------------------
// ToolCallRecorder — dedicated tool call audit trail
// ---------------------------------------------------------------------------

/// Records tool call invocations for audit trail.
///
/// Separate from [`AuditLog`] — this is a dedicated, structured recorder
/// for tool calls that can be backed by a persistent store (e.g., AgentFS)
/// for post-hoc analysis and debugging.
#[async_trait]
pub trait ToolCallRecorder: Send + Sync {
    /// Record a completed tool call.
    async fn record_tool_call(
        &self,
        agent_id: crate::process::AgentId,
        tool_name: &str,
        args: &serde_json::Value,
        result: &serde_json::Value,
        success: bool,
        duration_ms: u64,
    );
}

/// No-op recorder — default for tests and when no persistent backend is
/// configured.
pub struct NoopToolCallRecorder;

#[async_trait]
impl ToolCallRecorder for NoopToolCallRecorder {
    async fn record_tool_call(
        &self,
        _agent_id: crate::process::AgentId,
        _tool_name: &str,
        _args: &serde_json::Value,
        _result: &serde_json::Value,
        _success: bool,
        _duration_ms: u64,
    ) {
    }
}
// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_event(agent_id: AgentId, user: &str, event_type: AuditEventType) -> AuditEvent {
        AuditEvent {
            timestamp: Timestamp::now(),
            agent_id,
            session_id: SessionId::new(),
            user_id: UserId(user.to_string()),
            event_type,
            details: serde_json::Value::Null,
        }
    }

    #[tokio::test]
    async fn test_record_and_query_all() {
        let log = InMemoryAuditLog::new(100);
        let aid = AgentId::new();

        log.record(make_event(
            aid,
            "alice",
            AuditEventType::ProcessSpawned {
                manifest_name: "scout".to_string(),
                parent_id:     None,
            },
        ))
        .await;

        log.record(make_event(
            aid,
            "alice",
            AuditEventType::ProcessCompleted {
                result: "done".to_string(),
            },
        ))
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
    async fn test_query_by_agent_id() {
        let log = InMemoryAuditLog::new(100);
        let a1 = AgentId::new();
        let a2 = AgentId::new();

        log.record(make_event(
            a1,
            "alice",
            AuditEventType::ProcessSpawned {
                manifest_name: "scout".to_string(),
                parent_id:     None,
            },
        ))
        .await;

        log.record(make_event(
            a2,
            "bob",
            AuditEventType::ProcessSpawned {
                manifest_name: "worker".to_string(),
                parent_id:     None,
            },
        ))
        .await;

        let filtered = log
            .query(AuditFilter {
                agent_id: Some(a1),
                limit: 100,
                ..Default::default()
            })
            .await;
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].agent_id, a1);
    }

    #[tokio::test]
    async fn test_query_by_user_id() {
        let log = InMemoryAuditLog::new(100);
        let aid = AgentId::new();

        log.record(make_event(
            aid,
            "alice",
            AuditEventType::ToolCall {
                tool_name: "bash".to_string(),
                approved:  true,
            },
        ))
        .await;

        log.record(make_event(
            AgentId::new(),
            "bob",
            AuditEventType::ToolCall {
                tool_name: "grep".to_string(),
                approved:  true,
            },
        ))
        .await;

        let filtered = log
            .query(AuditFilter {
                user_id: Some(UserId("alice".to_string())),
                limit: 100,
                ..Default::default()
            })
            .await;
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].user_id.0, "alice");
    }

    #[tokio::test]
    async fn test_query_by_event_type() {
        let log = InMemoryAuditLog::new(100);
        let aid = AgentId::new();

        log.record(make_event(
            aid,
            "alice",
            AuditEventType::ProcessSpawned {
                manifest_name: "scout".to_string(),
                parent_id:     None,
            },
        ))
        .await;

        log.record(make_event(
            aid,
            "alice",
            AuditEventType::ToolCall {
                tool_name: "bash".to_string(),
                approved:  true,
            },
        ))
        .await;

        log.record(make_event(
            aid,
            "alice",
            AuditEventType::ProcessCompleted {
                result: "done".to_string(),
            },
        ))
        .await;

        let filtered = log
            .query(AuditFilter {
                event_type: Some("ToolCall".to_string()),
                limit: 100,
                ..Default::default()
            })
            .await;
        assert_eq!(filtered.len(), 1);
    }

    #[tokio::test]
    async fn test_bounded_buffer_evicts_oldest() {
        let log = InMemoryAuditLog::new(3);
        let aid = AgentId::new();

        for i in 0..5 {
            log.record(make_event(
                aid,
                "alice",
                AuditEventType::ProcessCompleted {
                    result: format!("result-{i}"),
                },
            ))
            .await;
        }

        let all = log
            .query(AuditFilter {
                limit: 100,
                ..Default::default()
            })
            .await;
        // Only 3 events should remain (the last 3)
        assert_eq!(all.len(), 3);
        // Verify oldest were evicted
        if let AuditEventType::ProcessCompleted { ref result } = all[0].event_type {
            assert_eq!(result, "result-2");
        } else {
            panic!("expected ProcessCompleted");
        }
    }

    #[tokio::test]
    async fn test_query_with_limit() {
        let log = InMemoryAuditLog::new(100);
        let aid = AgentId::new();

        for _ in 0..10 {
            log.record(make_event(
                aid,
                "alice",
                AuditEventType::ProcessSpawned {
                    manifest_name: "test".to_string(),
                    parent_id:     None,
                },
            ))
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
    async fn test_query_combined_filters() {
        let log = InMemoryAuditLog::new(100);
        let target_agent = AgentId::new();
        let other_agent = AgentId::new();

        // target agent, alice, ToolCall
        log.record(make_event(
            target_agent,
            "alice",
            AuditEventType::ToolCall {
                tool_name: "bash".to_string(),
                approved:  true,
            },
        ))
        .await;

        // target agent, alice, ProcessSpawned
        log.record(make_event(
            target_agent,
            "alice",
            AuditEventType::ProcessSpawned {
                manifest_name: "x".to_string(),
                parent_id:     None,
            },
        ))
        .await;

        // other agent, alice, ToolCall
        log.record(make_event(
            other_agent,
            "alice",
            AuditEventType::ToolCall {
                tool_name: "grep".to_string(),
                approved:  true,
            },
        ))
        .await;

        // target agent, bob, ToolCall
        log.record(make_event(
            target_agent,
            "bob",
            AuditEventType::ToolCall {
                tool_name: "read".to_string(),
                approved:  true,
            },
        ))
        .await;

        let filtered = log
            .query(AuditFilter {
                agent_id: Some(target_agent),
                user_id: Some(UserId("alice".to_string())),
                event_type: Some("ToolCall".to_string()),
                limit: 100,
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
    async fn test_record_async_fires() {
        let log: Arc<dyn AuditLog> = Arc::new(InMemoryAuditLog::new(100));
        let aid = AgentId::new();

        record_async(
            &log,
            make_event(
                aid,
                "alice",
                AuditEventType::ProcessSpawned {
                    manifest_name: "test".to_string(),
                    parent_id:     None,
                },
            ),
        );

        // Give the spawned task time to complete
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let all = log
            .query(AuditFilter {
                limit: 100,
                ..Default::default()
            })
            .await;
        assert_eq!(all.len(), 1);
    }

    #[test]
    fn test_event_type_name_mapping() {
        assert_eq!(
            event_type_name(&AuditEventType::ProcessSpawned {
                manifest_name: "x".to_string(),
                parent_id:     None,
            }),
            "ProcessSpawned"
        );
        assert_eq!(
            event_type_name(&AuditEventType::ProcessCompleted {
                result: "x".to_string(),
            }),
            "ProcessCompleted"
        );
        assert_eq!(
            event_type_name(&AuditEventType::ProcessFailed {
                error: "x".to_string(),
            }),
            "ProcessFailed"
        );
        assert_eq!(
            event_type_name(&AuditEventType::ProcessKilled { by: AgentId::new() }),
            "ProcessKilled"
        );
        assert_eq!(
            event_type_name(&AuditEventType::ToolCall {
                tool_name: "x".to_string(),
                approved:  true,
            }),
            "ToolCall"
        );
        assert_eq!(
            event_type_name(&AuditEventType::LlmCall {
                model:       "x".to_string(),
                tokens_in:   0,
                tokens_out:  0,
                duration_ms: 0,
            }),
            "LlmCall"
        );
        assert_eq!(
            event_type_name(&AuditEventType::MemoryAccess {
                operation: MemoryOp::Store,
                key:       "x".to_string(),
            }),
            "MemoryAccess"
        );
        assert_eq!(
            event_type_name(&AuditEventType::SignalSent {
                target_id: AgentId::new(),
                signal:    "x".to_string(),
            }),
            "SignalSent"
        );
    }

    #[tokio::test]
    async fn test_default_capacity() {
        let log = InMemoryAuditLog::default();
        assert_eq!(log.capacity, 10_000);
    }

    #[test]
    fn test_audit_event_serializable() {
        let event = AuditEvent {
            timestamp:  Timestamp::now(),
            agent_id:   AgentId::new(),
            session_id: SessionId::new(),
            user_id:    UserId("alice".to_string()),
            event_type: AuditEventType::ToolCall {
                tool_name: "bash".to_string(),
                approved:  true,
            },
            details:    serde_json::json!({"args": ["ls", "-la"]}),
        };
        let json = serde_json::to_string(&event);
        assert!(json.is_ok());
    }
}
