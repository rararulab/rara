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

//! Principal — the identity under which an agent process runs.

use serde::{Deserialize, Serialize};

/// User identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct UserId(pub String);

impl std::fmt::Display for UserId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// User role determining permission level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    Admin,
    User,
}

/// The identity under which an agent process runs.
///
/// Every agent process inherits its parent's principal, ensuring a consistent
/// identity chain throughout the process tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Principal {
    pub user_id: UserId,
    pub role: Role,
}

impl Principal {
    /// Create an admin principal.
    pub fn admin(user_id: impl Into<String>) -> Self {
        Self {
            user_id: UserId(user_id.into()),
            role: Role::Admin,
        }
    }

    /// Create a regular user principal.
    pub fn user(user_id: impl Into<String>) -> Self {
        Self {
            user_id: UserId(user_id.into()),
            role: Role::User,
        }
    }

    /// Whether this principal has admin privileges.
    pub fn is_admin(&self) -> bool {
        self.role == Role::Admin
    }
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
    }
}
