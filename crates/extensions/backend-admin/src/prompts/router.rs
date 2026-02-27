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

//! HTTP API routes for prompt retrieval (read-only).
//!
//! All endpoints live under `/api/v1/prompts` and use JSON response bodies.
//! The router is constructed via [`routes`] and expects a shared
//! `Arc<dyn PromptRepo>` as axum state.
//!
//! ## Route table
//!
//! | Method | Path                      | Description          |
//! |--------|---------------------------|----------------------|
//! | `GET`  | `/api/v1/prompts`         | List all prompts     |
//! | `GET`  | `/api/v1/prompts/{*name}` | Get a prompt by name |

use std::sync::Arc;

use rara_kernel::prompt::PromptRepo;
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};
use serde::Serialize;
use utoipa_axum::router::OpenApiRouter;

// ---------------------------------------------------------------------------
// Response types
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

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

type RepoState = Arc<dyn PromptRepo>;

/// Build an [`OpenApiRouter`] with read-only prompt endpoints and the given
/// [`PromptRepo`] as shared state.
pub fn routes(repo: Arc<dyn PromptRepo>) -> OpenApiRouter {
    OpenApiRouter::new()
        .route("/api/v1/prompts", get(list_prompts))
        .route("/api/v1/prompts/{*name}", get(get_prompt))
        .with_state(repo)
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
