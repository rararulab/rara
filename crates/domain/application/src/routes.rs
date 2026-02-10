//! HTTP API routes for application lifecycle management.

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{delete, get, post, put},
};
use job_domain_shared::id::ApplicationId;
use serde::Deserialize;
use tracing::instrument;
use uuid::Uuid;

use crate::{
    error::ApplicationError,
    service::ApplicationService,
    types::{
        Application, ApplicationFilter, ApplicationStatus, ChangeSource,
        CreateApplicationRequest, StatusChangeRecord, UpdateApplicationRequest,
    },
};

/// JSON body for the status transition endpoint.
#[derive(Debug, Deserialize)]
struct TransitionRequest {
    status: ApplicationStatus,
    source: ChangeSource,
    note:   Option<String>,
}

/// Register all application routes on a new router with shared state.
pub fn routes(service: Arc<ApplicationService>) -> Router {
    Router::new()
        .route("/api/v1/applications", post(create_application))
        .route("/api/v1/applications", get(list_applications))
        .route("/api/v1/applications/{id}", get(get_application))
        .route("/api/v1/applications/{id}", put(update_application))
        .route(
            "/api/v1/applications/{id}/transition",
            post(transition_status),
        )
        .route(
            "/api/v1/applications/{id}/history",
            get(get_status_history),
        )
        .route("/api/v1/applications/{id}", delete(delete_application))
        .with_state(service)
}

#[instrument(skip(service, req))]
async fn create_application(
    State(service): State<Arc<ApplicationService>>,
    Json(req): Json<CreateApplicationRequest>,
) -> Result<(StatusCode, Json<Application>), ApplicationError> {
    let app = service.create_application(req).await?;
    Ok((StatusCode::CREATED, Json(app)))
}

#[instrument(skip(service))]
async fn list_applications(
    State(service): State<Arc<ApplicationService>>,
    Query(filter): Query<ApplicationFilter>,
) -> Result<Json<Vec<Application>>, ApplicationError> {
    let apps = service.list_applications(&filter).await?;
    Ok(Json(apps))
}

#[instrument(skip(service))]
async fn get_application(
    State(service): State<Arc<ApplicationService>>,
    Path(id): Path<Uuid>,
) -> Result<Json<Application>, ApplicationError> {
    let app = service.get_application(ApplicationId::from(id)).await?;
    Ok(Json(app))
}

#[instrument(skip(service, req))]
async fn update_application(
    State(service): State<Arc<ApplicationService>>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateApplicationRequest>,
) -> Result<Json<Application>, ApplicationError> {
    let app = service
        .update_application(ApplicationId::from(id), req)
        .await?;
    Ok(Json(app))
}

#[instrument(skip(service, body))]
async fn transition_status(
    State(service): State<Arc<ApplicationService>>,
    Path(id): Path<Uuid>,
    Json(body): Json<TransitionRequest>,
) -> Result<Json<Application>, ApplicationError> {
    let app = service
        .transition_status(ApplicationId::from(id), body.status, body.source, body.note)
        .await?;
    Ok(Json(app))
}

#[instrument(skip(service))]
async fn get_status_history(
    State(service): State<Arc<ApplicationService>>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<StatusChangeRecord>>, ApplicationError> {
    let history = service
        .get_status_history(ApplicationId::from(id))
        .await?;
    Ok(Json(history))
}

#[instrument(skip(service))]
async fn delete_application(
    State(service): State<Arc<ApplicationService>>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApplicationError> {
    service.delete_application(ApplicationId::from(id)).await?;
    Ok(StatusCode::NO_CONTENT)
}
