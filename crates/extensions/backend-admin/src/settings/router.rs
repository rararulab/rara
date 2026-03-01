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

//! Flat KV settings HTTP API.
//!
//! | Method | Path                       | Description            |
//! |--------|----------------------------|------------------------|
//! | GET    | `/api/v1/settings`         | list all               |
//! | PATCH  | `/api/v1/settings`         | batch update           |
//! | GET    | `/api/v1/settings/{*key}`  | get one                |
//! | PUT    | `/api/v1/settings/{*key}`  | set one                |
//! | DELETE | `/api/v1/settings/{*key}`  | delete one             |

use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};
use rara_domain_shared::settings::SettingsProvider;
use utoipa_axum::router::OpenApiRouter;

use crate::settings::SettingsSvc;

// -- state wrapper --

type SharedProvider = Arc<dyn SettingsProvider>;

pub fn routes(svc: SettingsSvc) -> OpenApiRouter {
    let provider: SharedProvider = Arc::new(svc);

    let settings_router = axum::Router::new()
        .route("/api/v1/settings", get(list_settings).patch(batch_update_settings))
        .route("/api/v1/settings/{*key}", get(get_setting).put(set_setting).delete(delete_setting))
        .with_state(provider);

    OpenApiRouter::from(settings_router)
}

// -- request / response types -----------------------------------------------

#[derive(Debug, serde::Deserialize)]
struct SetValueBody {
    value: String,
}

// -- handlers ---------------------------------------------------------------

async fn list_settings(
    State(provider): State<SharedProvider>,
) -> Json<HashMap<String, String>> {
    Json(provider.list().await)
}

async fn get_setting(
    State(provider): State<SharedProvider>,
    Path(key): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let key = key.trim_start_matches('/');
    match provider.get(key).await {
        Some(value) => Ok(Json(serde_json::json!({ "key": key, "value": value }))),
        None => Err(StatusCode::NOT_FOUND),
    }
}

async fn set_setting(
    State(provider): State<SharedProvider>,
    Path(key): Path<String>,
    Json(body): Json<SetValueBody>,
) -> Result<StatusCode, (StatusCode, String)> {
    let key = key.trim_start_matches('/');
    provider
        .set(key, &body.value)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_setting(
    State(provider): State<SharedProvider>,
    Path(key): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    let key = key.trim_start_matches('/');
    provider
        .delete(key)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn batch_update_settings(
    State(provider): State<SharedProvider>,
    Json(patches): Json<HashMap<String, Option<String>>>,
) -> Result<StatusCode, (StatusCode, String)> {
    provider
        .batch_update(patches)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}
