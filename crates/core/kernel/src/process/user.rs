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

//! Kernel user management — user records, permissions, and platform identity
//! bindings.
//!
//! This module defines the OS-level user model for the kernel:
//! - [`KernelUser`] — a user record (like `/etc/passwd`)
//! - [`Permission`] — fine-grained capabilities
//! - [`PlatformIdentity`] — external platform identity bindings
//! - [`UserStore`] — persistence trait for user CRUD + platform lookups

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::principal::Role;
use crate::error::Result;

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
    UseTool(String),
    /// Can manage skills.
    ManageSkills,
    /// Can manage MCP servers.
    ManageMcp,
}

/// Kernel user — analogous to a record in `/etc/passwd`.
///
/// Each user has a unique name, a role, and a set of fine-grained permissions.
/// The `name` field is the primary lookup key used by [`Principal::from_user`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KernelUser {
    pub id: uuid::Uuid,
    pub name: String,
    pub role: Role,
    pub permissions: Vec<Permission>,
    pub enabled: bool,
    pub created_at: jiff::Timestamp,
    pub updated_at: jiff::Timestamp,
}

/// Platform identity binding — one kernel user can have multiple platform
/// identities (e.g. Telegram, Web, CLI).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformIdentity {
    pub id: uuid::Uuid,
    pub user_id: uuid::Uuid,
    /// Platform name: "telegram", "web", "cli", etc.
    pub platform: String,
    /// Platform-specific user identifier.
    pub platform_user_id: String,
    /// Human-readable display name, if available.
    pub display_name: Option<String>,
    pub linked_at: jiff::Timestamp,
}

/// Well-known user name for the root superuser.
pub const ROOT_USER_NAME: &str = "root";
/// Well-known user name for the system service account.
pub const SYSTEM_USER_NAME: &str = "system";

impl KernelUser {
    /// Create the root superuser — `Role::Root` + `Permission::All`.
    pub fn root() -> Self {
        Self {
            id: uuid::Uuid::new_v4(),
            name: ROOT_USER_NAME.to_string(),
            role: Role::Root,
            permissions: vec![Permission::All],
            enabled: true,
            created_at: jiff::Timestamp::now(),
            updated_at: jiff::Timestamp::now(),
        }
    }

    /// Create the system service account — `Role::Admin` + `Permission::All`.
    ///
    /// Used by background workers via `Principal::admin("system")`.
    pub fn system() -> Self {
        Self {
            id: uuid::Uuid::new_v4(),
            name: SYSTEM_USER_NAME.to_string(),
            role: Role::Admin,
            permissions: vec![Permission::All],
            enabled: true,
            created_at: jiff::Timestamp::now(),
            updated_at: jiff::Timestamp::now(),
        }
    }

    /// Check whether this user has the given permission.
    ///
    /// Users with `Permission::All` automatically have every permission.
    pub fn has_permission(&self, perm: &Permission) -> bool {
        self.permissions.contains(&Permission::All) || self.permissions.contains(perm)
    }

    /// Check whether this user can use the named tool.
    pub fn can_use_tool(&self, tool_name: &str) -> bool {
        self.has_permission(&Permission::All)
            || self.has_permission(&Permission::UseAllTools)
            || self.has_permission(&Permission::UseTool(tool_name.to_string()))
    }
}

/// Persistence trait for kernel user management.
#[async_trait]
pub trait UserStore: Send + Sync {
    async fn get_by_id(&self, id: uuid::Uuid) -> Result<Option<KernelUser>>;
    async fn get_by_name(&self, name: &str) -> Result<Option<KernelUser>>;
    async fn get_by_platform(
        &self,
        platform: &str,
        platform_user_id: &str,
    ) -> Result<Option<KernelUser>>;
    async fn create(&self, user: &KernelUser) -> Result<()>;
    async fn update(&self, user: &KernelUser) -> Result<()>;
    async fn delete(&self, id: uuid::Uuid) -> Result<()>;
    async fn list(&self) -> Result<Vec<KernelUser>>;
    async fn link_platform(&self, identity: &PlatformIdentity) -> Result<()>;
    async fn unlink_platform(&self, id: uuid::Uuid) -> Result<()>;
    async fn list_platforms(&self, user_id: uuid::Uuid) -> Result<Vec<PlatformIdentity>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_has_all_permissions() {
        let root = KernelUser::root();
        assert_eq!(root.name, "root");
        assert_eq!(root.role, Role::Root);
        assert!(root.enabled);
        assert!(root.has_permission(&Permission::All));
        assert!(root.has_permission(&Permission::Spawn));
        assert!(root.has_permission(&Permission::ManageUsers));
        assert!(root.has_permission(&Permission::UseAllTools));
        assert!(root.has_permission(&Permission::ManageSkills));
        assert!(root.has_permission(&Permission::ManageMcp));
    }

    #[test]
    fn system_has_all_permissions() {
        let system = KernelUser::system();
        assert_eq!(system.name, "system");
        assert_eq!(system.role, Role::Admin);
        assert!(system.enabled);
        assert!(system.has_permission(&Permission::All));
        assert!(system.has_permission(&Permission::Spawn));
    }

    #[test]
    fn regular_user_only_has_granted_permissions() {
        let user = KernelUser {
            id: uuid::Uuid::new_v4(),
            name: "alice".to_string(),
            role: Role::User,
            permissions: vec![Permission::Spawn],
            enabled: true,
            created_at: jiff::Timestamp::now(),
            updated_at: jiff::Timestamp::now(),
        };
        assert!(user.has_permission(&Permission::Spawn));
        assert!(!user.has_permission(&Permission::ManageUsers));
        assert!(!user.has_permission(&Permission::All));
    }

    #[test]
    fn can_use_tool_with_all() {
        let root = KernelUser::root();
        assert!(root.can_use_tool("bash"));
        assert!(root.can_use_tool("anything"));
    }

    #[test]
    fn can_use_tool_with_use_all_tools() {
        let user = KernelUser {
            id: uuid::Uuid::new_v4(),
            name: "bob".to_string(),
            role: Role::User,
            permissions: vec![Permission::UseAllTools],
            enabled: true,
            created_at: jiff::Timestamp::now(),
            updated_at: jiff::Timestamp::now(),
        };
        assert!(user.can_use_tool("bash"));
        assert!(user.can_use_tool("read_file"));
    }

    #[test]
    fn can_use_tool_with_specific_tool() {
        let user = KernelUser {
            id: uuid::Uuid::new_v4(),
            name: "carol".to_string(),
            role: Role::User,
            permissions: vec![Permission::UseTool("bash".to_string())],
            enabled: true,
            created_at: jiff::Timestamp::now(),
            updated_at: jiff::Timestamp::now(),
        };
        assert!(user.can_use_tool("bash"));
        assert!(!user.can_use_tool("read_file"));
    }

    #[test]
    fn permission_serde_roundtrip() {
        let perms = vec![
            Permission::All,
            Permission::Spawn,
            Permission::UseTool("bash".to_string()),
        ];
        let json = serde_json::to_string(&perms).unwrap();
        let deserialized: Vec<Permission> = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, perms);
    }

    #[test]
    fn kernel_user_serde_roundtrip() {
        let user = KernelUser::root();
        let json = serde_json::to_string(&user).unwrap();
        let deserialized: KernelUser = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.name, "root");
        assert_eq!(deserialized.role, Role::Root);
        assert_eq!(deserialized.permissions, vec![Permission::All]);
    }
}
