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

/// User identity (stores user **name**, not UUID).
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
