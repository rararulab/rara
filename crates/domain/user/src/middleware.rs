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

//! axum middleware extractors for JWT-based authentication.

use axum::{extract::FromRequestParts, http::request::Parts};

use crate::{
    error::AuthError,
    jwt::{JwtConfig, decode_token},
};

/// 已认证用户 — 从 JWT token 中提取
#[derive(Debug, Clone)]
pub struct AuthUser {
    pub user_id: uuid::Uuid,
    pub name:    String,
    pub role:    String,
}

impl<S: Send + Sync> FromRequestParts<S> for AuthUser {
    type Rejection = AuthError;

    fn from_request_parts(
        parts: &mut Parts,
        _state: &S,
    ) -> impl std::future::Future<Output = Result<Self, Self::Rejection>> + Send {
        async move {
            // 从 Extension 中获取 JwtConfig
            let jwt_config = parts
                .extensions
                .get::<JwtConfig>()
                .ok_or(AuthError::InternalError {
                    message: "JWT config not found in state".to_string(),
                })?
                .clone();

            // 尝试从 Authorization header 提取 token
            let token = extract_token_from_header(parts)
                .or_else(|| extract_token_from_query(parts))
                .ok_or(AuthError::InvalidCredentials)?;

            // 解码并验证 token
            let claims = decode_token(&jwt_config, &token)?;

            // 只接受 access token
            if claims.token_type != "access" {
                return Err(AuthError::InvalidCredentials);
            }

            let user_id =
                claims
                    .sub
                    .parse::<uuid::Uuid>()
                    .map_err(|_| AuthError::InternalError {
                        message: "invalid user id in token".to_string(),
                    })?;

            Ok(AuthUser {
                user_id,
                name: claims.name,
                role: claims.role,
            })
        }
    }
}

/// 要求 Root 角色的 extractor
#[derive(Debug, Clone)]
pub struct RequireRoot(pub AuthUser);

impl<S: Send + Sync> FromRequestParts<S> for RequireRoot {
    type Rejection = AuthError;

    fn from_request_parts(
        parts: &mut Parts,
        state: &S,
    ) -> impl std::future::Future<Output = Result<Self, Self::Rejection>> + Send {
        async move {
            let user = AuthUser::from_request_parts(parts, state).await?;
            if user.role != "Root" {
                return Err(AuthError::InvalidCredentials);
            }
            Ok(RequireRoot(user))
        }
    }
}

/// 从 Authorization: Bearer <token> header 提取 token
fn extract_token_from_header(parts: &Parts) -> Option<String> {
    let auth_header = parts.headers.get("authorization")?.to_str().ok()?;
    let token = auth_header.strip_prefix("Bearer ")?;
    Some(token.to_string())
}

/// 从 query parameter ?token=<jwt> 提取 token（用于 WebSocket）
fn extract_token_from_query(parts: &Parts) -> Option<String> {
    let query = parts.uri.query()?;
    for pair in query.split('&') {
        if let Some(value) = pair.strip_prefix("token=") {
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}
