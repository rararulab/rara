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

use axum::{Json, extract::State, http::StatusCode};
use rara_domain_shared::settings::model::{
    AgentRuntimeSettingsPatch, GmailRuntimeSettingsPatch, Settings, UpdateRequest,
};
use utoipa_axum::router::OpenApiRouter;

use crate::settings::SettingsSvc;

#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct GmailAdminSettingsView {
    pub configured:        bool,
    pub auto_send_enabled: bool,
    pub address:           Option<String>,
    pub app_password_hint: Option<String>,
    #[schema(value_type = Option<String>)]
    pub updated_at:        Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, Default, serde::Deserialize, utoipa::ToSchema)]
pub struct GmailAdminUpdateRequest {
    pub address:           Option<String>,
    pub app_password:      Option<String>,
    pub auto_send_enabled: Option<bool>,
}

pub(super) fn routes() -> OpenApiRouter<SettingsSvc> {
    OpenApiRouter::new().route(
        "/api/v1/gmail/settings",
        axum::routing::get(get_gmail_settings).put(update_gmail_settings),
    )
}

#[utoipa::path(get, path = "/api/v1/gmail/settings", tag = "gmail-admin", responses((status = 200, body = GmailAdminSettingsView)))]
async fn get_gmail_settings(State(state): State<SettingsSvc>) -> Json<GmailAdminSettingsView> {
    Json(GmailAdminSettingsView::from(state.current()))
}

#[utoipa::path(put, path = "/api/v1/gmail/settings", tag = "gmail-admin", request_body = GmailAdminUpdateRequest, responses((status = 200, body = GmailAdminSettingsView)))]
async fn update_gmail_settings(
    State(state): State<SettingsSvc>,
    Json(req): Json<GmailAdminUpdateRequest>,
) -> Result<Json<GmailAdminSettingsView>, (StatusCode, String)> {
    let updated = state
        .update(UpdateRequest {
            agent: Some(AgentRuntimeSettingsPatch {
                gmail: Some(GmailRuntimeSettingsPatch {
                    address:           req.address,
                    app_password:      req.app_password,
                    auto_send_enabled: req.auto_send_enabled,
                }),
                ..AgentRuntimeSettingsPatch::default()
            }),
            ..UpdateRequest::default()
        })
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to update gmail settings: {e}"),
            )
        })?;

    Ok(Json(GmailAdminSettingsView::from(updated)))
}

impl From<Settings> for GmailAdminSettingsView {
    fn from(value: Settings) -> Self {
        let gmail = value.agent.gmail;
        Self {
            configured:        gmail.address.is_some() && gmail.app_password.is_some(),
            auto_send_enabled: gmail.auto_send_enabled,
            address:           gmail.address,
            app_password_hint: secret_hint(gmail.app_password.as_deref()),
            updated_at:        value.updated_at,
        }
    }
}

fn secret_hint(secret: Option<&str>) -> Option<String> {
    let secret = secret?;
    let chars: Vec<char> = secret.chars().collect();
    if chars.is_empty() {
        return None;
    }
    if chars.len() <= 4 {
        return Some("*".repeat(chars.len()));
    }
    let suffix: String = chars[chars.len() - 4..].iter().collect();
    Some(format!("***{suffix}"))
}
