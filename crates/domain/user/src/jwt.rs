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

//! JWT token encoding/decoding utilities.

use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

use crate::{error::AuthError, types::UserInfo};

/// JWT 配置
#[derive(Debug, Clone)]
pub struct JwtConfig {
    pub secret:                    String,
    pub access_token_expiry_secs:  u64,
    pub refresh_token_expiry_secs: u64,
}

impl JwtConfig {
    /// 使用给定的 secret 创建默认配置
    /// access token 有效期 1 小时，refresh token 有效期 7 天
    pub fn new(secret: String) -> Self {
        Self {
            secret,
            access_token_expiry_secs: 3600,     // 1h
            refresh_token_expiry_secs: 604_800, // 7d
        }
    }
}

/// JWT Claims — 令牌中携带的声明
#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    /// subject — 用户 ID (UUID 字符串)
    pub sub:        String,
    /// 用户名
    pub name:       String,
    /// 用户角色
    pub role:       String,
    /// 过期时间 (Unix timestamp)
    pub exp:        u64,
    /// 签发时间 (Unix timestamp)
    pub iat:        u64,
    /// 令牌类型: "access" | "refresh"
    pub token_type: String,
}

/// 生成 access token
pub fn encode_access_token(config: &JwtConfig, user: &UserInfo) -> Result<String, AuthError> {
    let now = chrono::Utc::now().timestamp() as u64;
    let claims = Claims {
        sub:        user.id.to_string(),
        name:       user.name.clone(),
        role:       user.role.clone(),
        exp:        now + config.access_token_expiry_secs,
        iat:        now,
        token_type: "access".to_string(),
    };
    jsonwebtoken::encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(config.secret.as_bytes()),
    )
    .map_err(|e| AuthError::TokenInvalid { source: e })
}

/// 生成 refresh token
pub fn encode_refresh_token(config: &JwtConfig, user: &UserInfo) -> Result<String, AuthError> {
    let now = chrono::Utc::now().timestamp() as u64;
    let claims = Claims {
        sub:        user.id.to_string(),
        name:       user.name.clone(),
        role:       user.role.clone(),
        exp:        now + config.refresh_token_expiry_secs,
        iat:        now,
        token_type: "refresh".to_string(),
    };
    jsonwebtoken::encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(config.secret.as_bytes()),
    )
    .map_err(|e| AuthError::TokenInvalid { source: e })
}

/// 解码并验证 JWT token
pub fn decode_token(config: &JwtConfig, token: &str) -> Result<Claims, AuthError> {
    let mut validation = Validation::new(Algorithm::HS256);
    validation.validate_exp = true;

    jsonwebtoken::decode::<Claims>(
        token,
        &DecodingKey::from_secret(config.secret.as_bytes()),
        &validation,
    )
    .map(|data| data.claims)
    .map_err(|e| {
        if matches!(e.kind(), jsonwebtoken::errors::ErrorKind::ExpiredSignature) {
            AuthError::TokenExpired
        } else {
            AuthError::TokenInvalid { source: e }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> JwtConfig { JwtConfig::new("test-secret-key-for-jwt-testing".to_string()) }

    fn test_user() -> UserInfo {
        UserInfo {
            id:      uuid::Uuid::new_v4(),
            name:    "testuser".to_string(),
            role:    "User".to_string(),
            enabled: true,
        }
    }

    #[test]
    fn encode_decode_access_token() {
        let config = test_config();
        let user = test_user();
        let token = encode_access_token(&config, &user).unwrap();
        let claims = decode_token(&config, &token).unwrap();
        assert_eq!(claims.sub, user.id.to_string());
        assert_eq!(claims.name, "testuser");
        assert_eq!(claims.role, "User");
        assert_eq!(claims.token_type, "access");
    }

    #[test]
    fn encode_decode_refresh_token() {
        let config = test_config();
        let user = test_user();
        let token = encode_refresh_token(&config, &user).unwrap();
        let claims = decode_token(&config, &token).unwrap();
        assert_eq!(claims.sub, user.id.to_string());
        assert_eq!(claims.token_type, "refresh");
    }

    #[test]
    fn decode_with_wrong_secret_fails() {
        let config = test_config();
        let user = test_user();
        let token = encode_access_token(&config, &user).unwrap();

        let bad_config = JwtConfig::new("wrong-secret".to_string());
        let result = decode_token(&bad_config, &token);
        assert!(result.is_err());
    }

    #[test]
    fn expired_token_returns_token_expired() {
        // Manually create a token with exp in the past
        let config = test_config();
        let now = chrono::Utc::now().timestamp() as u64;
        let claims = Claims {
            sub:        uuid::Uuid::new_v4().to_string(),
            name:       "test".to_string(),
            role:       "User".to_string(),
            exp:        now - 120, // 2 minutes in the past (past default 60s leeway)
            iat:        now - 180,
            token_type: "access".to_string(),
        };
        let token = jsonwebtoken::encode(
            &jsonwebtoken::Header::new(Algorithm::HS256),
            &claims,
            &jsonwebtoken::EncodingKey::from_secret(config.secret.as_bytes()),
        )
        .unwrap();

        let result = decode_token(&config, &token);
        match &result {
            Err(AuthError::TokenExpired) => {}
            other => panic!("expected TokenExpired, got: {other:?}"),
        }
    }
}
