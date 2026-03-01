use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct SystemStats {
    pub active_processes: usize,
    pub total_spawned: u64,
    pub total_completed: u64,
    pub total_failed: u64,
    pub global_semaphore_available: usize,
    pub total_tokens_consumed: u64,
    pub uptime_ms: u64,
}

#[derive(Debug, Deserialize)]
pub struct ProcessStats {
    pub agent_id: String,
    pub name: String,
    pub state: String,
    pub parent_id: Option<String>,
    pub session_id: String,
    pub uptime_ms: u64,
    pub metrics: MetricsSnapshot,
    pub children: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct MetricsSnapshot {
    pub messages_received: u64,
    pub llm_calls: u64,
    pub tool_calls: u64,
    pub tokens_consumed: u64,
}

#[derive(Debug, Deserialize)]
pub struct AgentInfo {
    pub name: String,
    pub role: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ApprovalRequest {
    pub id: String,
    pub agent_id: String,
    pub tool_name: String,
    pub risk_level: String,
    pub requested_at: String,
}

#[derive(Debug, Deserialize)]
pub struct AuditEvent {
    pub timestamp: String,
    pub agent_id: String,
    pub event_type: String,
    pub details: serde_json::Value,
}
