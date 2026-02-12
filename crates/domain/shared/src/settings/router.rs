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

//! HTTP routes for runtime settings.

use axum::{Json, Router, extract::State, http::StatusCode, routing::get};

use crate::settings::{
    model::{Settings, UpdateRequest},
    service::SettingsSvc,
};

/// Build `/api/v1/settings` routes.
pub fn routes(svc: SettingsSvc) -> Router {
    Router::new()
        .nest(
            "/api/v1",
            Router::new().route("/settings", get(get_settings).post(update_settings)),
        )
        .with_state(svc)
}

async fn get_settings(
    State(state): State<SettingsSvc>,
) -> Result<Json<RuntimeSettingsView>, (StatusCode, String)> {
    let current = state.current();
    Ok(Json(current.into()))
}

async fn update_settings(
    State(state): State<SettingsSvc>,
    Json(req): Json<UpdateRequest>,
) -> Result<Json<RuntimeSettingsView>, (StatusCode, String)> {
    let updated = state.update(req).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to update runtime settings: {e}"),
        )
    })?;

    Ok(Json(updated.into()))
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct RuntimeSettingsView {
    pub ai:         AiSettingsView,
    pub telegram:   TgSettingsResp,
    // TODO: use jiff
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AiSettingsView {
    pub configured:         bool,
    pub model:              Option<String>,
    pub openrouter_api_key: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TgSettingsResp {
    pub configured: bool,
    pub chat_id:    Option<i64>,
    pub token_hint: Option<String>,
}

impl Into<RuntimeSettingsView> for Settings {
    fn into(self) -> RuntimeSettingsView {
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

        RuntimeSettingsView {
            ai:         AiSettingsView {
                configured:         self.ai.openrouter_api_key.is_some(),
                model:              self.ai.model.clone(),
                openrouter_api_key: self.ai.openrouter_api_key.clone(),
            },
            telegram:   TgSettingsResp {
                configured: self.telegram.bot_token.is_some() && self.telegram.chat_id.is_some(),
                chat_id:    self.telegram.chat_id,
                token_hint: secret_hint(self.telegram.bot_token.as_deref()),
            },
            updated_at: self.updated_at,
        }
    }
}
