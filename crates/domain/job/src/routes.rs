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

//! HTTP API routes for job discovery and saved job management.

use std::collections::HashSet;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{delete, get, post},
};
use tracing::instrument;
use uuid::Uuid;

use crate::{
    discovery_service::JobSourceService,
    error::{SavedJobError, SourceError},
    tracker_service::SavedJobService,
    types::{
        CreateSavedJobRequest, DiscoveryCriteria, DiscoveryJobResponse, PipelineEvent, SavedJob,
        SavedJobFilter, SavedJobStatus,
    },
};

// ===========================================================================
// Discovery routes
// ===========================================================================

/// Register all job source discovery routes on a new router with shared state.
pub fn discovery_routes(service: JobSourceService) -> Router {
    Router::new()
        .route("/api/v1/jobs/discover", post(discover_jobs))
        .with_state(service)
}

#[tracing::instrument(skip(service, criteria), fields(keywords = ?criteria.keywords))]
async fn discover_jobs(
    State(service): State<JobSourceService>,
    Json(criteria): Json<DiscoveryCriteria>,
) -> Result<(StatusCode, Json<Vec<DiscoveryJobResponse>>), SourceError> {
    tracing::info!("starting job discovery");
    // JobSourceService::discover() is synchronous (calls Python via PyO3),
    // so we wrap it in spawn_blocking to avoid blocking the async runtime.
    let result = tokio::task::spawn_blocking(move || {
        let existing_source_keys = HashSet::new();
        let existing_fuzzy_keys = HashSet::new();
        service.discover(&criteria, &existing_source_keys, &existing_fuzzy_keys)
    })
    .await
    .map_err(|e| SourceError::NonRetryable {
        source_name: "system".to_owned(),
        message:     format!("task join error: {e}"),
    })?;

    tracing::info!(
        job_count = result.jobs.len(),
        has_error = result.error.is_some(),
        "discover result received from driver"
    );

    // If the driver encountered an error, propagate it.
    if let Some(ref err) = result.error {
        tracing::warn!(%err, "discover returning error from driver");
        return Err(result.error.unwrap());
    }

    let job_count = result.jobs.len();
    let response = result
        .jobs
        .into_iter()
        .map(DiscoveryJobResponse::from)
        .collect();

    tracing::info!(job_count, "job discovery complete");
    Ok((StatusCode::OK, Json(response)))
}

// ===========================================================================
// Tracker routes
// ===========================================================================

/// Register all saved-job routes on a new router with shared state.
pub fn tracker_routes(service: SavedJobService) -> Router {
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
    State(service): State<SavedJobService>,
    Json(req): Json<CreateSavedJobRequest>,
) -> Result<(StatusCode, Json<SavedJob>), SavedJobError> {
    let saved_job = service.create(&req.url).await?;
    Ok((StatusCode::CREATED, Json(saved_job)))
}

#[instrument(skip(service))]
async fn list_saved_jobs(
    State(service): State<SavedJobService>,
    Query(filter): Query<SavedJobFilter>,
) -> Result<Json<Vec<SavedJob>>, SavedJobError> {
    let status = filter.status.and_then(|s| parse_status(&s));
    let jobs = service.list(status).await?;
    Ok(Json(jobs))
}

#[instrument(skip(service))]
async fn get_saved_job(
    State(service): State<SavedJobService>,
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
    State(service): State<SavedJobService>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, SavedJobError> {
    service.delete(id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[instrument(skip(service))]
async fn retry_saved_job(
    State(service): State<SavedJobService>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, SavedJobError> {
    service.retry(id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[instrument(skip(service))]
async fn list_saved_job_events(
    State(service): State<SavedJobService>,
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
