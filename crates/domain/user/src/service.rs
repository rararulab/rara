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

//! Authentication and user management service.

use argon2::{
    Argon2, PasswordHash, PasswordHasher, PasswordVerifier,
    password_hash::{SaltString, rand_core::OsRng},
};
use rand::Rng;
use rand::distr::Alphanumeric;
use sqlx::PgPool;
use tracing::info;

use crate::error::AuthError;
use crate::jwt::{JwtConfig, encode_access_token, encode_refresh_token, decode_token};
use crate::types::*;

// -- DB row types (chrono at DB boundary) ------------------------------------

#[derive(sqlx::FromRow)]
struct UserRow {
    id:            uuid::Uuid,
    name:          String,
    role:          i16,
    #[allow(dead_code)]
    permissions:   serde_json::Value,
    enabled:       bool,
    password_hash: Option<String>,
    #[allow(dead_code)]
    created_at:    chrono::DateTime<chrono::Utc>,
    #[allow(dead_code)]
    updated_at:    chrono::DateTime<chrono::Utc>,
}

#[derive(sqlx::FromRow)]
struct InviteCodeRow {
    id:         uuid::Uuid,
    code:       String,
    created_by: uuid::Uuid,
    used_by:    Option<uuid::Uuid>,
    expires_at: chrono::DateTime<chrono::Utc>,
    created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(sqlx::FromRow)]
struct LinkCodeRow {
    id:            uuid::Uuid,
    code:          String,
    user_id:       uuid::Uuid,
    direction:     String,
    platform_data: Option<serde_json::Value>,
    expires_at:    chrono::DateTime<chrono::Utc>,
    created_at:    chrono::DateTime<chrono::Utc>,
}

#[derive(sqlx::FromRow)]
struct PlatformRow {
    #[allow(dead_code)]
    id:               uuid::Uuid,
    #[allow(dead_code)]
    user_id:          uuid::Uuid,
    platform:         String,
    platform_user_id: String,
    display_name:     Option<String>,
    linked_at:        chrono::DateTime<chrono::Utc>,
}

fn role_name(role: i16) -> String {
    match role {
        0 => "Root".to_string(),
        1 => "Admin".to_string(),
        _ => "User".to_string(),
    }
}

fn row_to_user_info(row: &UserRow) -> UserInfo {
    UserInfo {
        id:      row.id,
        name:    row.name.clone(),
        role:    role_name(row.role),
        enabled: row.enabled,
    }
}

fn row_to_invite_code(row: InviteCodeRow) -> InviteCode {
    InviteCode {
        id:         row.id,
        code:       row.code,
        created_by: row.created_by,
        used_by:    row.used_by,
        expires_at: row.expires_at.to_rfc3339(),
        created_at: row.created_at.to_rfc3339(),
    }
}

fn row_to_link_code(row: LinkCodeRow) -> LinkCode {
    LinkCode {
        id:         row.id,
        code:       row.code,
        user_id:    row.user_id,
        direction:  row.direction,
        expires_at: row.expires_at.to_rfc3339(),
        created_at: row.created_at.to_rfc3339(),
    }
}

/// 认证服务
#[derive(Clone)]
pub struct AuthService {
    pool:       PgPool,
    jwt_config: JwtConfig,
}

impl AuthService {
    pub fn new(pool: PgPool, jwt_config: JwtConfig) -> Self {
        Self { pool, jwt_config }
    }

    pub fn jwt_config(&self) -> &JwtConfig { &self.jwt_config }

    /// 用户登录 — 验证 argon2 密码哈希，签发 token
    pub async fn login(&self, req: LoginRequest) -> Result<AuthResponse, AuthError> {
        let row = sqlx::query_as::<_, UserRow>(
            "SELECT id, name, role, permissions, enabled, password_hash, created_at, updated_at \
             FROM kernel_users WHERE name = $1",
        )
        .bind(&req.username)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AuthError::InternalError { message: e.to_string() })?
        .ok_or(AuthError::InvalidCredentials)?;

        if !row.enabled {
            return Err(AuthError::UserDisabled { username: req.username });
        }

        let hash = row.password_hash.as_deref().ok_or(AuthError::InvalidCredentials)?;
        verify_password(&req.password, hash)?;

        let user_info = row_to_user_info(&row);
        let access_token = encode_access_token(&self.jwt_config, &user_info)?;
        let refresh_token = encode_refresh_token(&self.jwt_config, &user_info)?;

        info!(username = %req.username, "user logged in");

        Ok(AuthResponse {
            access_token,
            refresh_token,
            user: user_info,
        })
    }

    /// 用户注册 — 验证邀请码、创建用户、签发 token
    pub async fn register(&self, req: RegisterRequest) -> Result<AuthResponse, AuthError> {
        // 检查用户名是否已存在
        let exists = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM kernel_users WHERE name = $1)",
        )
        .bind(&req.username)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AuthError::InternalError { message: e.to_string() })?;

        if exists {
            return Err(AuthError::UsernameAlreadyExists { username: req.username });
        }

        // 验证邀请码
        let invite = sqlx::query_as::<_, InviteCodeRow>(
            "SELECT * FROM invite_codes WHERE code = $1",
        )
        .bind(&req.invite_code)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AuthError::InternalError { message: e.to_string() })?
        .ok_or(AuthError::InviteCodeInvalid)?;

        if invite.used_by.is_some() {
            return Err(AuthError::InviteCodeInvalid);
        }
        if invite.expires_at < chrono::Utc::now() {
            return Err(AuthError::InviteCodeExpired);
        }

        // 创建用户
        let user_id = uuid::Uuid::new_v4();
        let password_hash = hash_password(&req.password)?;

        sqlx::query(
            "INSERT INTO kernel_users (id, name, role, permissions, enabled, password_hash) \
             VALUES ($1, $2, 2, '[]', true, $3)",
        )
        .bind(user_id)
        .bind(&req.username)
        .bind(&password_hash)
        .execute(&self.pool)
        .await
        .map_err(|e| AuthError::InternalError { message: e.to_string() })?;

        // 标记邀请码已使用
        sqlx::query("UPDATE invite_codes SET used_by = $1 WHERE id = $2")
            .bind(user_id)
            .bind(invite.id)
            .execute(&self.pool)
            .await
            .map_err(|e| AuthError::InternalError { message: e.to_string() })?;

        let user_info = UserInfo {
            id:      user_id,
            name:    req.username.clone(),
            role:    "User".to_string(),
            enabled: true,
        };

        let access_token = encode_access_token(&self.jwt_config, &user_info)?;
        let refresh_token = encode_refresh_token(&self.jwt_config, &user_info)?;

        info!(username = %req.username, "user registered");

        Ok(AuthResponse {
            access_token,
            refresh_token,
            user: user_info,
        })
    }

    /// 刷新令牌 — 验证 refresh token，签发新 access token
    pub async fn refresh(&self, req: RefreshRequest) -> Result<AuthResponse, AuthError> {
        let claims = decode_token(&self.jwt_config, &req.refresh_token)?;

        if claims.token_type != "refresh" {
            return Err(AuthError::InvalidCredentials);
        }

        let user_id: uuid::Uuid = claims.sub.parse().map_err(|_| AuthError::InternalError {
            message: "invalid user id in token".to_string(),
        })?;

        // 查询用户最新信息
        let row = sqlx::query_as::<_, UserRow>(
            "SELECT id, name, role, permissions, enabled, password_hash, created_at, updated_at \
             FROM kernel_users WHERE id = $1",
        )
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AuthError::InternalError { message: e.to_string() })?
        .ok_or(AuthError::UserNotFound { username: claims.name.clone() })?;

        if !row.enabled {
            return Err(AuthError::UserDisabled { username: row.name });
        }

        let user_info = row_to_user_info(&row);
        let access_token = encode_access_token(&self.jwt_config, &user_info)?;
        let refresh_token = encode_refresh_token(&self.jwt_config, &user_info)?;

        Ok(AuthResponse {
            access_token,
            refresh_token,
            user: user_info,
        })
    }

    /// 修改密码
    pub async fn change_password(
        &self,
        user_id: uuid::Uuid,
        req: ChangePasswordRequest,
    ) -> Result<(), AuthError> {
        let row = sqlx::query_as::<_, UserRow>(
            "SELECT id, name, role, permissions, enabled, password_hash, created_at, updated_at \
             FROM kernel_users WHERE id = $1",
        )
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AuthError::InternalError { message: e.to_string() })?
        .ok_or(AuthError::InternalError { message: "user not found".to_string() })?;

        let hash = row.password_hash.as_deref().ok_or(AuthError::InvalidCredentials)?;
        verify_password(&req.old_password, hash)?;

        let new_hash = hash_password(&req.new_password)?;
        sqlx::query("UPDATE kernel_users SET password_hash = $1, updated_at = now() WHERE id = $2")
            .bind(&new_hash)
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(|e| AuthError::InternalError { message: e.to_string() })?;

        info!(user_id = %user_id, "password changed");
        Ok(())
    }

    /// 获取用户档案（含平台绑定信息）
    pub async fn get_profile(&self, user_id: uuid::Uuid) -> Result<UserProfile, AuthError> {
        let row = sqlx::query_as::<_, UserRow>(
            "SELECT id, name, role, permissions, enabled, password_hash, created_at, updated_at \
             FROM kernel_users WHERE id = $1",
        )
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AuthError::InternalError { message: e.to_string() })?
        .ok_or(AuthError::InternalError { message: "user not found".to_string() })?;

        let platform_rows = sqlx::query_as::<_, PlatformRow>(
            "SELECT * FROM user_platform_identities WHERE user_id = $1 ORDER BY linked_at",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AuthError::InternalError { message: e.to_string() })?;

        let platforms = platform_rows
            .into_iter()
            .map(|p| PlatformInfo {
                platform:         p.platform,
                platform_user_id: p.platform_user_id,
                display_name:     p.display_name,
                linked_at:        p.linked_at.to_rfc3339(),
            })
            .collect();

        Ok(UserProfile {
            user: row_to_user_info(&row),
            platforms,
        })
    }

    /// 生成邀请码（8 字符字母数字，7 天有效期）
    pub async fn generate_invite_code(
        &self,
        created_by: uuid::Uuid,
    ) -> Result<InviteCode, AuthError> {
        let code = generate_random_code(8);
        let expires_at = chrono::Utc::now() + chrono::Duration::days(7);

        let row = sqlx::query_as::<_, InviteCodeRow>(
            "INSERT INTO invite_codes (code, created_by, expires_at) \
             VALUES ($1, $2, $3) RETURNING *",
        )
        .bind(&code)
        .bind(created_by)
        .bind(expires_at)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AuthError::InternalError { message: e.to_string() })?;

        info!(code = %code, "invite code generated");
        Ok(row_to_invite_code(row))
    }

    /// 列出所有邀请码
    pub async fn list_invite_codes(&self) -> Result<Vec<InviteCode>, AuthError> {
        let rows = sqlx::query_as::<_, InviteCodeRow>(
            "SELECT * FROM invite_codes ORDER BY created_at DESC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AuthError::InternalError { message: e.to_string() })?;

        Ok(rows.into_iter().map(row_to_invite_code).collect())
    }

    /// 生成链接码（6 字符，5 分钟有效期）
    pub async fn generate_link_code(
        &self,
        user_id: uuid::Uuid,
        direction: &str,
    ) -> Result<LinkCode, AuthError> {
        let code = generate_random_code(6);
        let expires_at = chrono::Utc::now() + chrono::Duration::minutes(5);

        let row = sqlx::query_as::<_, LinkCodeRow>(
            "INSERT INTO link_codes (code, user_id, direction, expires_at) \
             VALUES ($1, $2, $3, $4) RETURNING *",
        )
        .bind(&code)
        .bind(user_id)
        .bind(direction)
        .bind(expires_at)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AuthError::InternalError { message: e.to_string() })?;

        info!(code = %code, direction = %direction, "link code generated");
        Ok(row_to_link_code(row))
    }

    /// 验证链接码
    pub async fn verify_link_code(&self, code: &str) -> Result<LinkCodeInfo, AuthError> {
        let row = sqlx::query_as::<_, LinkCodeRow>(
            "SELECT * FROM link_codes WHERE code = $1",
        )
        .bind(code)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AuthError::InternalError { message: e.to_string() })?
        .ok_or(AuthError::LinkCodeInvalid)?;

        if row.expires_at < chrono::Utc::now() {
            return Err(AuthError::LinkCodeInvalid);
        }

        Ok(LinkCodeInfo {
            user_id:       row.user_id,
            direction:     row.direction,
            platform_data: row.platform_data,
        })
    }

    /// 列出所有用户（管理接口）
    pub async fn list_users(&self) -> Result<Vec<UserInfo>, AuthError> {
        let rows = sqlx::query_as::<_, UserRow>(
            "SELECT id, name, role, permissions, enabled, password_hash, created_at, updated_at \
             FROM kernel_users ORDER BY created_at",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AuthError::InternalError { message: e.to_string() })?;

        Ok(rows.iter().map(row_to_user_info).collect())
    }

    /// 禁用用户
    pub async fn disable_user(&self, user_id: uuid::Uuid) -> Result<(), AuthError> {
        sqlx::query("UPDATE kernel_users SET enabled = false, updated_at = now() WHERE id = $1")
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(|e| AuthError::InternalError { message: e.to_string() })?;

        info!(user_id = %user_id, "user disabled");
        Ok(())
    }
}

// -- Password hashing helpers ------------------------------------------------

/// 使用 argon2 哈希密码
pub fn hash_password(password: &str) -> Result<String, AuthError> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    argon2
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| AuthError::InternalError {
            message: format!("password hash error: {e}"),
        })
}

/// 验证密码与哈希是否匹配
fn verify_password(password: &str, hash: &str) -> Result<(), AuthError> {
    let parsed = PasswordHash::new(hash).map_err(|e| AuthError::InternalError {
        message: format!("password hash parse error: {e}"),
    })?;
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .map_err(|_| AuthError::InvalidCredentials)
}

/// 生成指定长度的随机字母数字码
fn generate_random_code(len: usize) -> String {
    rand::rng()
        .sample_iter(&Alphanumeric)
        .take(len)
        .map(char::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_and_verify_password() {
        let password = "test-password-123";
        let hash = hash_password(password).unwrap();
        assert!(verify_password(password, &hash).is_ok());
        assert!(verify_password("wrong-password", &hash).is_err());
    }

    #[test]
    fn generate_code_correct_length() {
        let code8 = generate_random_code(8);
        assert_eq!(code8.len(), 8);
        assert!(code8.chars().all(|c| c.is_ascii_alphanumeric()));

        let code6 = generate_random_code(6);
        assert_eq!(code6.len(), 6);
        assert!(code6.chars().all(|c| c.is_ascii_alphanumeric()));
    }
}
