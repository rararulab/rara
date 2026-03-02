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

//! REST API routes for coding task management.

use axum::{
    Json,
    extract::{Path, State},
    routing::{get, post},
};
use rara_coding_task::{
    error::CodingTaskError,
    service::CodingTaskService,
    types::{CodingTaskDetail, CodingTaskSummary, CreateCodingTaskRequest},
};
use uuid::Uuid;

/// Build the coding-task routes. Returns a plain `axum::Router` (no OpenAPI).
pub fn routes(service: CodingTaskService) -> axum::Router {
    axum::Router::new()
        .route("/api/v1/coding-tasks", get(list_tasks).post(dispatch_task))
        .route("/api/v1/coding-tasks/{id}", get(get_task))
        .route("/api/v1/coding-tasks/{id}/merge", post(merge_task))
        .route("/api/v1/coding-tasks/{id}/cancel", post(cancel_task))
        .with_state(service)
}

async fn list_tasks(
    State(svc): State<CodingTaskService>,
) -> Result<Json<Vec<CodingTaskSummary>>, CodingTaskError> {
    let tasks = svc.list().await?;
    let summaries: Vec<CodingTaskSummary> = tasks.iter().map(CodingTaskSummary::from).collect();
    Ok(Json(summaries))
}

async fn dispatch_task(
    State(svc): State<CodingTaskService>,
    Json(req): Json<CreateCodingTaskRequest>,
) -> Result<Json<CodingTaskDetail>, CodingTaskError> {
    let task = svc
        .dispatch(
            req.repo_url.as_deref(),
            &req.prompt,
            req.agent_type,
            req.session_key,
        )
        .await?;
    Ok(Json(CodingTaskDetail::from(&task)))
}

async fn get_task(
    State(svc): State<CodingTaskService>,
    Path(id): Path<Uuid>,
) -> Result<Json<CodingTaskDetail>, CodingTaskError> {
    let task = svc.get(id).await?;
    Ok(Json(CodingTaskDetail::from(&task)))
}

async fn merge_task(
    State(svc): State<CodingTaskService>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, CodingTaskError> {
    svc.merge(id).await?;
    Ok(Json(serde_json::json!({ "status": "merged" })))
}

async fn cancel_task(
    State(svc): State<CodingTaskService>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, CodingTaskError> {
    svc.cancel(id).await?;
    Ok(Json(serde_json::json!({ "status": "cancelled" })))
}
