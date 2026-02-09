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

//! HTTP API routes for interview plan management.

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{delete, get, post, put},
};
use job_domain_core::id::InterviewId;
use job_domain_interview::types::{
    CreateInterviewPlanRequest, InterviewFilter, InterviewPlan, InterviewTaskStatus,
    PrepGenerationRequest, UpdateInterviewPlanRequest,
};
use job_domain_resume::repository::ResumeRepository;
use serde::Deserialize;
use uuid::Uuid;

use crate::{api::error::ApiError, state::AppState};

/// Register all interview routes on a new router with shared state.
pub fn interview_routes<R: ResumeRepository + 'static>(state: Arc<AppState<R>>) -> Router {
    Router::new()
        .route("/api/v1/interviews", post(create_interview::<R>))
        .route("/api/v1/interviews", get(list_interviews::<R>))
        .route("/api/v1/interviews/{id}", get(get_interview::<R>))
        .route("/api/v1/interviews/{id}", put(update_interview::<R>))
        .route("/api/v1/interviews/{id}/status", post(update_status::<R>))
        .route("/api/v1/interviews/{id}/prep", post(regenerate_prep::<R>))
        .route("/api/v1/interviews/{id}", delete(delete_interview::<R>))
        .with_state(state)
}

/// JSON body for the status-update endpoint.
#[derive(Debug, Deserialize)]
struct UpdateStatusRequest {
    /// The new task status.
    status: InterviewTaskStatus,
}

/// POST /api/v1/interviews
async fn create_interview<R: ResumeRepository + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Json(req): Json<CreateInterviewPlanRequest>,
) -> Result<(StatusCode, Json<InterviewPlan>), ApiError> {
    let plan = state.interview_service.create_plan(req).await?;
    Ok((StatusCode::CREATED, Json(plan)))
}

/// GET /api/v1/interviews
async fn list_interviews<R: ResumeRepository + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Query(filter): Query<InterviewFilter>,
) -> Result<Json<Vec<InterviewPlan>>, ApiError> {
    let plans = state.interview_service.list_plans(&filter).await?;
    Ok(Json(plans))
}

/// GET /api/v1/interviews/:id
async fn get_interview<R: ResumeRepository + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Path(id): Path<Uuid>,
) -> Result<Json<InterviewPlan>, ApiError> {
    let plan = state
        .interview_service
        .get_plan(InterviewId::from(id))
        .await?;
    Ok(Json(plan))
}

/// PUT /api/v1/interviews/:id
async fn update_interview<R: ResumeRepository + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateInterviewPlanRequest>,
) -> Result<Json<InterviewPlan>, ApiError> {
    let plan = state
        .interview_service
        .update_plan(InterviewId::from(id), req)
        .await?;
    Ok(Json(plan))
}

/// POST /api/v1/interviews/:id/status
async fn update_status<R: ResumeRepository + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateStatusRequest>,
) -> Result<Json<InterviewPlan>, ApiError> {
    let plan = state
        .interview_service
        .update_status(InterviewId::from(id), body.status)
        .await?;
    Ok(Json(plan))
}

/// POST /api/v1/interviews/:id/prep
async fn regenerate_prep<R: ResumeRepository + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Path(id): Path<Uuid>,
    Json(prep_req): Json<PrepGenerationRequest>,
) -> Result<Json<InterviewPlan>, ApiError> {
    let plan = state
        .interview_service
        .regenerate_prep(InterviewId::from(id), prep_req)
        .await?;
    Ok(Json(plan))
}

/// DELETE /api/v1/interviews/:id
async fn delete_interview<R: ResumeRepository + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    state
        .interview_service
        .delete_plan(InterviewId::from(id))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
