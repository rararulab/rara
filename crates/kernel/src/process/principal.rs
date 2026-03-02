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

//! Principal — the identity under which an agent process runs.

use serde::{Deserialize, Serialize};

use super::user::{KernelUser, Permission};

/// User identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct UserId(pub String);

impl std::fmt::Display for UserId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "{}", self.0) }
}

/// User role determining permission level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    Root,
    Admin,
    User,
}

/// The identity under which an agent process runs.
///
/// Every agent process inherits its parent's principal, ensuring a consistent
/// identity chain throughout the process tree.
///
/// The `user_id` stores the **user name** (not UUID), matching
/// [`UserStore::get_by_name`](super::user::UserStore::get_by_name).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Principal {
    pub user_id:     UserId,
    pub role:        Role,
    pub permissions: Vec<Permission>,
}

impl Principal {
    /// Create a principal from a [`KernelUser`].
    ///
    /// The `UserId` is set to the user's **name** (not UUID), so that
    /// `UserStore::get_by_name()` can look it up consistently.
    pub fn from_user(user: &KernelUser) -> Self {
        Self {
            user_id:     UserId(user.name.clone()),
            role:        user.role,
            permissions: user.permissions.clone(),
        }
    }

    /// Check whether this principal has the given permission.
    ///
    /// Root role bypasses all checks. Otherwise checks `Permission::All`
    /// and the specific permission.
    pub fn has_permission(&self, perm: &Permission) -> bool {
        self.role == Role::Root
            || self.permissions.contains(&Permission::All)
            || self.permissions.contains(perm)
    }

    /// Create an admin principal (backward-compatible).
    pub fn admin(user_id: impl Into<String>) -> Self {
        Self {
            user_id:     UserId(user_id.into()),
            role:        Role::Admin,
            permissions: vec![],
        }
    }

    /// Create a regular user principal (backward-compatible).
    pub fn user(user_id: impl Into<String>) -> Self {
        Self {
            user_id:     UserId(user_id.into()),
            role:        Role::User,
            permissions: vec![],
        }
    }

    /// Whether this principal has admin privileges.
    pub fn is_admin(&self) -> bool { self.role == Role::Admin || self.role == Role::Root }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_principal_admin() {
        let p = Principal::admin("admin-1");
        assert!(p.is_admin());
        assert_eq!(p.role, Role::Admin);
        assert_eq!(p.user_id.0, "admin-1");
        assert!(p.permissions.is_empty());
    }

    #[test]
    fn test_principal_user() {
        let p = Principal::user("user-42");
        assert!(!p.is_admin());
        assert_eq!(p.role, Role::User);
        assert_eq!(p.user_id.0, "user-42");
    }

    #[test]
    fn test_user_id_display() {
        let uid = UserId("test-user".to_string());
        assert_eq!(uid.to_string(), "test-user");
    }

    #[test]
    fn test_principal_serde_roundtrip() {
        let p = Principal::admin("serde-test");
        let json = serde_json::to_string(&p).unwrap();
        let deserialized: Principal = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.user_id.0, "serde-test");
        assert_eq!(deserialized.role, Role::Admin);
        assert!(deserialized.permissions.is_empty());
    }

    #[test]
    fn test_from_user() {
        let user = KernelUser::root();
        let p = Principal::from_user(&user);
        assert_eq!(p.user_id.0, "root");
        assert_eq!(p.role, Role::Root);
        assert!(p.is_admin());
        assert!(p.has_permission(&Permission::Spawn));
        assert!(p.has_permission(&Permission::ManageUsers));
    }

    #[test]
    fn test_root_bypasses_all_checks() {
        let p = Principal {
            user_id:     UserId("root".to_string()),
            role:        Role::Root,
            permissions: vec![], // no explicit permissions, but Root bypasses
        };
        assert!(p.has_permission(&Permission::Spawn));
        assert!(p.has_permission(&Permission::ManageUsers));
        assert!(p.has_permission(&Permission::All));
    }

    #[test]
    fn test_has_permission_with_all() {
        let p = Principal {
            user_id:     UserId("system".to_string()),
            role:        Role::Admin,
            permissions: vec![Permission::All],
        };
        assert!(p.has_permission(&Permission::Spawn));
        assert!(p.has_permission(&Permission::ManageMcp));
    }

    #[test]
    fn test_has_permission_specific() {
        let p = Principal {
            user_id:     UserId("alice".to_string()),
            role:        Role::User,
            permissions: vec![Permission::Spawn],
        };
        assert!(p.has_permission(&Permission::Spawn));
        assert!(!p.has_permission(&Permission::ManageUsers));
    }
}
