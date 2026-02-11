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

//! HTTP API routes for saved job management.

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{delete, get, post},
};
use tracing::instrument;
use uuid::Uuid;

use crate::{
    error::SavedJobError,
    service::SavedJobService,
    types::{CreateSavedJobRequest, PipelineEvent, SavedJob, SavedJobFilter, SavedJobStatus},
};

/// Register all saved-job routes on a new router with shared state.
pub fn routes(service: Arc<SavedJobService>) -> Router {
    Router::new()
        .route("/api/v1/saved-jobs", post(create_saved_job))
        .route("/api/v1/saved-jobs", get(list_saved_jobs))
        .route("/api/v1/saved-jobs/{id}", get(get_saved_job))
        .route("/api/v1/saved-jobs/{id}", delete(delete_saved_job))
        .route("/api/v1/saved-jobs/{id}/retry", post(retry_saved_job))
        .route("/api/v1/saved-jobs/{id}/events", get(list_saved_job_events))
        .with_state(service)
}

#[instrument(skip(service, req))]
async fn create_saved_job(
    State(service): State<Arc<SavedJobService>>,
    Json(req): Json<CreateSavedJobRequest>,
) -> Result<(StatusCode, Json<SavedJob>), SavedJobError> {
    let saved_job = service.create(&req.url).await?;
    Ok((StatusCode::CREATED, Json(saved_job)))
}

#[instrument(skip(service))]
async fn list_saved_jobs(
    State(service): State<Arc<SavedJobService>>,
    Query(filter): Query<SavedJobFilter>,
) -> Result<Json<Vec<SavedJob>>, SavedJobError> {
    let status = filter.status.and_then(|s| parse_status(&s));
    let jobs = service.list(status).await?;
    Ok(Json(jobs))
}

#[instrument(skip(service))]
async fn get_saved_job(
    State(service): State<Arc<SavedJobService>>,
    Path(id): Path<Uuid>,
) -> Result<Json<SavedJob>, SavedJobError> {
    let job = service
        .get(id)
        .await?
        .ok_or(SavedJobError::NotFound { id })?;
    Ok(Json(job))
}

#[instrument(skip(service))]
async fn delete_saved_job(
    State(service): State<Arc<SavedJobService>>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, SavedJobError> {
    service.delete(id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[instrument(skip(service))]
async fn retry_saved_job(
    State(service): State<Arc<SavedJobService>>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, SavedJobError> {
    service.retry(id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[instrument(skip(service))]
async fn list_saved_job_events(
    State(service): State<Arc<SavedJobService>>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<PipelineEvent>>, SavedJobError> {
    let events = service.list_events(id).await?;
    Ok(Json(events))
}

/// Parse a status string (e.g. "analyzed") into a `SavedJobStatus`.
fn parse_status(s: &str) -> Option<SavedJobStatus> {
    match s {
        "pending_crawl" => Some(SavedJobStatus::PendingCrawl),
        "crawling" => Some(SavedJobStatus::Crawling),
        "crawled" => Some(SavedJobStatus::Crawled),
        "analyzing" => Some(SavedJobStatus::Analyzing),
        "analyzed" => Some(SavedJobStatus::Analyzed),
        "failed" => Some(SavedJobStatus::Failed),
        "expired" => Some(SavedJobStatus::Expired),
        _ => None,
    }
}
