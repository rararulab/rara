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

//! HTTP API routes for job discovery, saved-job management, and bot
//! integration.

use std::collections::HashSet;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::{StatusCode, header},
    response::IntoResponse,
    routing::{delete, get, post},
};
use utoipa_axum::router::OpenApiRouter;
use opendal::Operator;
use tracing::instrument;
use uuid::Uuid;

use crate::{
    error::{SavedJobError, SourceError},
    service::JobService,
    types::{
        CreateSavedJobRequest, DiscoveryCriteria, DiscoveryJobResponse, NormalizedJob,
        PipelineEvent, SavedJob, SavedJobFilter,
    },
};

// ===========================================================================
// Discovery routes
// ===========================================================================

/// Discovery routes (caller may apply DedupLayer).
pub fn discovery_routes(service: JobService) -> OpenApiRouter {
    OpenApiRouter::new()
        .route("/api/v1/jobs/discover", post(discover_jobs))
        .with_state(service)
}

#[tracing::instrument(skip(service, criteria), fields(keywords = ?criteria.keywords))]
async fn discover_jobs(
    State(service): State<JobService>,
    Json(criteria): Json<DiscoveryCriteria>,
) -> Result<(StatusCode, Json<Vec<DiscoveryJobResponse>>), SourceError> {
    tracing::info!("starting job discovery");
    // JobService::discover() is synchronous (calls Python via PyO3),
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
// Management routes (tracker + bot internal)
// ===========================================================================

/// Saved-job CRUD and bot internal routes.
pub fn management_routes(service: JobService, object_store: Operator) -> OpenApiRouter {
    let state = ManagementState {
        service,
        object_store,
    };
    OpenApiRouter::new()
        // Tracker
        .route("/api/v1/saved-jobs", post(create_saved_job))
        .route("/api/v1/saved-jobs", get(list_saved_jobs))
        .route("/api/v1/saved-jobs/{id}", get(get_saved_job))
        .route("/api/v1/saved-jobs/{id}", delete(delete_saved_job))
        .route("/api/v1/saved-jobs/{id}/retry", post(retry_saved_job))
        .route("/api/v1/saved-jobs/{id}/markdown", get(get_saved_job_markdown))
        .route(
            "/api/v1/saved-jobs/{id}/events",
            get(list_saved_job_events),
        )
        // Bot internal
        .route("/api/v1/internal/bot/jd-parse", post(parse_jd_from_bot))
        .with_state(state)
}

#[derive(Clone)]
struct ManagementState {
    service:      JobService,
    object_store: Operator,
}

// -- Tracker handlers -------------------------------------------------------

#[instrument(skip(state, req))]
async fn create_saved_job(
    State(state): State<ManagementState>,
    Json(req): Json<CreateSavedJobRequest>,
) -> Result<(StatusCode, Json<SavedJob>), SavedJobError> {
    let saved_job = state.service.create(&req.url).await?;
    Ok((StatusCode::CREATED, Json(saved_job)))
}

#[instrument(skip(state))]
async fn list_saved_jobs(
    State(state): State<ManagementState>,
    Query(filter): Query<SavedJobFilter>,
) -> Result<Json<Vec<SavedJob>>, SavedJobError> {
    let status = filter.status.and_then(|s| s.parse().ok());
    let jobs = state.service.list(status).await?;
    Ok(Json(jobs))
}

#[instrument(skip(state))]
async fn get_saved_job(
    State(state): State<ManagementState>,
    Path(id): Path<Uuid>,
) -> Result<Json<SavedJob>, SavedJobError> {
    let job = state
        .service
        .get(id)
        .await?
        .ok_or(SavedJobError::NotFound { id })?;
    Ok(Json(job))
}

#[instrument(skip(state))]
async fn delete_saved_job(
    State(state): State<ManagementState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, SavedJobError> {
    state.service.delete(id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[instrument(skip(state))]
async fn retry_saved_job(
    State(state): State<ManagementState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, SavedJobError> {
    state.service.retry(id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[instrument(skip(state))]
async fn list_saved_job_events(
    State(state): State<ManagementState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<PipelineEvent>>, SavedJobError> {
    let events = state.service.list_events(id).await?;
    Ok(Json(events))
}

#[instrument(skip(state))]
async fn get_saved_job_markdown(
    State(state): State<ManagementState>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, SavedJobError> {
    let job = state
        .service
        .get(id)
        .await?
        .ok_or(SavedJobError::NotFound { id })?;
    let s3_key = job
        .markdown_s3_key
        .ok_or_else(|| SavedJobError::ValidationError {
            message: "saved job has no markdown object key".to_owned(),
        })?;

    let data =
        state
            .object_store
            .read(&s3_key)
            .await
            .map_err(|e| SavedJobError::ObjectStoreError {
                message: format!("failed to read markdown object {s3_key}: {e}"),
            })?;
    let markdown = String::from_utf8_lossy(data.to_bytes().as_ref()).to_string();

    Ok((
        [(header::CONTENT_TYPE, "text/markdown; charset=utf-8")],
        markdown,
    ))
}

// -- Bot internal handler ---------------------------------------------------

#[derive(Debug, serde::Deserialize)]
struct BotJdParseRequest {
    text: String,
}

#[derive(Debug, serde::Serialize)]
struct BotJdParseResponse {
    id:      Uuid,
    title:   String,
    company: String,
}

async fn parse_jd_from_bot(
    State(state): State<ManagementState>,
    Json(req): Json<BotJdParseRequest>,
) -> Result<(StatusCode, Json<BotJdParseResponse>), (StatusCode, String)> {
    if req.text.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "text must not be empty".to_owned()));
    }

    let saved: NormalizedJob = state.service.parse_jd(&req.text).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to parse jd: {e}"),
        )
    })?;

    Ok((
        StatusCode::OK,
        Json(BotJdParseResponse {
            id:      saved.id,
            title:   saved.title,
            company: saved.company,
        }),
    ))
}
