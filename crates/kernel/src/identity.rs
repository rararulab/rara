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

use std::{marker::PhantomData, sync::Arc};

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

// ---------------------------------------------------------------------------
// Principal — type-state marker types
// ---------------------------------------------------------------------------

/// Marker: principal only carries the user id; permissions have not yet
/// been resolved from the user store. Never safe to use for permission
/// checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Lookup;

/// Marker: principal was fully populated from the user store with a real
/// role and permission list. Safe for authorization decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Resolved;

/// The identity under which an agent process runs.
///
/// Parameterised over a type state:
///
/// - [`Principal<Lookup>`] — produced by [`Principal::lookup`]. Only the
///   `user_id` is populated; `role` and `permissions` are placeholders.
///   Intended purely as a query key for
///   [`crate::security::SecuritySubsystem::resolve_principal`]. Calling
///   authorization methods is a **compile error**.
/// - [`Principal<Resolved>`] (the default) — produced by
///   [`Principal::from_user`] or
///   [`crate::security::SecuritySubsystem::resolve_principal`]. Carries the
///   full role + permission list from the database and is the only form that
///   exposes `has_permission`, `is_admin`, and `role`.
///
/// The default type parameter is `Resolved` so that downstream code that
/// stores or passes around fully-resolved principals stays unchanged.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Principal<State = Resolved> {
    pub user_id: UserId,
    role:        Role,
    permissions: Vec<Permission>,
    #[serde(skip)]
    _state:      PhantomData<State>,
}

impl<State> PartialEq for Principal<State> {
    fn eq(&self, other: &Self) -> bool {
        self.user_id == other.user_id
            && self.role == other.role
            && self.permissions == other.permissions
    }
}

impl<State> Eq for Principal<State> {}

// -- Constructors common to any state --------------------------------------

impl Principal<Lookup> {
    /// Create a lookup-key principal for identity resolution.
    ///
    /// The returned value only carries the user id — role and permissions
    /// are placeholders. Pass it to
    /// [`crate::security::SecuritySubsystem::resolve_principal`] to obtain a
    /// [`Principal<Resolved>`] before storing it in a session or performing
    /// any permission check.
    pub fn lookup(user_id: impl Into<String>) -> Self {
        Self {
            user_id:     UserId(user_id.into()),
            role:        Role::User,
            permissions: Vec::new(),
            _state:      PhantomData,
        }
    }
}

impl Principal<Resolved> {
    /// Downgrade a resolved principal back to a lookup key, discarding the
    /// cached role and permissions. Used when re-validating a principal
    /// through [`crate::security::SecuritySubsystem::resolve_principal`].
    pub fn into_lookup(self) -> Principal<Lookup> {
        Principal {
            user_id:     self.user_id,
            role:        Role::User,
            permissions: Vec::new(),
            _state:      PhantomData,
        }
    }

    /// Create a resolved principal from a [`KernelUser`] record.
    ///
    /// This is the canonical entry point for producing a principal that is
    /// safe to use for permission checks.
    pub fn from_user(user: &KernelUser) -> Self {
        Self {
            user_id:     UserId(user.name.clone()),
            role:        user.role,
            permissions: user.permissions.clone(),
            _state:      PhantomData,
        }
    }

    /// Check whether this principal has the given permission.
    pub fn has_permission(&self, perm: &Permission) -> bool {
        self.role == Role::Root
            || self.permissions.contains(&Permission::All)
            || self.permissions.contains(perm)
    }

    /// Whether this principal has admin privileges.
    pub fn is_admin(&self) -> bool { self.role == Role::Admin || self.role == Role::Root }

    /// Access this principal's role.
    pub fn role(&self) -> Role { self.role }

    /// Access this principal's permission list.
    pub fn permissions(&self) -> &[Permission] { &self.permissions }
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

    #[test]
    fn resolved_principal_exposes_permissions() {
        let user = root_user();
        let principal = Principal::from_user(&user);
        assert!(principal.has_permission(&Permission::Spawn));
        assert!(principal.is_admin());
        assert_eq!(principal.role(), Role::Root);
    }

    #[test]
    fn lookup_principal_only_carries_user_id() {
        let p = Principal::<Lookup>::lookup("alice");
        assert_eq!(p.user_id, UserId("alice".into()));
        // Compile-time: p.has_permission(...), p.is_admin(), p.role() do NOT
        // exist on Principal<Lookup> — only Principal<Resolved>.
    }
}
