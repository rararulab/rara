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

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::Result;

/// User identity (stores user **name**, not UUID).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct UserId(pub String);

impl std::fmt::Display for UserId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "{}", self.0) }
}

/// User role determining permission level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Role {
    Root,
    Admin,
    User,
}

/// Fine-grained permission granted to a kernel user.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Permission {
    /// Super permission — root only, bypasses all checks.
    All,
    /// Can spawn agent processes.
    Spawn,
    /// Can manage users (create, modify, delete).
    ManageUsers,
    /// Can use all tools without restriction.
    UseAllTools,
    /// Can use a specific named tool.
    UseTool(crate::tool::ToolName),
    /// Can manage skills.
    ManageSkills,
    /// Can manage MCP servers.
    ManageMcp,
}

/// Kernel user — analogous to a record in `/etc/passwd`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KernelUser {
    /// Primary key — must be unique across all users.
    pub name:        String,
    pub role:        Role,
    pub permissions: Vec<Permission>,
    pub enabled:     bool,
}

impl KernelUser {
    /// Check whether this user has the given permission.
    pub fn has_permission(&self, perm: &Permission) -> bool {
        self.permissions.contains(&Permission::All) || self.permissions.contains(perm)
    }

    /// Check whether this user can use the named tool.
    pub fn can_use_tool(&self, tool_name: &str) -> bool {
        self.has_permission(&Permission::All)
            || self.has_permission(&Permission::UseAllTools)
            || self
                .permissions
                .iter()
                .any(|p| matches!(p, Permission::UseTool(n) if n.as_str() == tool_name))
    }
}

/// The identity under which an agent process runs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Principal {
    pub user_id:     UserId,
    pub role:        Role,
    pub permissions: Vec<Permission>,
}

impl Principal {
    /// Create a principal from a [`KernelUser`].
    pub fn from_user(user: &KernelUser) -> Self {
        Self {
            user_id:     UserId(user.name.clone()),
            role:        user.role,
            permissions: user.permissions.clone(),
        }
    }

    /// Check whether this principal has the given permission.
    pub fn has_permission(&self, perm: &Permission) -> bool {
        self.role == Role::Root
            || self.permissions.contains(&Permission::All)
            || self.permissions.contains(perm)
    }

    /// Create a lookup-key principal for identity resolution.
    ///
    /// The returned `Principal` only carries the user id — role and
    /// permissions are placeholders. Call
    /// `SecuritySubsystem::resolve_principal` to obtain a fully-populated
    /// principal before storing it in a session.
    pub fn lookup(user_id: impl Into<String>) -> Self {
        Self {
            user_id:     UserId(user_id.into()),
            role:        Role::User,
            permissions: vec![],
        }
    }

    /// Whether this principal has admin privileges.
    pub fn is_admin(&self) -> bool { self.role == Role::Admin || self.role == Role::Root }
}

pub type UserStoreRef = Arc<dyn UserStore>;

/// Read-only user lookup, backed by in-memory config.
#[async_trait]
pub trait UserStore: Send + Sync {
    async fn get_by_name(&self, name: &str) -> Result<Option<KernelUser>>;
    async fn list(&self) -> Result<Vec<KernelUser>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root_user() -> KernelUser {
        KernelUser {
            name:        "root".into(),
            role:        Role::Root,
            permissions: vec![Permission::All],
            enabled:     true,
        }
    }

    fn admin_user() -> KernelUser {
        KernelUser {
            name:        "admin".into(),
            role:        Role::Admin,
            permissions: vec![Permission::All],
            enabled:     true,
        }
    }

    fn regular_user() -> KernelUser {
        KernelUser {
            name:        "user".into(),
            role:        Role::User,
            permissions: vec![],
            enabled:     true,
        }
    }

    #[test]
    fn root_can_use_any_tool() {
        let user = root_user();
        assert!(user.can_use_tool("bash"));
        assert!(user.can_use_tool("write-file"));
        assert!(user.can_use_tool("http-fetch"));
    }

    #[test]
    fn admin_can_use_any_tool() {
        let user = admin_user();
        assert!(user.can_use_tool("bash"));
        assert!(user.can_use_tool("write-file"));
    }

    #[test]
    fn regular_user_cannot_use_any_tool() {
        let user = regular_user();
        assert!(!user.can_use_tool("bash"));
        assert!(!user.can_use_tool("write-file"));
        assert!(!user.can_use_tool("http-fetch"));
        assert!(!user.can_use_tool("read-file"));
    }

    #[test]
    fn user_with_specific_tool_permission() {
        let user = KernelUser {
            name:        "limited".into(),
            role:        Role::User,
            permissions: vec![
                Permission::Spawn,
                Permission::UseTool(crate::tool::ToolName::new("http-fetch")),
            ],
            enabled:     true,
        };
        assert!(user.can_use_tool("http-fetch"));
        assert!(!user.can_use_tool("bash"));
    }

    #[test]
    fn user_with_use_all_tools_permission() {
        let user = KernelUser {
            name:        "power_user".into(),
            role:        Role::User,
            permissions: vec![Permission::Spawn, Permission::UseAllTools],
            enabled:     true,
        };
        assert!(user.can_use_tool("bash"));
        assert!(user.can_use_tool("write-file"));
    }
}
