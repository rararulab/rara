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

//! Flat KV settings HTTP API.
//!
//! | Method | Path                                    | Description            |
//! |--------|-----------------------------------------|------------------------|
//! | GET    | `/api/v1/settings`                      | list all               |
//! | PATCH  | `/api/v1/settings`                      | batch update           |
//! | GET    | `/api/v1/settings/{*key}`               | get one                |
//! | PUT    | `/api/v1/settings/{*key}`               | set one                |
//! | DELETE | `/api/v1/settings/{*key}`               | delete one             |
//! | GET    | `/api/v1/settings/versions`             | list recent versions   |
//! | GET    | `/api/v1/settings/versions/current`     | current version number |
//! | GET    | `/api/v1/settings/versions/{n}`         | snapshot at version N  |
//! | POST   | `/api/v1/settings/versions/{n}/rollback`| rollback to version N  |

use std::{collections::HashMap, sync::Arc};

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use rara_domain_shared::settings::SettingsProvider;
use utoipa_axum::router::OpenApiRouter;

use crate::settings::{SettingsSvc, service::VersionEntry};

// -- state wrapper --

type SharedProvider = Arc<dyn SettingsProvider>;

pub fn routes(svc: SettingsSvc) -> OpenApiRouter {
    let svc = Arc::new(svc);
    let provider: SharedProvider = svc.clone();

    let settings_router = axum::Router::new()
        .route(
            "/api/v1/settings",
            get(list_settings).patch(batch_update_settings),
        )
        .route(
            "/api/v1/settings/{*key}",
            get(get_setting).put(set_setting).delete(delete_setting),
        )
        .with_state(provider);

    // Version routes use Arc<SettingsSvc> directly — these methods live on the
    // concrete type, not the SettingsProvider trait.
    let version_router = axum::Router::new()
        .route("/api/v1/settings/versions", get(list_versions))
        .route(
            "/api/v1/settings/versions/current",
            get(get_current_version),
        )
        .route("/api/v1/settings/versions/{n}", get(snapshot_at_version))
        .route(
            "/api/v1/settings/versions/{n}/rollback",
            post(rollback_to_version),
        )
        .with_state(svc);

    // Version routes merged first so they take precedence over the `{*key}`
    // wildcard.
    OpenApiRouter::from(version_router.merge(settings_router))
}

// -- request / response types -----------------------------------------------

#[derive(Debug, serde::Deserialize)]
struct SetValueBody {
    value: String,
}

// -- handlers ---------------------------------------------------------------

async fn list_settings(State(provider): State<SharedProvider>) -> Json<HashMap<String, String>> {
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

// -- version handlers
// -----------------------------------------------------------

/// List recent version log entries.
async fn list_versions(
    State(svc): State<Arc<SettingsSvc>>,
) -> Result<Json<Vec<VersionEntry>>, StatusCode> {
    svc.list_versions(100)
        .await
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

/// Return the current global version number.
async fn get_current_version(
    State(svc): State<Arc<SettingsSvc>>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    svc.current_version()
        .await
        .map(|v| Json(serde_json::json!({"version": v})))
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

/// Return a point-in-time snapshot of all settings at version `n`.
async fn snapshot_at_version(
    State(svc): State<Arc<SettingsSvc>>,
    Path(version): Path<i64>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    svc.snapshot(version)
        .await
        .map(|snap| Json(serde_json::json!({"version": version, "settings": snap})))
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

/// Rollback settings to the state at version `n` (creates a new version).
async fn rollback_to_version(
    State(svc): State<Arc<SettingsSvc>>,
    Path(version): Path<i64>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    svc.rollback_to(version)
        .await
        .map(|new_ver| Json(serde_json::json!({"rolled_back_to": version, "new_version": new_ver})))
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}
