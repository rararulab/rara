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

//! HTTP API routes for application lifecycle management.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use rara_domain_shared::id::ApplicationId;
use serde::Deserialize;
use tracing::instrument;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::{
    error::ApplicationError,
    service::ApplicationService,
    types::{
        Application, ApplicationChannel, ApplicationFilter, ApplicationStatus, ChangeSource,
        CreateApplicationRequest, Priority, StatusChangeRecord, UpdateApplicationRequest,
    },
};

/// JSON body for the status transition endpoint.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
struct TransitionRequest {
    status: ApplicationStatus,
    source: ChangeSource,
    note:   Option<String>,
}

/// Register all application routes on a new router with shared state.
pub fn routes(service: ApplicationService) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(create_application, list_applications))
        .routes(routes!(get_application, update_application, delete_application))
        .routes(routes!(transition_status))
        .routes(routes!(get_status_history))
        .with_state(service)
}

/// Create a new application.
#[utoipa::path(
    post,
    path = "/api/v1/applications",
    tag = "applications",
    request_body = CreateApplicationRequest,
    responses(
        (status = 201, description = "Application created", body = Application),
    )
)]
#[instrument(skip(service, req))]
async fn create_application(
    State(service): State<ApplicationService>,
    Json(req): Json<CreateApplicationRequest>,
) -> Result<(StatusCode, Json<Application>), ApplicationError> {
    let app = service.create_application(req).await?;
    Ok((StatusCode::CREATED, Json(app)))
}

/// List applications with optional filters.
#[utoipa::path(
    get,
    path = "/api/v1/applications",
    tag = "applications",
    params(
        ("status" = Option<ApplicationStatus>, Query, description = "Filter by status"),
        ("channel" = Option<ApplicationChannel>, Query, description = "Filter by channel"),
        ("priority" = Option<Priority>, Query, description = "Filter by priority"),
    ),
    responses(
        (status = 200, description = "List of applications", body = Vec<Application>),
    )
)]
#[instrument(skip(service))]
async fn list_applications(
    State(service): State<ApplicationService>,
    Query(filter): Query<ApplicationFilter>,
) -> Result<Json<Vec<Application>>, ApplicationError> {
    let apps = service.list_applications(&filter).await?;
    Ok(Json(apps))
}

/// Get a single application by ID.
#[utoipa::path(
    get,
    path = "/api/v1/applications/{id}",
    tag = "applications",
    params(("id" = Uuid, Path, description = "Application ID")),
    responses(
        (status = 200, description = "Application found", body = Application),
        (status = 404, description = "Application not found"),
    )
)]
#[instrument(skip(service))]
async fn get_application(
    State(service): State<ApplicationService>,
    Path(id): Path<Uuid>,
) -> Result<Json<Application>, ApplicationError> {
    let app = service.get_application(ApplicationId::from(id)).await?;
    Ok(Json(app))
}

/// Update an existing application.
#[utoipa::path(
    put,
    path = "/api/v1/applications/{id}",
    tag = "applications",
    params(("id" = Uuid, Path, description = "Application ID")),
    request_body = UpdateApplicationRequest,
    responses(
        (status = 200, description = "Application updated", body = Application),
    )
)]
#[instrument(skip(service, req))]
async fn update_application(
    State(service): State<ApplicationService>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateApplicationRequest>,
) -> Result<Json<Application>, ApplicationError> {
    let app = service
        .update_application(ApplicationId::from(id), req)
        .await?;
    Ok(Json(app))
}

/// Transition application status.
#[utoipa::path(
    post,
    path = "/api/v1/applications/{id}/transition",
    tag = "applications",
    params(("id" = Uuid, Path, description = "Application ID")),
    request_body = TransitionRequest,
    responses(
        (status = 200, description = "Status transitioned", body = Application),
        (status = 409, description = "Invalid transition"),
    )
)]
#[instrument(skip(service, body))]
async fn transition_status(
    State(service): State<ApplicationService>,
    Path(id): Path<Uuid>,
    Json(body): Json<TransitionRequest>,
) -> Result<Json<Application>, ApplicationError> {
    let app = service
        .transition_status(ApplicationId::from(id), body.status, body.source, body.note)
        .await?;
    Ok(Json(app))
}

/// Get status change history for an application.
#[utoipa::path(
    get,
    path = "/api/v1/applications/{id}/history",
    tag = "applications",
    params(("id" = Uuid, Path, description = "Application ID")),
    responses(
        (status = 200, description = "Status history", body = Vec<StatusChangeRecord>),
    )
)]
#[instrument(skip(service))]
async fn get_status_history(
    State(service): State<ApplicationService>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<StatusChangeRecord>>, ApplicationError> {
    let history = service.get_status_history(ApplicationId::from(id)).await?;
    Ok(Json(history))
}

/// Delete an application.
#[utoipa::path(
    delete,
    path = "/api/v1/applications/{id}",
    tag = "applications",
    params(("id" = Uuid, Path, description = "Application ID")),
    responses(
        (status = 204, description = "Application deleted"),
    )
)]
#[instrument(skip(service))]
async fn delete_application(
    State(service): State<ApplicationService>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApplicationError> {
    service.delete_application(ApplicationId::from(id)).await?;
    Ok(StatusCode::NO_CONTENT)
}
