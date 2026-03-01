//! Execution approval manager — gates dangerous operations behind human approval.

use std::sync::RwLock;

use dashmap::DashMap;
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::process::AgentId;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Risk level classification for tool invocations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

/// Decision outcome for an approval request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    Approved,
    Denied,
    TimedOut,
}

/// An approval request submitted by an agent before executing a tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalRequest {
    pub id:           Uuid,
    pub agent_id:     AgentId,
    pub tool_name:    String,
    pub tool_args:    serde_json::Value,
    pub summary:      String,
    pub risk_level:   RiskLevel,
    pub requested_at: Timestamp,
    pub timeout_secs: u64,
}

/// Response after an approval request is resolved.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalResponse {
    pub request_id: Uuid,
    pub decision:   ApprovalDecision,
    pub decided_at: Timestamp,
    pub decided_by: Option<String>,
}

/// Policy controlling which tools require approval.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalPolicy {
    /// Tool names that always require human approval.
    pub require_approval: Vec<String>,
    /// Default timeout in seconds for approval requests.
    pub timeout_secs: u64,
    /// If true, auto-approve all requests (bypass mode).
    pub auto_approve: bool,
}

impl Default for ApprovalPolicy {
    fn default() -> Self {
        Self {
            require_approval: vec!["bash".to_string(), "shell_exec".to_string()],
            timeout_secs: 120,
            auto_approve: false,
        }
    }
}

// ---------------------------------------------------------------------------
// ApprovalManager
// ---------------------------------------------------------------------------

/// Maximum pending requests per agent to prevent resource exhaustion.
const MAX_PENDING_PER_AGENT: usize = 5;

/// Internal pending request holding the oneshot sender.
struct PendingRequest {
    request: ApprovalRequest,
    sender:  tokio::sync::oneshot::Sender<ApprovalDecision>,
}

/// Manages approval requests with oneshot channels for blocking resolution.
///
/// When an agent calls a tool that requires approval, the agent's execution
/// blocks on a oneshot channel until a human resolves it (via `resolve()`)
/// or the request times out.
pub struct ApprovalManager {
    pending: DashMap<Uuid, PendingRequest>,
    policy:  RwLock<ApprovalPolicy>,
}

impl ApprovalManager {
    pub fn new(policy: ApprovalPolicy) -> Self {
        Self {
            pending: DashMap::new(),
            policy:  RwLock::new(policy),
        }
    }

    /// Check if a tool requires approval based on current policy.
    pub fn requires_approval(&self, tool_name: &str) -> bool {
        let policy = self.policy.read().unwrap_or_else(|e| e.into_inner());
        if policy.auto_approve {
            return false;
        }
        policy.require_approval.iter().any(|t| t == tool_name)
    }

    /// Submit an approval request. Blocks until resolved or timed out.
    ///
    /// If `auto_approve` is enabled in the policy, returns `Approved` immediately.
    /// If the agent already has `MAX_PENDING_PER_AGENT` pending requests, returns `Denied`.
    pub async fn request_approval(&self, req: ApprovalRequest) -> ApprovalDecision {
        // Auto-approve bypass
        {
            let policy = self.policy.read().unwrap_or_else(|e| e.into_inner());
            if policy.auto_approve {
                info!(request_id = %req.id, tool = %req.tool_name, "auto-approved");
                return ApprovalDecision::Approved;
            }
        }

        // Per-agent pending limit
        let agent_pending = self
            .pending
            .iter()
            .filter(|r| r.value().request.agent_id == req.agent_id)
            .count();
        if agent_pending >= MAX_PENDING_PER_AGENT {
            warn!(agent_id = ?req.agent_id, "approval rejected: too many pending");
            return ApprovalDecision::Denied;
        }

        let timeout = std::time::Duration::from_secs(req.timeout_secs);
        let id = req.id;

        let (tx, rx) = tokio::sync::oneshot::channel();
        self.pending.insert(id, PendingRequest { request: req, sender: tx });

        info!(request_id = %id, "approval request submitted, waiting for resolution");

        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(decision)) => {
                debug!(request_id = %id, ?decision, "approval resolved");
                decision
            }
            _ => {
                self.pending.remove(&id);
                warn!(request_id = %id, "approval request timed out");
                ApprovalDecision::TimedOut
            }
        }
    }

    /// Resolve a pending request (called by external API / TG callback / WebSocket).
    pub fn resolve(
        &self,
        request_id: Uuid,
        decision: ApprovalDecision,
        decided_by: Option<String>,
    ) -> Result<ApprovalResponse, String> {
        match self.pending.remove(&request_id) {
            Some((_, pending)) => {
                let response = ApprovalResponse {
                    request_id,
                    decision,
                    decided_at: Timestamp::now(),
                    decided_by,
                };
                let _ = pending.sender.send(decision);
                info!(request_id = %request_id, ?decision, "approval resolved");
                Ok(response)
            }
            None => Err(format!("no pending approval request: {request_id}")),
        }
    }

    /// List all pending requests (for dashboard / API).
    pub fn list_pending(&self) -> Vec<ApprovalRequest> {
        self.pending.iter().map(|r| r.value().request.clone()).collect()
    }

    /// Number of pending requests.
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// Update the approval policy (hot-reload).
    pub fn update_policy(&self, policy: ApprovalPolicy) {
        *self.policy.write().unwrap_or_else(|e| e.into_inner()) = policy;
    }

    /// Get a copy of the current policy.
    pub fn policy(&self) -> ApprovalPolicy {
        self.policy.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Classify the risk level of a tool invocation.
    pub fn classify_risk(tool_name: &str) -> RiskLevel {
        match tool_name {
            "bash" | "shell_exec" => RiskLevel::Critical,
            "file_write" | "file_delete" | "write" | "edit" => RiskLevel::High,
            "web_fetch" | "browser_navigate" => RiskLevel::Medium,
            _ => RiskLevel::Low,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn default_manager() -> ApprovalManager {
        ApprovalManager::new(ApprovalPolicy::default())
    }

    fn make_request(agent_name: &str, tool_name: &str, timeout_secs: u64) -> ApprovalRequest {
        let _ = agent_name;
        ApprovalRequest {
            id:           Uuid::new_v4(),
            agent_id:     AgentId::new(),
            tool_name:    tool_name.to_string(),
            tool_args:    serde_json::json!({}),
            summary:      format!("execute {tool_name}"),
            risk_level:   ApprovalManager::classify_risk(tool_name),
            requested_at: Timestamp::now(),
            timeout_secs,
        }
    }

    #[test]
    fn requires_approval_default_policy() {
        let mgr = default_manager();
        assert!(mgr.requires_approval("bash"));
        assert!(mgr.requires_approval("shell_exec"));
        assert!(!mgr.requires_approval("file_read"));
    }

    #[test]
    fn requires_approval_custom_policy() {
        let policy = ApprovalPolicy {
            require_approval: vec!["file_write".to_string()],
            timeout_secs: 30,
            auto_approve: false,
        };
        let mgr = ApprovalManager::new(policy);
        assert!(mgr.requires_approval("file_write"));
        assert!(!mgr.requires_approval("bash"));
    }

    #[test]
    fn requires_approval_auto_approve_bypasses() {
        let policy = ApprovalPolicy {
            require_approval: vec!["bash".to_string()],
            timeout_secs: 60,
            auto_approve: true,
        };
        let mgr = ApprovalManager::new(policy);
        assert!(!mgr.requires_approval("bash"));
    }

    #[test]
    fn classify_risk_levels() {
        assert_eq!(ApprovalManager::classify_risk("bash"), RiskLevel::Critical);
        assert_eq!(ApprovalManager::classify_risk("shell_exec"), RiskLevel::Critical);
        assert_eq!(ApprovalManager::classify_risk("file_write"), RiskLevel::High);
        assert_eq!(ApprovalManager::classify_risk("file_delete"), RiskLevel::High);
        assert_eq!(ApprovalManager::classify_risk("web_fetch"), RiskLevel::Medium);
        assert_eq!(ApprovalManager::classify_risk("file_read"), RiskLevel::Low);
        assert_eq!(ApprovalManager::classify_risk("unknown"), RiskLevel::Low);
    }

    #[test]
    fn resolve_nonexistent_returns_error() {
        let mgr = default_manager();
        let result = mgr.resolve(Uuid::new_v4(), ApprovalDecision::Approved, None);
        assert!(result.is_err());
    }

    #[test]
    fn list_pending_empty() {
        let mgr = default_manager();
        assert!(mgr.list_pending().is_empty());
        assert_eq!(mgr.pending_count(), 0);
    }

    #[test]
    fn update_policy_hot_reload() {
        let mgr = default_manager();
        assert!(mgr.requires_approval("bash"));

        mgr.update_policy(ApprovalPolicy {
            require_approval: vec!["file_write".to_string()],
            timeout_secs: 30,
            auto_approve: false,
        });

        assert!(!mgr.requires_approval("bash"));
        assert!(mgr.requires_approval("file_write"));
        assert_eq!(mgr.policy().timeout_secs, 30);
    }

    #[tokio::test]
    async fn request_approval_auto_approve() {
        let policy = ApprovalPolicy {
            require_approval: vec!["bash".to_string()],
            timeout_secs: 60,
            auto_approve: true,
        };
        let mgr = ApprovalManager::new(policy);
        let req = make_request("agent-1", "bash", 60);
        let decision = mgr.request_approval(req).await;
        assert_eq!(decision, ApprovalDecision::Approved);
        assert_eq!(mgr.pending_count(), 0);
    }

    #[tokio::test]
    async fn request_approval_timeout() {
        let mgr = Arc::new(default_manager());
        let req = make_request("agent-1", "bash", 1); // 1 second timeout
        let decision = mgr.request_approval(req).await;
        assert_eq!(decision, ApprovalDecision::TimedOut);
        assert_eq!(mgr.pending_count(), 0);
    }

    #[tokio::test]
    async fn request_approval_approved() {
        let mgr = Arc::new(default_manager());
        let req = make_request("agent-1", "bash", 60);
        let request_id = req.id;

        let mgr2 = Arc::clone(&mgr);
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            let result = mgr2.resolve(request_id, ApprovalDecision::Approved, Some("admin".into()));
            assert!(result.is_ok());
        });

        let decision = mgr.request_approval(req).await;
        assert_eq!(decision, ApprovalDecision::Approved);
    }

    #[tokio::test]
    async fn request_approval_denied() {
        let mgr = Arc::new(default_manager());
        let req = make_request("agent-1", "bash", 60);
        let request_id = req.id;

        let mgr2 = Arc::clone(&mgr);
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            let _ = mgr2.resolve(request_id, ApprovalDecision::Denied, None);
        });

        let decision = mgr.request_approval(req).await;
        assert_eq!(decision, ApprovalDecision::Denied);
    }

    #[tokio::test]
    async fn max_pending_per_agent() {
        let mgr = Arc::new(default_manager());
        let agent_id = AgentId::new();

        // Fill up MAX_PENDING_PER_AGENT requests
        let mut ids = Vec::new();
        for _ in 0..MAX_PENDING_PER_AGENT {
            let mut req = make_request("agent-1", "bash", 300);
            req.agent_id = agent_id;
            ids.push(req.id);
            let mgr_clone = Arc::clone(&mgr);
            tokio::spawn(async move {
                mgr_clone.request_approval(req).await;
            });
        }

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(mgr.pending_count(), MAX_PENDING_PER_AGENT);

        // Next request from same agent should be denied
        let mut req6 = make_request("agent-1", "bash", 300);
        req6.agent_id = agent_id;
        let decision = mgr.request_approval(req6).await;
        assert_eq!(decision, ApprovalDecision::Denied);

        // Different agent can still submit
        let req_other = make_request("agent-2", "bash", 300);
        let other_id = req_other.id;
        let mgr2 = Arc::clone(&mgr);
        tokio::spawn(async move { mgr2.request_approval(req_other).await; });
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        assert_eq!(mgr.pending_count(), MAX_PENDING_PER_AGENT + 1);

        // Cleanup
        for id in &ids {
            let _ = mgr.resolve(*id, ApprovalDecision::Denied, None);
        }
        let _ = mgr.resolve(other_id, ApprovalDecision::Denied, None);
    }

    #[tokio::test]
    async fn list_pending_shows_active_requests() {
        let mgr = Arc::new(default_manager());
        let req = make_request("agent-1", "bash", 300);
        let request_id = req.id;
        let tool = req.tool_name.clone();

        let mgr2 = Arc::clone(&mgr);
        tokio::spawn(async move { mgr2.request_approval(req).await; });

        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        let pending = mgr.list_pending();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, request_id);
        assert_eq!(pending[0].tool_name, tool);

        // Cleanup
        let _ = mgr.resolve(request_id, ApprovalDecision::Denied, None);
    }
}
