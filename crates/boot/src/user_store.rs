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

//! Config-driven [`UserStore`] implementation and platform identity mapping.

use std::collections::HashMap;

use async_trait::async_trait;
use rara_kernel::{
    error::Result,
    identity::{KernelUser, Permission, Role, UserStore},
};
use serde::Deserialize;

// -- Config types (defined here because app → boot dependency direction) ------

/// A user entry in the YAML configuration file.
#[derive(Debug, Clone, Deserialize)]
pub struct UserConfig {
    pub name:      String,
    /// `"root"` | `"admin"` | `"user"`
    pub role:      String,
    #[serde(default)]
    pub platforms: Vec<PlatformBindingConfig>,
}

/// A platform identity binding for a configured user.
#[derive(Debug, Clone, Deserialize)]
pub struct PlatformBindingConfig {
    /// Channel type: `"telegram"`, `"web"`, `"cli"`, etc.
    #[serde(rename = "type")]
    pub channel_type: String,
    /// Platform-side user identifier (e.g. Telegram user ID).
    pub user_id:      String,
}

// -- Helpers -----------------------------------------------------------------

fn parse_role(s: &str) -> Role {
    match s {
        "root" => Role::Root,
        "admin" => Role::Admin,
        _ => Role::User,
    }
}

fn default_permissions(role: Role) -> Vec<Permission> {
    match role {
        Role::Root | Role::Admin => vec![Permission::All],
        Role::User => vec![Permission::Spawn],
    }
}

// -- InMemoryUserStore -------------------------------------------------------

/// In-memory user store built from YAML config at startup.
pub struct InMemoryUserStore {
    by_name: HashMap<String, KernelUser>,
}

impl InMemoryUserStore {
    /// Build from config entries.
    pub fn from_config(users: &[UserConfig]) -> Self {
        let mut by_name = HashMap::with_capacity(users.len());

        for cfg in users {
            let role = parse_role(&cfg.role);
            let user = KernelUser {
                name: cfg.name.clone(),
                role,
                permissions: default_permissions(role),
                enabled: true,
            };
            by_name.insert(user.name.clone(), user);
        }

        Self { by_name }
    }
}

#[async_trait]
impl UserStore for InMemoryUserStore {
    async fn get_by_name(&self, name: &str) -> Result<Option<KernelUser>> {
        Ok(self.by_name.get(name).cloned())
    }

    async fn list(&self) -> Result<Vec<KernelUser>> { Ok(self.by_name.values().cloned().collect()) }
}
