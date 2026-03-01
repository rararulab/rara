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

//! HTTP API routes for authentication and user management.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use tracing::instrument;
use uuid::Uuid;

use crate::error::AuthError;
use crate::middleware::{AuthUser, RequireRoot};
use crate::service::AuthService;
use crate::types::*;

/// 构建认证和用户管理路由（plain axum::Router）
pub fn auth_routes(service: AuthService) -> axum::Router {
    axum::Router::new()
        // 公开路由 — 不需要认证
        .route("/api/v1/auth/login", axum::routing::post(login))
        .route("/api/v1/auth/register", axum::routing::post(register))
        .route("/api/v1/auth/refresh", axum::routing::post(refresh))
        // 需要认证的路由
        .route("/api/v1/users/me", axum::routing::get(get_profile))
        .route("/api/v1/users/me/password", axum::routing::put(change_password))
        .route("/api/v1/users/me/link-code", axum::routing::post(generate_link_code))
        // 管理路由 — 需要 Root 角色
        .route("/api/v1/admin/users", axum::routing::get(list_users))
        .route("/api/v1/admin/invite-codes", axum::routing::post(create_invite_code).get(list_invite_codes))
        .route("/api/v1/admin/users/{id}", axum::routing::delete(disable_user))
        .with_state(service)
}

/// 用户登录
#[instrument(skip(service, req))]
async fn login(
    State(service): State<AuthService>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<AuthResponse>, AuthError> {
    let response = service.login(req).await?;
    Ok(Json(response))
}

/// 用户注册
#[instrument(skip(service, req))]
async fn register(
    State(service): State<AuthService>,
    Json(req): Json<RegisterRequest>,
) -> Result<(StatusCode, Json<AuthResponse>), AuthError> {
    let response = service.register(req).await?;
    Ok((StatusCode::CREATED, Json(response)))
}

/// 刷新令牌
#[instrument(skip(service, req))]
async fn refresh(
    State(service): State<AuthService>,
    Json(req): Json<RefreshRequest>,
) -> Result<Json<AuthResponse>, AuthError> {
    let response = service.refresh(req).await?;
    Ok(Json(response))
}

/// 获取当前用户档案
#[instrument(skip(service))]
async fn get_profile(
    State(service): State<AuthService>,
    user: AuthUser,
) -> Result<Json<UserProfile>, AuthError> {
    let profile = service.get_profile(user.user_id).await?;
    Ok(Json(profile))
}

/// 修改密码
#[instrument(skip(service, req))]
async fn change_password(
    State(service): State<AuthService>,
    user: AuthUser,
    Json(req): Json<ChangePasswordRequest>,
) -> Result<StatusCode, AuthError> {
    service.change_password(user.user_id, req).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// 生成平台链接码
#[instrument(skip(service))]
async fn generate_link_code(
    State(service): State<AuthService>,
    user: AuthUser,
    Json(req): Json<GenerateLinkCodeRequest>,
) -> Result<Json<LinkCode>, AuthError> {
    let link = service.generate_link_code(user.user_id, &req.direction).await?;
    Ok(Json(link))
}

/// 列出所有用户（管理接口）
#[instrument(skip(service))]
async fn list_users(
    State(service): State<AuthService>,
    _root: RequireRoot,
) -> Result<Json<Vec<UserInfo>>, AuthError> {
    let users = service.list_users().await?;
    Ok(Json(users))
}

/// 生成邀请码
#[instrument(skip(service))]
async fn create_invite_code(
    State(service): State<AuthService>,
    root: RequireRoot,
) -> Result<(StatusCode, Json<InviteCode>), AuthError> {
    let invite = service.generate_invite_code(root.0.user_id).await?;
    Ok((StatusCode::CREATED, Json(invite)))
}

/// 列出所有邀请码
#[instrument(skip(service))]
async fn list_invite_codes(
    State(service): State<AuthService>,
    _root: RequireRoot,
) -> Result<Json<Vec<InviteCode>>, AuthError> {
    let codes = service.list_invite_codes().await?;
    Ok(Json(codes))
}

/// 禁用用户
#[instrument(skip(service))]
async fn disable_user(
    State(service): State<AuthService>,
    _root: RequireRoot,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AuthError> {
    service.disable_user(id).await?;
    Ok(StatusCode::NO_CONTENT)
}
