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

//! HTTP API routes for scheduler task management.

use axum::{
    Json,
    extract::{Path, Query, State},
};
use rara_domain_scheduler::{
    error::SchedulerError,
    service::SchedulerService,
    types::{ScheduledTask, TaskFilter, TaskRunRecord},
};
use rara_domain_shared::id::SchedulerTaskId;
use serde::Deserialize;
use tracing::instrument;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct TaskListQuery {
    pub enabled: Option<bool>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct HistoryQuery {
    pub limit: Option<i64>,
}

/// Register all scheduler routes on a new router with shared state.
pub fn routes(service: SchedulerService) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_tasks))
        .routes(routes!(get_task))
        .routes(routes!(enable_task))
        .routes(routes!(disable_task))
        .routes(routes!(get_history))
        .with_state(service)
}

/// List scheduler tasks with optional filters.
#[utoipa::path(
    get,
    path = "/api/v1/scheduler/tasks",
    tag = "scheduler",
    params(
        ("enabled" = Option<bool>, Query, description = "Filter by enabled status"),
    ),
    responses(
        (status = 200, description = "List of scheduled tasks", body = Vec<ScheduledTask>),
    )
)]
#[instrument(skip(service))]
async fn list_tasks(
    State(service): State<SchedulerService>,
    Query(query): Query<TaskListQuery>,
) -> Result<Json<Vec<ScheduledTask>>, SchedulerError> {
    let filter = TaskFilter {
        enabled:       query.enabled,
        name_contains: None,
    };
    let tasks = service.list_tasks(&filter).await?;
    Ok(Json(tasks))
}

/// Get a single scheduler task by ID.
#[utoipa::path(
    get,
    path = "/api/v1/scheduler/tasks/{id}",
    tag = "scheduler",
    params(("id" = Uuid, Path, description = "Scheduler task ID")),
    responses(
        (status = 200, description = "Scheduler task found", body = ScheduledTask),
        (status = 404, description = "Scheduler task not found"),
    )
)]
#[instrument(skip(service))]
async fn get_task(
    State(service): State<SchedulerService>,
    Path(id): Path<Uuid>,
) -> Result<Json<ScheduledTask>, SchedulerError> {
    let task = service.get_task(SchedulerTaskId::from(id)).await?;
    Ok(Json(task))
}

/// Enable a scheduler task.
#[utoipa::path(
    post,
    path = "/api/v1/scheduler/tasks/{id}/enable",
    tag = "scheduler",
    params(("id" = Uuid, Path, description = "Scheduler task ID")),
    responses(
        (status = 200, description = "Task enabled", body = ScheduledTask),
    )
)]
#[instrument(skip(service))]
async fn enable_task(
    State(service): State<SchedulerService>,
    Path(id): Path<Uuid>,
) -> Result<Json<ScheduledTask>, SchedulerError> {
    let task = service.enable_task(SchedulerTaskId::from(id)).await?;
    Ok(Json(task))
}

/// Disable a scheduler task.
#[utoipa::path(
    post,
    path = "/api/v1/scheduler/tasks/{id}/disable",
    tag = "scheduler",
    params(("id" = Uuid, Path, description = "Scheduler task ID")),
    responses(
        (status = 200, description = "Task disabled", body = ScheduledTask),
    )
)]
#[instrument(skip(service))]
async fn disable_task(
    State(service): State<SchedulerService>,
    Path(id): Path<Uuid>,
) -> Result<Json<ScheduledTask>, SchedulerError> {
    let task = service.disable_task(SchedulerTaskId::from(id)).await?;
    Ok(Json(task))
}

/// Get run history for a scheduler task.
#[utoipa::path(
    get,
    path = "/api/v1/scheduler/tasks/{id}/history",
    tag = "scheduler",
    params(
        ("id" = Uuid, Path, description = "Scheduler task ID"),
        ("limit" = Option<i64>, Query, description = "Maximum number of history records to return"),
    ),
    responses(
        (status = 200, description = "Task run history", body = Vec<TaskRunRecord>),
    )
)]
#[instrument(skip(service))]
async fn get_history(
    State(service): State<SchedulerService>,
    Path(id): Path<Uuid>,
    Query(query): Query<HistoryQuery>,
) -> Result<Json<Vec<TaskRunRecord>>, SchedulerError> {
    let limit = query.limit.unwrap_or(20);
    let history = service
        .get_history(SchedulerTaskId::from(id), limit)
        .await?;
    Ok(Json(history))
}
