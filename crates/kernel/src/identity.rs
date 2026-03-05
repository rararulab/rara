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
    UseTool(String),
    /// Can manage skills.
    ManageSkills,
    /// Can manage MCP servers.
    ManageMcp,
}

/// Kernel user — analogous to a record in `/etc/passwd`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KernelUser {
    pub id:          uuid::Uuid,
    pub name:        String,
    pub role:        Role,
    pub permissions: Vec<Permission>,
    pub enabled:     bool,
    pub created_at:  jiff::Timestamp,
    pub updated_at:  jiff::Timestamp,
}

/// Well-known user name for the root superuser.
pub const ROOT_USER_NAME: &str = "root";
/// Well-known user name for the system service account.
pub const SYSTEM_USER_NAME: &str = "system";

impl KernelUser {
    /// Create the root superuser — `Role::Root` + `Permission::All`.
    pub fn root() -> Self {
        Self {
            id:          uuid::Uuid::new_v4(),
            name:        ROOT_USER_NAME.to_string(),
            role:        Role::Root,
            permissions: vec![Permission::All],
            enabled:     true,
            created_at:  jiff::Timestamp::now(),
            updated_at:  jiff::Timestamp::now(),
        }
    }

    /// Create the system service account — `Role::Admin` + `Permission::All`.
    pub fn system() -> Self {
        Self {
            id:          uuid::Uuid::new_v4(),
            name:        SYSTEM_USER_NAME.to_string(),
            role:        Role::Admin,
            permissions: vec![Permission::All],
            enabled:     true,
            created_at:  jiff::Timestamp::now(),
            updated_at:  jiff::Timestamp::now(),
        }
    }

    /// Check whether this user has the given permission.
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
    /// [`SecuritySubsystem::resolve_principal`] to obtain a fully-populated
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

/// Persistence trait for kernel user management.
#[async_trait]
pub trait UserStore: Send + Sync {
    async fn get_by_id(&self, id: uuid::Uuid) -> Result<Option<KernelUser>>;
    async fn get_by_name(&self, name: &str) -> Result<Option<KernelUser>>;
    async fn create(&self, user: &KernelUser) -> Result<()>;
    async fn update(&self, user: &KernelUser) -> Result<()>;
    async fn delete(&self, id: uuid::Uuid) -> Result<()>;
    async fn list(&self) -> Result<Vec<KernelUser>>;
}
