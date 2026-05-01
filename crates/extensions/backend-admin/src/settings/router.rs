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
//! | Method | Path                       | Description            |
//! |--------|----------------------------|------------------------|
//! | GET    | `/api/v1/settings`         | list all               |
//! | PATCH  | `/api/v1/settings`         | batch update           |
//! | GET    | `/api/v1/settings/{*key}`  | get one                |
//! | PUT    | `/api/v1/settings/{*key}`  | set one                |
//! | DELETE | `/api/v1/settings/{*key}`  | delete one             |

use std::{collections::HashMap, sync::Arc};

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};
use rara_domain_shared::settings::{SettingsProvider, keys};
use rara_kernel::llm::DriverRegistryRef;
use utoipa_axum::router::OpenApiRouter;

use crate::{chat::model_catalog::ModelCatalog, settings::SettingsSvc};

// -- state wrapper --

type SharedProvider = Arc<dyn SettingsProvider>;

/// Combined state for the settings router.
///
/// Bundles the settings provider with the
/// [`DriverRegistry`](rara_kernel::llm::DriverRegistry) reference
/// and chat-model [`ModelCatalog`] so a `PATCH /api/v1/settings`
/// touching `llm.default_provider` can both flip the active driver and
/// drop the cached model list in one request handler — no event bus,
/// no settings watcher (issue #2014).
#[derive(Clone)]
pub struct SettingsRouterState {
    /// Settings KV provider — list / get / set / delete / batch_update.
    pub provider:        SharedProvider,
    /// Kernel driver registry, used to swap `default_driver` when
    /// `llm.default_provider` changes.
    pub driver_registry: DriverRegistryRef,
    /// Chat-model catalog whose 5-minute cache must be invalidated when
    /// the active provider switches.
    pub model_catalog:   ModelCatalog,
}

pub fn routes(
    svc: SettingsSvc,
    driver_registry: DriverRegistryRef,
    model_catalog: ModelCatalog,
) -> OpenApiRouter {
    let provider: SharedProvider = Arc::new(svc);
    let state = SettingsRouterState {
        provider,
        driver_registry,
        model_catalog,
    };

    let settings_router = axum::Router::new()
        .route(
            "/api/v1/settings",
            get(list_settings).patch(batch_update_settings),
        )
        .route(
            "/api/v1/settings/{*key}",
            get(get_setting).put(set_setting).delete(delete_setting),
        )
        .with_state(state);

    OpenApiRouter::from(settings_router)
}

// -- request / response types -----------------------------------------------

#[derive(Debug, serde::Deserialize)]
struct SetValueBody {
    value: String,
}

// -- handlers ---------------------------------------------------------------

async fn list_settings(State(state): State<SettingsRouterState>) -> Json<HashMap<String, String>> {
    Json(state.provider.list().await)
}

async fn get_setting(
    State(state): State<SettingsRouterState>,
    Path(key): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let key = key.trim_start_matches('/');
    match state.provider.get(key).await {
        Some(value) => Ok(Json(serde_json::json!({ "key": key, "value": value }))),
        None => Err(StatusCode::NOT_FOUND),
    }
}

async fn set_setting(
    State(state): State<SettingsRouterState>,
    Path(key): Path<String>,
    Json(body): Json<SetValueBody>,
) -> Result<StatusCode, (StatusCode, String)> {
    let key = key.trim_start_matches('/');
    state
        .provider
        .set(key, &body.value)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    apply_default_provider_side_effects(&state, key, Some(body.value.as_str())).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_setting(
    State(state): State<SettingsRouterState>,
    Path(key): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    let key = key.trim_start_matches('/');
    state
        .provider
        .delete(key)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn batch_update_settings(
    State(state): State<SettingsRouterState>,
    axum::Extension(principal): axum::Extension<
        rara_kernel::identity::Principal<rara_kernel::identity::Resolved>,
    >,
    Json(patches): Json<HashMap<String, Option<String>>>,
) -> Result<StatusCode, (StatusCode, String)> {
    // Runtime settings are admin-only; reject plain users even if the
    // bearer token matched (useful once per-user tokens are introduced).
    if !principal.is_admin() {
        return Err((
            StatusCode::FORBIDDEN,
            "settings mutation requires admin role".to_owned(),
        ));
    }
    tracing::info!(
        actor = %principal.user_id,
        keys = patches.len(),
        "settings.batch_update"
    );
    // Capture the default-provider patch before the map is consumed by
    // `batch_update` so the side-effect can fire with the new value.
    let default_provider_patch = patches.get(keys::LLM_DEFAULT_PROVIDER).cloned();
    state
        .provider
        .batch_update(patches)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    if let Some(value) = default_provider_patch {
        apply_default_provider_side_effects(&state, keys::LLM_DEFAULT_PROVIDER, value.as_deref())
            .await;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Re-route the runtime lister/embedder and drop the chat-model cache
/// when `llm.default_provider` changes.
///
/// Called from both `set_setting` (single-key PUT) and
/// `batch_update_settings` (PATCH) so all write paths converge on the
/// same invalidation. A `None` value (key deleted) is treated as a
/// switch — the registry's stale `default_driver` would otherwise
/// continue serving until the operator picks a new provider; we
/// invalidate the cache anyway so the next read hits whatever
/// `default_driver` resolves to (typically still `default_driver` from
/// boot config).
pub(crate) async fn apply_default_provider_side_effects(
    state: &SettingsRouterState,
    key: &str,
    value: Option<&str>,
) {
    if key != keys::LLM_DEFAULT_PROVIDER {
        return;
    }
    if let Some(v) = value {
        let trimmed = v.trim();
        if !trimmed.is_empty() {
            state.driver_registry.set_default_driver(trimmed);
        }
    }
    state.model_catalog.invalidate().await;
    tracing::info!(
        new_provider = ?value,
        "llm.default_provider changed: registry default swapped, chat-model cache invalidated"
    );
}
