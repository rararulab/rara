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
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use rara_domain_shared::id::InterviewId;
use serde::Deserialize;
use tracing::instrument;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::{
    error::InterviewError,
    service::InterviewService,
    types::{
        CreateInterviewPlanRequest, InterviewFilter, InterviewPlan, InterviewTaskStatus,
        PrepGenerationRequest, UpdateInterviewPlanRequest,
    },
};

#[derive(Debug, Deserialize, utoipa::ToSchema)]
struct UpdateStatusRequest {
    status: InterviewTaskStatus,
}

/// Register all interview routes on a new router with shared state.
pub fn routes(service: InterviewService) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(create_interview, list_interviews))
        .routes(routes!(get_interview, update_interview, delete_interview))
        .routes(routes!(update_status))
        .routes(routes!(regenerate_prep))
        .with_state(service)
}

/// Create a new interview plan.
#[utoipa::path(
    post,
    path = "/api/v1/interviews",
    tag = "interviews",
    request_body = CreateInterviewPlanRequest,
    responses(
        (status = 201, description = "Interview plan created", body = InterviewPlan),
    )
)]
#[instrument(skip(service, req))]
async fn create_interview(
    State(service): State<InterviewService>,
    Json(req): Json<CreateInterviewPlanRequest>,
) -> Result<(StatusCode, Json<InterviewPlan>), InterviewError> {
    let plan = service.create_plan(req).await?;
    Ok((StatusCode::CREATED, Json(plan)))
}

/// List interview plans with optional filters.
#[utoipa::path(
    get,
    path = "/api/v1/interviews",
    tag = "interviews",
    params(
        ("application_id" = Option<String>, Query, description = "Filter by application ID"),
        ("company" = Option<String>, Query, description = "Filter by company name"),
        ("task_status" = Option<InterviewTaskStatus>, Query, description = "Filter by task status"),
        ("round" = Option<String>, Query, description = "Filter by interview round"),
        ("scheduled_after" = Option<String>, Query, description = "Scheduled at or after this timestamp"),
        ("scheduled_before" = Option<String>, Query, description = "Scheduled at or before this timestamp"),
    ),
    responses(
        (status = 200, description = "List of interview plans", body = Vec<InterviewPlan>),
    )
)]
#[instrument(skip(service))]
async fn list_interviews(
    State(service): State<InterviewService>,
    Query(filter): Query<InterviewFilter>,
) -> Result<Json<Vec<InterviewPlan>>, InterviewError> {
    let plans = service.list_plans(&filter).await?;
    Ok(Json(plans))
}

/// Get a single interview plan by ID.
#[utoipa::path(
    get,
    path = "/api/v1/interviews/{id}",
    tag = "interviews",
    params(("id" = Uuid, Path, description = "Interview plan ID")),
    responses(
        (status = 200, description = "Interview plan found", body = InterviewPlan),
        (status = 404, description = "Interview plan not found"),
    )
)]
#[instrument(skip(service))]
async fn get_interview(
    State(service): State<InterviewService>,
    Path(id): Path<Uuid>,
) -> Result<Json<InterviewPlan>, InterviewError> {
    let plan = service.get_plan(InterviewId::from(id)).await?;
    Ok(Json(plan))
}

/// Update an existing interview plan.
#[utoipa::path(
    put,
    path = "/api/v1/interviews/{id}",
    tag = "interviews",
    params(("id" = Uuid, Path, description = "Interview plan ID")),
    request_body = UpdateInterviewPlanRequest,
    responses(
        (status = 200, description = "Interview plan updated", body = InterviewPlan),
    )
)]
#[instrument(skip(service, req))]
async fn update_interview(
    State(service): State<InterviewService>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateInterviewPlanRequest>,
) -> Result<Json<InterviewPlan>, InterviewError> {
    let plan = service.update_plan(InterviewId::from(id), req).await?;
    Ok(Json(plan))
}

/// Update the status of an interview plan.
#[utoipa::path(
    post,
    path = "/api/v1/interviews/{id}/status",
    tag = "interviews",
    params(("id" = Uuid, Path, description = "Interview plan ID")),
    request_body = UpdateStatusRequest,
    responses(
        (status = 200, description = "Status updated", body = InterviewPlan),
    )
)]
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

/// Regenerate AI prep materials for an interview plan.
#[utoipa::path(
    post,
    path = "/api/v1/interviews/{id}/prep",
    tag = "interviews",
    params(("id" = Uuid, Path, description = "Interview plan ID")),
    request_body = PrepGenerationRequest,
    responses(
        (status = 200, description = "Prep materials regenerated", body = InterviewPlan),
    )
)]
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

/// Delete an interview plan.
#[utoipa::path(
    delete,
    path = "/api/v1/interviews/{id}",
    tag = "interviews",
    params(("id" = Uuid, Path, description = "Interview plan ID")),
    responses(
        (status = 204, description = "Interview plan deleted"),
    )
)]
#[instrument(skip(service))]
async fn delete_interview(
    State(service): State<InterviewService>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, InterviewError> {
    service.delete_plan(InterviewId::from(id)).await?;
    Ok(StatusCode::NO_CONTENT)
}
