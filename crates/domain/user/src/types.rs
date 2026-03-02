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

//! Request/response types for the user auth domain.

use serde::{Deserialize, Serialize};

/// 登录请求
#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

/// 注册请求
#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    pub username:    String,
    pub password:    String,
    pub invite_code: String,
}

/// 认证成功响应
#[derive(Debug, Serialize)]
pub struct AuthResponse {
    pub access_token:  String,
    pub refresh_token: String,
    pub user:          UserInfo,
}

/// 用户基本信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserInfo {
    pub id:      uuid::Uuid,
    pub name:    String,
    pub role:    String,
    pub enabled: bool,
}

/// 刷新令牌请求
#[derive(Debug, Deserialize)]
pub struct RefreshRequest {
    pub refresh_token: String,
}

/// 修改密码请求
#[derive(Debug, Deserialize)]
pub struct ChangePasswordRequest {
    pub old_password: String,
    pub new_password: String,
}

/// 用户详细档案
#[derive(Debug, Serialize)]
pub struct UserProfile {
    #[serde(flatten)]
    pub user:      UserInfo,
    pub platforms: Vec<PlatformInfo>,
}

/// 平台绑定信息
#[derive(Debug, Clone, Serialize)]
pub struct PlatformInfo {
    pub platform:         String,
    pub platform_user_id: String,
    pub display_name:     Option<String>,
    pub linked_at:        String,
}

/// 邀请码信息
#[derive(Debug, Serialize)]
pub struct InviteCode {
    pub id:         uuid::Uuid,
    pub code:       String,
    pub created_by: uuid::Uuid,
    pub used_by:    Option<uuid::Uuid>,
    pub expires_at: String,
    pub created_at: String,
}

/// 链接码信息
#[derive(Debug, Serialize)]
pub struct LinkCode {
    pub id:         uuid::Uuid,
    pub code:       String,
    pub user_id:    uuid::Uuid,
    pub direction:  String,
    pub expires_at: String,
    pub created_at: String,
}

/// 链接码验证结果
#[derive(Debug, Serialize)]
pub struct LinkCodeInfo {
    pub user_id:       uuid::Uuid,
    pub direction:     String,
    pub platform_data: Option<serde_json::Value>,
}

/// 生成链接码请求
#[derive(Debug, Deserialize)]
pub struct GenerateLinkCodeRequest {
    pub direction: String,
}

/// TG→Web 链接验证请求
#[derive(Debug, Deserialize)]
pub struct LinkVerifyRequest {
    pub code: String,
}
