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

use std::sync::Arc;

use crate::{
    approval::ApprovalManager,
    error::{KernelError, Result},
    guard::{Guard, GuardContext, Verdict},
    process::principal::{Principal, Role, UserId},
    process::user::{Permission, UserStore},
};

/// Unified security subsystem — authentication, authorization, and approval.
pub struct SecuritySubsystem {
    user_store: Arc<dyn UserStore>,
    guard:      Arc<dyn Guard>,
    approval:   Arc<ApprovalManager>,
}

impl SecuritySubsystem {
    pub fn new(
        user_store: Arc<dyn UserStore>,
        guard: Arc<dyn Guard>,
        approval: Arc<ApprovalManager>,
    ) -> Self {
        Self { user_store, guard, approval }
    }

    /// Validate that the principal's user exists, is enabled, and has Spawn
    /// permission.
    pub async fn validate_principal(&self, principal: &Principal) -> Result<()> {
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
        Ok(())
    }

    /// Resolve a user's role for agent routing.
    pub async fn resolve_user_role(&self, user: &UserId) -> Role {
        let user_id_str = &user.0;
        let kernel_user = match self.user_store.get_by_name(user_id_str).await {
            Ok(Some(u)) => Some(u),
            _ => {
                if let Some((_prefix, name)) = user_id_str.split_once(':') {
                    match self.user_store.get_by_name(name).await {
                        Ok(found) => found,
                        Err(_) => None,
                    }
                } else {
                    None
                }
            }
        };
        kernel_user.map(|u| u.role).unwrap_or(Role::User)
    }

    /// Check a batch of tool calls against the guard.
    pub async fn check_guard_batch(
        &self,
        ctx: &GuardContext,
        checks: &[(String, serde_json::Value)],
    ) -> Vec<Verdict> {
        let mut verdicts = Vec::with_capacity(checks.len());
        for (tool_name, args) in checks {
            let verdict = self.guard.check_tool(ctx, tool_name, args).await;
            verdicts.push(verdict);
        }
        verdicts
    }

    /// Check if a tool requires approval.
    pub fn requires_approval(&self, tool_name: &str) -> bool {
        self.approval.requires_approval(tool_name)
    }

    /// Access the approval manager.
    pub fn approval(&self) -> &Arc<ApprovalManager> {
        &self.approval
    }

    /// Access the guard.
    pub fn guard(&self) -> &Arc<dyn Guard> {
        &self.guard
    }

    /// Access the user store.
    pub fn user_store(&self) -> &Arc<dyn UserStore> {
        &self.user_store
    }

    /// Create a no-op security subsystem for testing.
    pub fn noop() -> Self {
        Self {
            user_store: Arc::new(crate::defaults::noop_user_store::NoopUserStore),
            guard: Arc::new(crate::defaults::noop::NoopGuard),
            approval: Arc::new(ApprovalManager::new(crate::approval::ApprovalPolicy::default())),
        }
    }
}
