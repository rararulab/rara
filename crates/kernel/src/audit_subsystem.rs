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

//! Unified audit subsystem — event logging and tool call recording.
//!
//! Combines [`AuditLog`] and [`ToolCallRecorder`] into a single cohesive
//! subsystem, replacing two flat fields on `KernelInner`.

use std::sync::Arc;

use crate::{
    audit::{AuditEvent, AuditFilter, AuditLog, InMemoryAuditLog, NoopToolCallRecorder, ToolCallRecorder},
    process::AgentId,
};

/// Unified audit subsystem — event logging and tool call recording.
pub struct AuditSubsystem {
    audit_log: Arc<dyn AuditLog>,
    tool_call_recorder: Arc<dyn ToolCallRecorder>,
}

impl AuditSubsystem {
    pub fn new(
        audit_log: Arc<dyn AuditLog>,
        tool_call_recorder: Arc<dyn ToolCallRecorder>,
    ) -> Self {
        Self { audit_log, tool_call_recorder }
    }

    /// Record a structured audit event (fire-and-forget).
    pub fn record(&self, event: AuditEvent) {
        crate::audit::record_async(&self.audit_log, event);
    }

    /// Record a tool call invocation.
    pub async fn record_tool_call(
        &self,
        agent_id: AgentId,
        tool_name: &str,
        args: &serde_json::Value,
        result: &serde_json::Value,
        success: bool,
        duration_ms: u64,
    ) {
        self.tool_call_recorder
            .record_tool_call(agent_id, tool_name, args, result, success, duration_ms)
            .await;
    }

    /// Query the audit log for events matching the filter.
    pub async fn query(&self, filter: AuditFilter) -> Vec<AuditEvent> {
        self.audit_log.query(filter).await
    }

    /// Access the raw audit log.
    pub fn audit_log(&self) -> &Arc<dyn AuditLog> {
        &self.audit_log
    }

    /// Access the tool call recorder.
    pub fn tool_call_recorder(&self) -> &Arc<dyn ToolCallRecorder> {
        &self.tool_call_recorder
    }

    /// Create a no-op audit subsystem for testing.
    pub fn noop() -> Self {
        Self {
            audit_log: Arc::new(InMemoryAuditLog::default()),
            tool_call_recorder: Arc::new(NoopToolCallRecorder),
        }
    }
}
