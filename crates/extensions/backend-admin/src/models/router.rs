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

//! HTTP API routes for model configuration management.
//!
//! ## Route table
//!
//! | Method   | Path                          | Description              |
//! |----------|-------------------------------|--------------------------|
//! | `GET`    | `/api/v1/models`              | List all model mappings  |
//! | `GET`    | `/api/v1/models/fallbacks`    | Get fallback models      |
//! | `PUT`    | `/api/v1/models/fallbacks`    | Set fallback models      |
//! | `GET`    | `/api/v1/models/{key}`        | Get model for key        |
//! | `PUT`    | `/api/v1/models/{key}`        | Set model for key        |
//! | `DELETE` | `/api/v1/models/{key}`        | Remove model key         |

use std::sync::Arc;

use agent_core::model_repo::ModelRepo;
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};
use serde::{Deserialize, Serialize};
use utoipa_axum::router::OpenApiRouter;

// ---------------------------------------------------------------------------
// Request / Response types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct ModelEntryView {
    pub key:   String,
    pub model: String,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct ModelListView {
    pub models: Vec<ModelEntryView>,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct ModelValueView {
    pub key:   String,
    pub model: String,
}

#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct SetModelRequest {
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct FallbackModelsView {
    pub models: Vec<String>,
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

type RepoState = Arc<dyn ModelRepo>;

/// Build an [`OpenApiRouter`] with model CRUD endpoints.
pub fn routes(repo: Arc<dyn ModelRepo>) -> OpenApiRouter {
    OpenApiRouter::new()
        .route("/api/v1/models", get(list_models))
        .route(
            "/api/v1/models/fallbacks",
            get(get_fallbacks).put(set_fallbacks),
        )
        .route(
            "/api/v1/models/{key}",
            get(get_model).put(set_model).delete(delete_model),
        )
        .with_state(repo)
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /api/v1/models` -- list all key-model mappings.
#[utoipa::path(
    get,
    path = "/api/v1/models",
    tag = "models",
    responses(
        (status = 200, description = "All model key-value mappings", body = ModelListView),
    )
)]
async fn list_models(State(repo): State<RepoState>) -> Json<ModelListView> {
    let entries = repo.list().await;
    let models = entries
        .into_iter()
        .map(|e| ModelEntryView {
            key:   e.key,
            model: e.model,
        })
        .collect();
    Json(ModelListView { models })
}

/// `GET /api/v1/models/fallbacks` -- get the global fallback model list.
#[utoipa::path(
    get,
    path = "/api/v1/models/fallbacks",
    tag = "models",
    responses(
        (status = 200, description = "Global fallback models", body = FallbackModelsView),
    )
)]
async fn get_fallbacks(State(repo): State<RepoState>) -> Json<FallbackModelsView> {
    let models = repo.fallback_models().await;
    Json(FallbackModelsView { models })
}

/// `PUT /api/v1/models/fallbacks` -- replace the global fallback model list.
#[utoipa::path(
    put,
    path = "/api/v1/models/fallbacks",
    tag = "models",
    request_body = FallbackModelsView,
    responses(
        (status = 200, description = "Fallback models updated", body = FallbackModelsView),
        (status = 500, description = "Internal server error"),
    )
)]
async fn set_fallbacks(
    State(repo): State<RepoState>,
    Json(req): Json<FallbackModelsView>,
) -> Result<Json<FallbackModelsView>, (StatusCode, String)> {
    repo.set_fallback_models(req.models.clone())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(req))
}

/// `GET /api/v1/models/{key}` -- get the model for a specific key.
#[utoipa::path(
    get,
    path = "/api/v1/models/{key}",
    tag = "models",
    params(("key" = String, Path, description = "Model key (e.g. chat, job, pipeline)")),
    responses(
        (status = 200, description = "Model for key", body = ModelValueView),
    )
)]
async fn get_model(State(repo): State<RepoState>, Path(key): Path<String>) -> Json<ModelValueView> {
    let model = repo.get(&key).await;
    Json(ModelValueView { key, model })
}

/// `PUT /api/v1/models/{key}` -- set the model for a specific key.
#[utoipa::path(
    put,
    path = "/api/v1/models/{key}",
    tag = "models",
    params(("key" = String, Path, description = "Model key")),
    request_body = SetModelRequest,
    responses(
        (status = 200, description = "Model set", body = ModelValueView),
        (status = 500, description = "Internal server error"),
    )
)]
async fn set_model(
    State(repo): State<RepoState>,
    Path(key): Path<String>,
    Json(req): Json<SetModelRequest>,
) -> Result<Json<ModelValueView>, (StatusCode, String)> {
    repo.set(&key, &req.model)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(ModelValueView {
        key,
        model: req.model,
    }))
}

/// `DELETE /api/v1/models/{key}` -- remove a model key mapping.
#[utoipa::path(
    delete,
    path = "/api/v1/models/{key}",
    tag = "models",
    params(("key" = String, Path, description = "Model key")),
    responses(
        (status = 204, description = "Model key removed"),
        (status = 500, description = "Internal server error"),
    )
)]
async fn delete_model(
    State(repo): State<RepoState>,
    Path(key): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    repo.remove(&key)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}
