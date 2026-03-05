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

//! SQLite-backed [`UserStore`] implementation, platform identity mapping,
//! and boot-time user initialization.

use async_trait::async_trait;
use rara_kernel::{
    error::{KernelError, Result},
    identity::{KernelUser, Permission, ROOT_USER_NAME, Role, SYSTEM_USER_NAME, UserStore},
};
use serde::Deserialize;
use sqlx::SqlitePool;
use tracing::info;

// -- Config types (defined here because app → boot dependency direction) ------

/// A user entry in the YAML configuration file.
#[derive(Debug, Clone, Deserialize)]
pub struct UserConfig {
    pub name: String,
    /// `"root"` | `"admin"` | `"user"`
    pub role: String,
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
    pub user_id: String,
}

// -- DB row types (chrono at DB boundary) ------------------------------------

#[derive(sqlx::FromRow)]
struct UserRow {
    id:          uuid::Uuid,
    name:        String,
    role:        i16,
    permissions: serde_json::Value,
    enabled:     bool,
    created_at:  chrono::DateTime<chrono::Utc>,
    updated_at:  chrono::DateTime<chrono::Utc>,
}

// -- Conversion helpers ------------------------------------------------------

fn role_to_i16(role: Role) -> i16 {
    match role {
        Role::Root => 0,
        Role::Admin => 1,
        Role::User => 2,
    }
}

fn role_from_i16(v: i16) -> Role {
    match v {
        0 => Role::Root,
        1 => Role::Admin,
        _ => Role::User,
    }
}

fn chrono_to_jiff(dt: chrono::DateTime<chrono::Utc>) -> jiff::Timestamp {
    jiff::Timestamp::from_second(dt.timestamp()).unwrap_or(jiff::Timestamp::UNIX_EPOCH)
}

fn jiff_to_chrono(ts: jiff::Timestamp) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::from_timestamp(ts.as_second(), 0).unwrap_or_default()
}

fn row_to_user(row: UserRow) -> KernelUser {
    let permissions: Vec<Permission> = serde_json::from_value(row.permissions).unwrap_or_default();
    KernelUser {
        id: row.id,
        name: row.name,
        role: role_from_i16(row.role),
        permissions,
        enabled: row.enabled,
        created_at: chrono_to_jiff(row.created_at),
        updated_at: chrono_to_jiff(row.updated_at),
    }
}

// -- SqliteUserStore ---------------------------------------------------------

/// SQLite-backed user store.
pub struct SqliteUserStore {
    pool: SqlitePool,
}

impl SqliteUserStore {
    pub fn new(pool: SqlitePool) -> Self { Self { pool } }
}

#[async_trait]
impl UserStore for SqliteUserStore {
    async fn get_by_id(&self, id: uuid::Uuid) -> Result<Option<KernelUser>> {
        let row = sqlx::query_as::<_, UserRow>("SELECT * FROM kernel_users WHERE id = ?1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| KernelError::Other {
                message: format!("user store: {e}").into(),
            })?;
        Ok(row.map(row_to_user))
    }

    async fn get_by_name(&self, name: &str) -> Result<Option<KernelUser>> {
        let row = sqlx::query_as::<_, UserRow>("SELECT * FROM kernel_users WHERE name = ?1")
            .bind(name)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| KernelError::Other {
                message: format!("user store: {e}").into(),
            })?;
        Ok(row.map(row_to_user))
    }

    async fn create(&self, user: &KernelUser) -> Result<()> {
        let perms =
            serde_json::to_value(&user.permissions).unwrap_or(serde_json::Value::Array(vec![]));
        sqlx::query(
            "INSERT INTO kernel_users (id, name, role, permissions, enabled, created_at, \
             updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )
        .bind(user.id)
        .bind(&user.name)
        .bind(role_to_i16(user.role))
        .bind(&perms)
        .bind(user.enabled)
        .bind(jiff_to_chrono(user.created_at))
        .bind(jiff_to_chrono(user.updated_at))
        .execute(&self.pool)
        .await
        .map_err(|e| KernelError::Other {
            message: format!("user store create: {e}").into(),
        })?;
        Ok(())
    }

    async fn update(&self, user: &KernelUser) -> Result<()> {
        let perms =
            serde_json::to_value(&user.permissions).unwrap_or(serde_json::Value::Array(vec![]));
        sqlx::query(
            "UPDATE kernel_users SET name = ?1, role = ?2, permissions = ?3, enabled = ?4, \
             updated_at = datetime('now') WHERE id = ?5",
        )
        .bind(&user.name)
        .bind(role_to_i16(user.role))
        .bind(&perms)
        .bind(user.enabled)
        .bind(user.id)
        .execute(&self.pool)
        .await
        .map_err(|e| KernelError::Other {
            message: format!("user store update: {e}").into(),
        })?;
        Ok(())
    }

    async fn delete(&self, id: uuid::Uuid) -> Result<()> {
        sqlx::query("DELETE FROM kernel_users WHERE id = ?1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| KernelError::Other {
                message: format!("user store delete: {e}").into(),
            })?;
        Ok(())
    }

    async fn list(&self) -> Result<Vec<KernelUser>> {
        let rows = sqlx::query_as::<_, UserRow>("SELECT * FROM kernel_users ORDER BY created_at")
            .fetch_all(&self.pool)
            .await
            .map_err(|e| KernelError::Other {
                message: format!("user store list: {e}").into(),
            })?;
        Ok(rows.into_iter().map(row_to_user).collect())
    }
}

// -- Boot-time default users -------------------------------------------------

/// Ensure `root` and `system` users exist in the database.
///
/// - `root` — `Role::Root` + `Permission::All`
/// - `system` — `Role::Admin` + `Permission::All` (used by background workers)
pub async fn ensure_default_users(
    pool: &SqlitePool,
) -> std::result::Result<(), crate::error::BootError> {
    let store = SqliteUserStore::new(pool.clone());

    if store
        .get_by_name(ROOT_USER_NAME)
        .await
        .map_err(|e| crate::error::BootError::UserStore {
            message: e.to_string(),
        })?
        .is_none()
    {
        store.create(&KernelUser::root()).await.map_err(|e| {
            crate::error::BootError::UserStore {
                message: e.to_string(),
            }
        })?;
        info!("kernel: root user created");
    }

    if store
        .get_by_name(SYSTEM_USER_NAME)
        .await
        .map_err(|e| crate::error::BootError::UserStore {
            message: e.to_string(),
        })?
        .is_none()
    {
        store.create(&KernelUser::system()).await.map_err(|e| {
            crate::error::BootError::UserStore {
                message: e.to_string(),
            }
        })?;
        info!("kernel: system user created");
    }

    Ok(())
}

// -- Boot-time configured users ----------------------------------------------

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

/// Sync users and platform identity mappings from YAML configuration.
///
/// For each [`UserConfig`]:
/// - Creates the [`KernelUser`] if it doesn't exist.
/// - Updates the role (and permissions) if the configured role changed.
/// - Upserts all platform identity bindings.
///
/// This function is idempotent and safe to call on every startup.
pub async fn ensure_configured_users(
    pool: &SqlitePool,
    users: &[UserConfig],
) -> std::result::Result<(), crate::error::BootError> {
    let store = SqliteUserStore::new(pool.clone());

    for cfg in users {
        let role = parse_role(&cfg.role);

        let boot_err =
            |e: rara_kernel::error::KernelError| crate::error::BootError::UserStore {
                message: e.to_string(),
            };

        // Ensure KernelUser exists
        match store.get_by_name(&cfg.name).await.map_err(boot_err)? {
            Some(existing) if existing.role != role => {
                let mut updated = existing;
                updated.role = role;
                updated.permissions = default_permissions(role);
                store.update(&updated).await.map_err(boot_err)?;
                info!(user = %cfg.name, ?role, "kernel user role updated from config");
            }
            Some(_) => { /* up to date */ }
            None => {
                let user = KernelUser {
                    id:          uuid::Uuid::new_v4(),
                    name:        cfg.name.clone(),
                    role,
                    permissions: default_permissions(role),
                    enabled:     true,
                    created_at:  jiff::Timestamp::now(),
                    updated_at:  jiff::Timestamp::now(),
                };
                store.create(&user).await.map_err(boot_err)?;
                info!(user = %cfg.name, ?role, "kernel user created from config");
            }
        }

    }

    Ok(())
}
