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

//! Error types for the user auth domain.

use snafu::Snafu;

/// 认证/用户管理相关错误
#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum AuthError {
    /// 用户名或密码错误
    #[snafu(display("invalid credentials"))]
    InvalidCredentials,

    /// 用户不存在
    #[snafu(display("user not found: {username}"))]
    UserNotFound { username: String },

    /// 用户已被禁用
    #[snafu(display("user disabled: {username}"))]
    UserDisabled { username: String },

    /// 邀请码无效
    #[snafu(display("invite code invalid"))]
    InviteCodeInvalid,

    /// 邀请码已过期
    #[snafu(display("invite code expired"))]
    InviteCodeExpired,

    /// Token 已过期
    #[snafu(display("token expired"))]
    TokenExpired,

    /// Token 无效
    #[snafu(display("token invalid: {source}"))]
    TokenInvalid { source: jsonwebtoken::errors::Error },

    /// 用户名已存在
    #[snafu(display("username already exists: {username}"))]
    UsernameAlreadyExists { username: String },

    /// 链接码无效或已过期
    #[snafu(display("link code invalid or expired"))]
    LinkCodeInvalid,

    /// 内部错误
    #[snafu(display("internal error: {message}"))]
    InternalError { message: String },
}

impl axum::response::IntoResponse for AuthError {
    fn into_response(self) -> axum::response::Response {
        let status = match &self {
            AuthError::InvalidCredentials => axum::http::StatusCode::UNAUTHORIZED,
            AuthError::UserNotFound { .. } => axum::http::StatusCode::NOT_FOUND,
            AuthError::UserDisabled { .. } => axum::http::StatusCode::FORBIDDEN,
            AuthError::InviteCodeInvalid => axum::http::StatusCode::BAD_REQUEST,
            AuthError::InviteCodeExpired => axum::http::StatusCode::BAD_REQUEST,
            AuthError::TokenExpired => axum::http::StatusCode::UNAUTHORIZED,
            AuthError::TokenInvalid { .. } => axum::http::StatusCode::UNAUTHORIZED,
            AuthError::UsernameAlreadyExists { .. } => axum::http::StatusCode::CONFLICT,
            AuthError::LinkCodeInvalid => axum::http::StatusCode::BAD_REQUEST,
            AuthError::InternalError { .. } => axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        };
        let message = self.to_string();
        if status.is_server_error() {
            tracing::error!(http_status = status.as_u16(), error = %message, "auth request error");
        }
        let body = serde_json::json!({ "error": message });
        (status, axum::Json(body)).into_response()
    }
}
