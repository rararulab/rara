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

//! Unified security subsystem — authentication, authorization, and approval.
//!
//! Consolidates `UserStore`, `Guard`, and `ApprovalManager` into a single
//! cohesive component that owns all security-related decisions.
//!
//! Also contains the execution approval manager that gates dangerous operations
//! behind human approval.

use std::sync::{Arc, RwLock};

use dashmap::DashMap;
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::{
    error::{KernelError, Result},
    identity::{Permission, Principal, Role, UserId, UserStore, UserStoreRef},
    session::SessionKey,
};

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
    pub session_key:  SessionKey,
    pub tool_name:    String,
    pub tool_args:    serde_json::Value,
    pub summary:      String,
    pub risk_level:   RiskLevel,
    pub requested_at: Timestamp,
    pub timeout_secs: u64,
    /// Optional context explaining why the agent wants to call this tool.
    pub context:      Option<String>,
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
    pub timeout_secs:     u64,
    /// If true, auto-approve all requests (bypass mode).
    pub auto_approve:     bool,
}

impl Default for ApprovalPolicy {
    fn default() -> Self {
        Self {
            require_approval: vec!["bash".to_string(), "shell_exec".to_string()],
            timeout_secs:     120,
            auto_approve:     false,
        }
    }
}

// ---------------------------------------------------------------------------
// ResolveError
// ---------------------------------------------------------------------------

/// Error from resolving an approval request.
#[derive(Debug, Clone)]
pub enum ResolveError {
    /// The request timed out before the user responded.
    Expired,
    /// The request ID was never seen.
    NotFound(Uuid),
}

impl std::fmt::Display for ResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Expired => write!(f, "approval request has expired"),
            Self::NotFound(id) => write!(f, "no pending approval request: {id}"),
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
    pending:    DashMap<Uuid, PendingRequest>,
    expired:    DashMap<Uuid, Timestamp>,
    policy:     RwLock<ApprovalPolicy>,
    /// Broadcast channel for notifying external listeners (e.g. Telegram
    /// adapter) when a new approval request is submitted.
    request_tx: tokio::sync::broadcast::Sender<ApprovalRequest>,
}

impl ApprovalManager {
    pub fn new(policy: ApprovalPolicy) -> Self {
        let (request_tx, _) = tokio::sync::broadcast::channel(16);
        Self {
            pending: DashMap::new(),
            expired: DashMap::new(),
            policy: RwLock::new(policy),
            request_tx,
        }
    }

    /// Subscribe to new approval requests. Channel adapters (e.g. Telegram)
    /// use this to send interactive approval prompts to users.
    pub fn subscribe_requests(&self) -> tokio::sync::broadcast::Receiver<ApprovalRequest> {
        self.request_tx.subscribe()
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
    /// If `auto_approve` is enabled in the policy, returns `Approved`
    /// immediately. If the agent already has `MAX_PENDING_PER_AGENT`
    /// pending requests, returns `Denied`.
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
            .filter(|r| r.value().request.session_key == req.session_key)
            .count();
        if agent_pending >= MAX_PENDING_PER_AGENT {
            warn!(agent_id = ?req.session_key, "approval rejected: too many pending");
            return ApprovalDecision::Denied;
        }

        let timeout = std::time::Duration::from_secs(req.timeout_secs);
        let id = req.id;

        let (tx, rx) = tokio::sync::oneshot::channel();

        // Notify external listeners (TG, web UI) before blocking.
        let _ = self.request_tx.send(req.clone());

        self.pending.insert(
            id,
            PendingRequest {
                request: req,
                sender:  tx,
            },
        );

        info!(request_id = %id, "approval request submitted, waiting for resolution");

        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(decision)) => {
                debug!(request_id = %id, ?decision, "approval resolved");
                decision
            }
            _ => {
                self.expired.insert(id, Timestamp::now());
                self.pending.remove(&id);
                warn!(request_id = %id, "approval request timed out");
                ApprovalDecision::TimedOut
            }
        }
    }

    /// Resolve a pending request (called by external API / TG callback /
    /// WebSocket).
    pub fn resolve(
        &self,
        request_id: Uuid,
        decision: ApprovalDecision,
        decided_by: Option<String>,
    ) -> std::result::Result<ApprovalResponse, ResolveError> {
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
            None => {
                if self.expired.remove(&request_id).is_some() {
                    Err(ResolveError::Expired)
                } else {
                    Err(ResolveError::NotFound(request_id))
                }
            }
        }
    }

    /// List all pending requests (for dashboard / API).
    pub fn list_pending(&self) -> Vec<ApprovalRequest> {
        self.pending
            .iter()
            .map(|r| r.value().request.clone())
            .collect()
    }

    /// Number of pending requests.
    pub fn pending_count(&self) -> usize { self.pending.len() }

    /// Update the approval policy (hot-reload).
    pub fn update_policy(&self, policy: ApprovalPolicy) {
        *self.policy.write().unwrap_or_else(|e| e.into_inner()) = policy;
    }

    /// Get a copy of the current policy.
    pub fn policy(&self) -> ApprovalPolicy {
        self.policy
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
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
// SecuritySubsystem
// ---------------------------------------------------------------------------

/// Shared reference to the [`SecuritySubsystem`].
pub type SecurityRef = Arc<SecuritySubsystem>;

/// Unified security subsystem — authentication, authorization, and approval.
pub struct SecuritySubsystem {
    user_store: UserStoreRef,
    approval:   Arc<ApprovalManager>,
}

impl SecuritySubsystem {
    pub fn new(user_store: UserStoreRef, approval: Arc<ApprovalManager>) -> Self {
        Self {
            user_store,
            approval,
        }
    }

    /// Resolve a [`Principal`] from the user store, validating that the user
    /// exists, is enabled, and has Spawn permission.
    ///
    /// Returns a fully-populated `Principal` with the correct role and
    /// permissions from the database — never a hollow placeholder.
    pub async fn resolve_principal(
        &self,
        principal: &Principal<crate::identity::Lookup>,
    ) -> Result<Principal> {
        let user = self
            .user_store
            .get_by_name(&principal.user_id.0)
            .await?
            .ok_or(KernelError::UserNotFound {
                name: principal.user_id.0.clone(),
            })?;
        if !user.enabled {
            return Err(KernelError::UserDisabled { name: user.name });
        }
        if !user.has_permission(&Permission::Spawn) {
            return Err(KernelError::PermissionDenied {
                reason: format!("user '{}' lacks Spawn permission", user.name),
            });
        }
        Ok(Principal::from_user(&user))
    }

    /// Resolve a user's role for agent routing.
    pub async fn resolve_user_role(&self, user: &UserId) -> Role {
        let user_id_str = &user.0;
        let kernel_user = match self.user_store.get_by_name(user_id_str).await {
            Ok(Some(u)) => Some(u),
            _ => {
                if let Some((_prefix, name)) = user_id_str.split_once(':') {
                    self.user_store.get_by_name(name).await.unwrap_or_default()
                } else {
                    None
                }
            }
        };
        kernel_user.map(|u| u.role).unwrap_or(Role::User)
    }

    /// Check if a tool requires approval.
    pub fn requires_approval(&self, tool_name: &str) -> bool {
        self.approval.requires_approval(tool_name)
    }

    /// Access the approval manager.
    pub fn approval(&self) -> &Arc<ApprovalManager> { &self.approval }

    /// Access the user store.
    pub fn user_store(&self) -> &Arc<dyn UserStore> { &self.user_store }
}
