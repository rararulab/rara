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

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{get, post},
};
use job_domain_shared::id::SchedulerTaskId;
use serde::Deserialize;
use tracing::instrument;
use uuid::Uuid;

use crate::{
    error::SchedulerError,
    service::SchedulerService,
    types::{ScheduledTask, TaskFilter, TaskRunRecord},
};

#[derive(Debug, Deserialize)]
pub struct TaskListQuery {
    pub enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct HistoryQuery {
    pub limit: Option<i64>,
}

/// Register all scheduler routes on a new router with shared state.
pub fn routes(service: Arc<SchedulerService>) -> Router {
    Router::new()
        .route("/api/v1/scheduler/tasks", get(list_tasks))
        .route("/api/v1/scheduler/tasks/{id}", get(get_task))
        .route("/api/v1/scheduler/tasks/{id}/enable", post(enable_task))
        .route("/api/v1/scheduler/tasks/{id}/disable", post(disable_task))
        .route("/api/v1/scheduler/tasks/{id}/history", get(get_history))
        .with_state(service)
}

#[instrument(skip(service))]
async fn list_tasks(
    State(service): State<Arc<SchedulerService>>,
    Query(query): Query<TaskListQuery>,
) -> Result<Json<Vec<ScheduledTask>>, SchedulerError> {
    let filter = TaskFilter {
        enabled:       query.enabled,
        name_contains: None,
    };
    let tasks = service.list_tasks(&filter).await?;
    Ok(Json(tasks))
}

#[instrument(skip(service))]
async fn get_task(
    State(service): State<Arc<SchedulerService>>,
    Path(id): Path<Uuid>,
) -> Result<Json<ScheduledTask>, SchedulerError> {
    let task = service.get_task(SchedulerTaskId::from(id)).await?;
    Ok(Json(task))
}

#[instrument(skip(service))]
async fn enable_task(
    State(service): State<Arc<SchedulerService>>,
    Path(id): Path<Uuid>,
) -> Result<Json<ScheduledTask>, SchedulerError> {
    let task = service.enable_task(SchedulerTaskId::from(id)).await?;
    Ok(Json(task))
}

#[instrument(skip(service))]
async fn disable_task(
    State(service): State<Arc<SchedulerService>>,
    Path(id): Path<Uuid>,
) -> Result<Json<ScheduledTask>, SchedulerError> {
    let task = service.disable_task(SchedulerTaskId::from(id)).await?;
    Ok(Json(task))
}

#[instrument(skip(service))]
async fn get_history(
    State(service): State<Arc<SchedulerService>>,
    Path(id): Path<Uuid>,
    Query(query): Query<HistoryQuery>,
) -> Result<Json<Vec<TaskRunRecord>>, SchedulerError> {
    let limit = query.limit.unwrap_or(20);
    let history = service
        .get_history(SchedulerTaskId::from(id), limit)
        .await?;
    Ok(Json(history))
}
