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
use job_domain_core::id::SchedulerTaskId;
use job_domain_resume::repository::ResumeRepository;
use job_domain_scheduler::types::{ScheduledTask, TaskFilter, TaskRunRecord};
use serde::Deserialize;
use uuid::Uuid;

use crate::{api::error::ApiError, state::AppState};

/// Register all scheduler routes on a new router with shared state.
pub fn scheduler_routes<R: ResumeRepository + 'static>(state: Arc<AppState<R>>) -> Router {
    Router::new()
        .route("/api/v1/scheduler/tasks", get(list_tasks::<R>))
        .route("/api/v1/scheduler/tasks/{id}", get(get_task::<R>))
        .route(
            "/api/v1/scheduler/tasks/{id}/enable",
            post(enable_task::<R>),
        )
        .route(
            "/api/v1/scheduler/tasks/{id}/disable",
            post(disable_task::<R>),
        )
        .route(
            "/api/v1/scheduler/tasks/{id}/history",
            get(get_history::<R>),
        )
        .with_state(state)
}

/// Query parameters for listing scheduler tasks.
#[derive(Debug, Deserialize)]
pub struct TaskListQuery {
    pub enabled: Option<bool>,
}

/// Query parameters for task run history.
#[derive(Debug, Deserialize)]
pub struct HistoryQuery {
    pub limit: Option<i64>,
}

/// GET /api/v1/scheduler/tasks
async fn list_tasks<R: ResumeRepository + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Query(query): Query<TaskListQuery>,
) -> Result<Json<Vec<ScheduledTask>>, ApiError> {
    let filter = TaskFilter {
        enabled: query.enabled,
        name_contains: None,
    };
    let tasks = state.scheduler_service.list_tasks(&filter).await?;
    Ok(Json(tasks))
}

/// GET /api/v1/scheduler/tasks/:id
async fn get_task<R: ResumeRepository + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Path(id): Path<Uuid>,
) -> Result<Json<ScheduledTask>, ApiError> {
    let task = state
        .scheduler_service
        .get_task(SchedulerTaskId::from(id))
        .await?;
    Ok(Json(task))
}

/// POST /api/v1/scheduler/tasks/:id/enable
async fn enable_task<R: ResumeRepository + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Path(id): Path<Uuid>,
) -> Result<Json<ScheduledTask>, ApiError> {
    let task = state
        .scheduler_service
        .enable_task(SchedulerTaskId::from(id))
        .await?;
    Ok(Json(task))
}

/// POST /api/v1/scheduler/tasks/:id/disable
async fn disable_task<R: ResumeRepository + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Path(id): Path<Uuid>,
) -> Result<Json<ScheduledTask>, ApiError> {
    let task = state
        .scheduler_service
        .disable_task(SchedulerTaskId::from(id))
        .await?;
    Ok(Json(task))
}

/// GET /api/v1/scheduler/tasks/:id/history
async fn get_history<R: ResumeRepository + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Path(id): Path<Uuid>,
    Query(query): Query<HistoryQuery>,
) -> Result<Json<Vec<TaskRunRecord>>, ApiError> {
    let limit = query.limit.unwrap_or(20);
    let history = state
        .scheduler_service
        .get_history(SchedulerTaskId::from(id), limit)
        .await?;
    Ok(Json(history))
}
