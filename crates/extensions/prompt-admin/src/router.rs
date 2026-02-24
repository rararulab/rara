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

//! HTTP API routes for prompt management.
//!
//! All endpoints live under `/api/v1/prompts` and use JSON request/response
//! bodies. The router is constructed via [`routes`] and expects a shared
//! `Arc<dyn PromptRepo>` as axum state.
//!
//! ## Route table
//!
//! | Method   | Path                       | Description              |
//! |----------|----------------------------|--------------------------|
//! | `GET`    | `/api/v1/prompts`          | List all prompts         |
//! | `GET`    | `/api/v1/prompts/{*name}`  | Get a prompt by name     |
//! | `PUT`    | `/api/v1/prompts/{*name}`  | Update a prompt          |
//! | `DELETE` | `/api/v1/prompts/{*name}`  | Reset a prompt to default|

use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};
use agent_core::prompt::{PromptError, PromptRepo};
use serde::{Deserialize, Serialize};
use utoipa_axum::router::OpenApiRouter;

// ---------------------------------------------------------------------------
// Request / Response types
// ---------------------------------------------------------------------------

/// A prompt entry with its current effective content.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct PromptFileView {
    pub name:        String,
    pub description: String,
    pub content:     String,
}

/// Response body for `GET /api/v1/prompts`.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct PromptListView {
    pub prompts: Vec<PromptFileView>,
}

/// Request body for `PUT /api/v1/prompts/{*name}`.
#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct PromptUpdateRequest {
    pub content: String,
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

type RepoState = Arc<dyn PromptRepo>;

/// Build an [`OpenApiRouter`] with all prompt CRUD endpoints and the given
/// [`PromptRepo`] as shared state.
pub fn routes(repo: Arc<dyn PromptRepo>) -> OpenApiRouter {
    OpenApiRouter::new()
        .route(
            "/api/v1/prompts",
            get(list_prompts),
        )
        .route(
            "/api/v1/prompts/{*name}",
            get(get_prompt).put(update_prompt).delete(reset_prompt),
        )
        .with_state(repo)
}

// ---------------------------------------------------------------------------
// Error mapping
// ---------------------------------------------------------------------------

fn map_prompt_error(err: PromptError) -> (StatusCode, String) {
    match &err {
        PromptError::NotFound { .. } => (StatusCode::NOT_FOUND, err.to_string()),
        PromptError::Io { .. } | PromptError::Watcher { .. } => {
            (StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
        }
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /api/v1/prompts` -- list all registered prompts.
#[utoipa::path(
    get,
    path = "/api/v1/prompts",
    tag = "prompts",
    responses(
        (status = 200, description = "List of all prompts", body = PromptListView),
    )
)]
async fn list_prompts(State(repo): State<RepoState>) -> Json<PromptListView> {
    let entries = repo.list().await;
    let prompts = entries
        .into_iter()
        .map(|e| PromptFileView {
            name:        e.name,
            description: e.description,
            content:     e.content,
        })
        .collect();
    Json(PromptListView { prompts })
}

/// `GET /api/v1/prompts/{*name}` -- get a single prompt by name.
#[utoipa::path(
    get,
    path = "/api/v1/prompts/{name}",
    tag = "prompts",
    params(("name" = String, Path, description = "Prompt name (e.g. ai/job_fit.system.md)")),
    responses(
        (status = 200, description = "Prompt found", body = PromptFileView),
        (status = 404, description = "Prompt not found"),
    )
)]
async fn get_prompt(
    State(repo): State<RepoState>,
    Path(name): Path<String>,
) -> Result<Json<PromptFileView>, (StatusCode, String)> {
    let entry = repo
        .get(&name)
        .await
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("prompt not found: {name}")))?;

    Ok(Json(PromptFileView {
        name:        entry.name,
        description: entry.description,
        content:     entry.content,
    }))
}

/// `PUT /api/v1/prompts/{*name}` -- update a prompt's content.
#[utoipa::path(
    put,
    path = "/api/v1/prompts/{name}",
    tag = "prompts",
    params(("name" = String, Path, description = "Prompt name (e.g. ai/job_fit.system.md)")),
    request_body = PromptUpdateRequest,
    responses(
        (status = 200, description = "Prompt updated", body = PromptFileView),
        (status = 404, description = "Prompt not found"),
        (status = 500, description = "Internal server error"),
    )
)]
async fn update_prompt(
    State(repo): State<RepoState>,
    Path(name): Path<String>,
    Json(req): Json<PromptUpdateRequest>,
) -> Result<Json<PromptFileView>, (StatusCode, String)> {
    let entry = repo.update(&name, &req.content).await.map_err(map_prompt_error)?;

    Ok(Json(PromptFileView {
        name:        entry.name,
        description: entry.description,
        content:     entry.content,
    }))
}

/// `DELETE /api/v1/prompts/{*name}` -- reset a prompt to its compiled-in
/// default content.
#[utoipa::path(
    delete,
    path = "/api/v1/prompts/{name}",
    tag = "prompts",
    params(("name" = String, Path, description = "Prompt name (e.g. ai/job_fit.system.md)")),
    responses(
        (status = 200, description = "Prompt reset to default", body = PromptFileView),
        (status = 404, description = "Prompt not found"),
        (status = 500, description = "Internal server error"),
    )
)]
async fn reset_prompt(
    State(repo): State<RepoState>,
    Path(name): Path<String>,
) -> Result<Json<PromptFileView>, (StatusCode, String)> {
    let entry = repo.reset(&name).await.map_err(map_prompt_error)?;

    Ok(Json(PromptFileView {
        name:        entry.name,
        description: entry.description,
        content:     entry.content,
    }))
}
