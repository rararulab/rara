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

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct SystemStats {
    pub active_processes:           usize,
    pub total_spawned:              u64,
    pub total_completed:            u64,
    pub total_failed:               u64,
    pub global_semaphore_available: usize,
    pub total_tokens_consumed:      u64,
    pub uptime_ms:                  u64,
}

#[derive(Debug, Deserialize)]
pub struct ProcessStats {
    pub agent_id:   String,
    pub name:       String,
    pub state:      String,
    pub parent_id:  Option<String>,
    pub session_id: String,
    pub uptime_ms:  u64,
    pub metrics:    MetricsSnapshot,
    pub children:   Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct MetricsSnapshot {
    pub messages_received: u64,
    pub llm_calls:         u64,
    pub tool_calls:        u64,
    pub tokens_consumed:   u64,
}

#[derive(Debug, Deserialize)]
pub struct AgentInfo {
    pub name:        String,
    pub role:        Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ApprovalRequest {
    pub id:           String,
    pub agent_id:     String,
    pub tool_name:    String,
    pub risk_level:   String,
    pub requested_at: String,
}

#[derive(Debug, Deserialize)]
pub struct AuditEvent {
    pub timestamp:  String,
    pub agent_id:   String,
    pub event_type: String,
    pub details:    serde_json::Value,
}

#[derive(Debug, Clone, Deserialize)]
pub struct KernelEventCommonFields {
    pub timestamp:  String,
    pub event_type: String,
    pub priority:   String,
    pub agent_id:   Option<String>,
    pub summary:    String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct KernelEventEnvelope {
    pub common: KernelEventCommonFields,
    pub event:  serde_json::Value,
}
