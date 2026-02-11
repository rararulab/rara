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

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{delete, get, post, put},
};
use job_domain_shared::id::InterviewId;
use serde::Deserialize;
use tracing::instrument;
use uuid::Uuid;

use crate::{
    error::InterviewError,
    service::InterviewService,
    types::{
        CreateInterviewPlanRequest, InterviewFilter, InterviewPlan, InterviewTaskStatus,
        PrepGenerationRequest, UpdateInterviewPlanRequest,
    },
};

#[derive(Debug, Deserialize)]
struct UpdateStatusRequest {
    status: InterviewTaskStatus,
}

/// Register all interview routes on a new router with shared state.
pub fn routes(service: InterviewService) -> Router {
    Router::new()
        .route("/api/v1/interviews", post(create_interview))
        .route("/api/v1/interviews", get(list_interviews))
        .route("/api/v1/interviews/{id}", get(get_interview))
        .route("/api/v1/interviews/{id}", put(update_interview))
        .route("/api/v1/interviews/{id}/status", post(update_status))
        .route("/api/v1/interviews/{id}/prep", post(regenerate_prep))
        .route("/api/v1/interviews/{id}", delete(delete_interview))
        .with_state(service)
}

#[instrument(skip(service, req))]
async fn create_interview(
    State(service): State<InterviewService>,
    Json(req): Json<CreateInterviewPlanRequest>,
) -> Result<(StatusCode, Json<InterviewPlan>), InterviewError> {
    let plan = service.create_plan(req).await?;
    Ok((StatusCode::CREATED, Json(plan)))
}

#[instrument(skip(service))]
async fn list_interviews(
    State(service): State<InterviewService>,
    Query(filter): Query<InterviewFilter>,
) -> Result<Json<Vec<InterviewPlan>>, InterviewError> {
    let plans = service.list_plans(&filter).await?;
    Ok(Json(plans))
}

#[instrument(skip(service))]
async fn get_interview(
    State(service): State<InterviewService>,
    Path(id): Path<Uuid>,
) -> Result<Json<InterviewPlan>, InterviewError> {
    let plan = service.get_plan(InterviewId::from(id)).await?;
    Ok(Json(plan))
}

#[instrument(skip(service, req))]
async fn update_interview(
    State(service): State<InterviewService>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateInterviewPlanRequest>,
) -> Result<Json<InterviewPlan>, InterviewError> {
    let plan = service.update_plan(InterviewId::from(id), req).await?;
    Ok(Json(plan))
}

#[instrument(skip(service, body))]
async fn update_status(
    State(service): State<InterviewService>,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateStatusRequest>,
) -> Result<Json<InterviewPlan>, InterviewError> {
    let plan = service
        .update_status(InterviewId::from(id), body.status)
        .await?;
    Ok(Json(plan))
}

#[instrument(skip(service, prep_req))]
async fn regenerate_prep(
    State(service): State<InterviewService>,
    Path(id): Path<Uuid>,
    Json(prep_req): Json<PrepGenerationRequest>,
) -> Result<Json<InterviewPlan>, InterviewError> {
    let plan = service
        .regenerate_prep(InterviewId::from(id), prep_req)
        .await?;
    Ok(Json(plan))
}

#[instrument(skip(service))]
async fn delete_interview(
    State(service): State<InterviewService>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, InterviewError> {
    service.delete_plan(InterviewId::from(id)).await?;
    Ok(StatusCode::NO_CONTENT)
}
