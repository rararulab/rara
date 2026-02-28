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

//! PostgreSQL-backed [`UserStore`] implementation and boot-time user
//! initialization.

use async_trait::async_trait;
use rara_kernel::{
    error::{KernelError, Result},
    process::{
        principal::Role,
        user::{
            KernelUser, Permission, PlatformIdentity, ROOT_USER_NAME, SYSTEM_USER_NAME, UserStore,
        },
    },
};
use sqlx::PgPool;
use tracing::info;

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

#[derive(sqlx::FromRow)]
struct PlatformRow {
    id:               uuid::Uuid,
    user_id:          uuid::Uuid,
    platform:         String,
    platform_user_id: String,
    display_name:     Option<String>,
    linked_at:        chrono::DateTime<chrono::Utc>,
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

fn row_to_platform(row: PlatformRow) -> PlatformIdentity {
    PlatformIdentity {
        id:               row.id,
        user_id:          row.user_id,
        platform:         row.platform,
        platform_user_id: row.platform_user_id,
        display_name:     row.display_name,
        linked_at:        chrono_to_jiff(row.linked_at),
    }
}

// -- PgUserStore -------------------------------------------------------------

/// PostgreSQL-backed user store.
pub struct PgUserStore {
    pool: PgPool,
}

impl PgUserStore {
    pub fn new(pool: PgPool) -> Self { Self { pool } }
}

#[async_trait]
impl UserStore for PgUserStore {
    async fn get_by_id(&self, id: uuid::Uuid) -> Result<Option<KernelUser>> {
        let row = sqlx::query_as::<_, UserRow>("SELECT * FROM kernel_users WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| KernelError::Other {
                message: format!("user store: {e}").into(),
            })?;
        Ok(row.map(row_to_user))
    }

    async fn get_by_name(&self, name: &str) -> Result<Option<KernelUser>> {
        let row = sqlx::query_as::<_, UserRow>("SELECT * FROM kernel_users WHERE name = $1")
            .bind(name)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| KernelError::Other {
                message: format!("user store: {e}").into(),
            })?;
        Ok(row.map(row_to_user))
    }

    async fn get_by_platform(
        &self,
        platform: &str,
        platform_user_id: &str,
    ) -> Result<Option<KernelUser>> {
        let row = sqlx::query_as::<_, UserRow>(
            "SELECT u.* FROM kernel_users u JOIN user_platform_identities p ON u.id = p.user_id \
             WHERE p.platform = $1 AND p.platform_user_id = $2",
        )
        .bind(platform)
        .bind(platform_user_id)
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
             updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7)",
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
            "UPDATE kernel_users SET name = $1, role = $2, permissions = $3, enabled = $4, \
             updated_at = now() WHERE id = $5",
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
        sqlx::query("DELETE FROM kernel_users WHERE id = $1")
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

    async fn link_platform(&self, identity: &PlatformIdentity) -> Result<()> {
        sqlx::query(
            "INSERT INTO user_platform_identities (id, user_id, platform, platform_user_id, \
             display_name, linked_at) VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(identity.id)
        .bind(identity.user_id)
        .bind(&identity.platform)
        .bind(&identity.platform_user_id)
        .bind(&identity.display_name)
        .bind(jiff_to_chrono(identity.linked_at))
        .execute(&self.pool)
        .await
        .map_err(|e| KernelError::Other {
            message: format!("user store link_platform: {e}").into(),
        })?;
        Ok(())
    }

    async fn unlink_platform(&self, id: uuid::Uuid) -> Result<()> {
        sqlx::query("DELETE FROM user_platform_identities WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| KernelError::Other {
                message: format!("user store unlink_platform: {e}").into(),
            })?;
        Ok(())
    }

    async fn list_platforms(&self, user_id: uuid::Uuid) -> Result<Vec<PlatformIdentity>> {
        let rows = sqlx::query_as::<_, PlatformRow>(
            "SELECT * FROM user_platform_identities WHERE user_id = $1 ORDER BY linked_at",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| KernelError::Other {
            message: format!("user store list_platforms: {e}").into(),
        })?;
        Ok(rows.into_iter().map(row_to_platform).collect())
    }
}

// -- Boot-time default users -------------------------------------------------

/// Ensure `root` and `system` users exist in the database.
///
/// - `root` — `Role::Root` + `Permission::All`
/// - `system` — `Role::Admin` + `Permission::All` (used by background workers
///   via `Principal::admin("system")`)
pub async fn ensure_default_users(
    pool: &PgPool,
) -> std::result::Result<(), crate::error::BootError> {
    let store = PgUserStore::new(pool.clone());

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
