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

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{delete, get, post, put},
};
use job_domain_application::types::{
    Application, ApplicationFilter, ChangeSource, CreateApplicationRequest, StatusChangeRecord,
    UpdateApplicationRequest,
};
use job_domain_core::{id::ApplicationId, status::ApplicationStatus};
use job_domain_resume::repository::ResumeRepository;
use serde::Deserialize;
use uuid::Uuid;

use crate::{api::error::ApiError, state::AppState};

/// Register all application routes on a new router with shared state.
pub fn application_routes<R: ResumeRepository + 'static>(state: Arc<AppState<R>>) -> Router {
    Router::new()
        .route("/api/v1/applications", post(create_application::<R>))
        .route("/api/v1/applications", get(list_applications::<R>))
        .route("/api/v1/applications/{id}", get(get_application::<R>))
        .route("/api/v1/applications/{id}", put(update_application::<R>))
        .route(
            "/api/v1/applications/{id}/transition",
            post(transition_status::<R>),
        )
        .route(
            "/api/v1/applications/{id}/history",
            get(get_status_history::<R>),
        )
        .route("/api/v1/applications/{id}", delete(delete_application::<R>))
        .with_state(state)
}

/// JSON body for the status transition endpoint.
#[derive(Debug, Deserialize)]
struct TransitionRequest {
    /// The target status.
    status: ApplicationStatus,
    /// Who or what is triggering the change.
    source: ChangeSource,
    /// Optional note describing the reason.
    note:   Option<String>,
}

/// POST /api/v1/applications
async fn create_application<R: ResumeRepository + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Json(req): Json<CreateApplicationRequest>,
) -> Result<(StatusCode, Json<Application>), ApiError> {
    let app = state.application_service.create_application(req).await?;
    Ok((StatusCode::CREATED, Json(app)))
}

/// GET /api/v1/applications
async fn list_applications<R: ResumeRepository + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Query(filter): Query<ApplicationFilter>,
) -> Result<Json<Vec<Application>>, ApiError> {
    let apps = state.application_service.list_applications(&filter).await?;
    Ok(Json(apps))
}

/// GET /api/v1/applications/:id
async fn get_application<R: ResumeRepository + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Path(id): Path<Uuid>,
) -> Result<Json<Application>, ApiError> {
    let app = state
        .application_service
        .get_application(ApplicationId::from(id))
        .await?;
    Ok(Json(app))
}

/// PUT /api/v1/applications/:id
async fn update_application<R: ResumeRepository + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateApplicationRequest>,
) -> Result<Json<Application>, ApiError> {
    let app = state
        .application_service
        .update_application(ApplicationId::from(id), req)
        .await?;
    Ok(Json(app))
}

/// POST /api/v1/applications/:id/transition
async fn transition_status<R: ResumeRepository + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Path(id): Path<Uuid>,
    Json(body): Json<TransitionRequest>,
) -> Result<Json<Application>, ApiError> {
    let app = state
        .application_service
        .transition_status(ApplicationId::from(id), body.status, body.source, body.note)
        .await?;
    Ok(Json(app))
}

/// GET /api/v1/applications/:id/history
async fn get_status_history<R: ResumeRepository + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<StatusChangeRecord>>, ApiError> {
    let history = state
        .application_service
        .get_status_history(ApplicationId::from(id))
        .await?;
    Ok(Json(history))
}

/// DELETE /api/v1/applications/:id
async fn delete_application<R: ResumeRepository + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    state
        .application_service
        .delete_application(ApplicationId::from(id))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
