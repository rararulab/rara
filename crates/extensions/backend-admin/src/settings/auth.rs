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

use axum::{Json, http::StatusCode};
use utoipa_axum::router::OpenApiRouter;

use crate::settings::SettingsSvc;

#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct SshKeyResponse {
    pub public_key: String,
}

pub(super) fn routes() -> OpenApiRouter<SettingsSvc> {
    OpenApiRouter::new().route("/api/v1/auth/ssh-key", axum::routing::get(get_ssh_key))
}

#[utoipa::path(
    get,
    path = "/api/v1/auth/ssh-key",
    tag = "auth-admin",
    responses((status = 200, description = "SSH public key", body = SshKeyResponse))
)]
async fn get_ssh_key() -> Result<Json<SshKeyResponse>, (StatusCode, String)> {
    let ssh_dir = rara_paths::data_dir().join("ssh");
    let public_key = rara_git::get_public_key(&ssh_dir)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(SshKeyResponse { public_key }))
}
