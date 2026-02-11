// Copyright 2026 Crrow
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

use std::sync::Arc;

use axum::{Json, Router, extract::State, http::StatusCode, routing::post};

use crate::{
    runtime_settings::{RuntimeSettings, RuntimeSettingsPatch},
    runtime_settings_service::{RuntimeSettingsService, RuntimeSettingsView, to_view},
};

type OnUpdated = Arc<dyn Fn(&RuntimeSettings) -> Result<(), String> + Send + Sync>;

#[derive(Clone)]
struct RouteState {
    settings_service: Arc<RuntimeSettingsService>,
    on_updated:       Option<OnUpdated>,
}

/// Build `/api/v1/settings` routes.
pub fn routes(
    settings_service: Arc<RuntimeSettingsService>,
    on_updated: Option<OnUpdated>,
) -> Router {
    Router::new()
        .route("/api/v1/settings", axum::routing::get(get_settings))
        .route("/api/v1/settings", post(update_settings))
        .with_state(RouteState {
            settings_service,
            on_updated,
        })
}

async fn get_settings(
    State(state): State<RouteState>,
) -> Result<Json<RuntimeSettingsView>, (StatusCode, String)> {
    let current = state.settings_service.current();
    Ok(Json(to_view(&current)))
}

async fn update_settings(
    State(state): State<RouteState>,
    Json(patch): Json<RuntimeSettingsPatch>,
) -> Result<Json<RuntimeSettingsView>, (StatusCode, String)> {
    let updated = state.settings_service.update(patch).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to update runtime settings: {e}"),
        )
    })?;

    if let Some(on_updated) = &state.on_updated {
        on_updated(&updated).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    }

    Ok(Json(to_view(&updated)))
}
