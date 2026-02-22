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
    routing::get,
};
use opendal::Operator;
use tracing::instrument;
use utoipa_axum::{router::OpenApiRouter, routes};
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
        .routes(routes!(discover_jobs))
        .with_state(service)
}

#[utoipa::path(
    post,
    path = "/api/v1/jobs/discover",
    tag = "jobs",
    request_body = DiscoveryCriteria,
    responses(
        (status = 200, description = "Discovered jobs", body = Vec<DiscoveryJobResponse>),
    )
)]
#[tracing::instrument(skip(service, criteria), fields(keywords = ?criteria.keywords))]
async fn discover_jobs(
    State(service): State<JobService>,
    Json(criteria): Json<DiscoveryCriteria>,
) -> Result<(StatusCode, Json<Vec<DiscoveryJobResponse>>), SourceError> {
    tracing::info!("starting job discovery (all sources)");

    let existing_source_keys = HashSet::new();
    let existing_fuzzy_keys = HashSet::new();
    let result = service
        .discover_all(&criteria, &existing_source_keys, &existing_fuzzy_keys)
        .await;

    tracing::info!(
        job_count = result.jobs.len(),
        has_error = result.error.is_some(),
        "discover result received from drivers"
    );

    // If all drivers failed and no jobs were collected, propagate the error.
    if result.jobs.is_empty() {
        if let Some(err) = result.error {
            tracing::warn!(%err, "discover returning error — no jobs collected");
            return Err(err);
        }
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
        .routes(routes!(create_saved_job, list_saved_jobs))
        .routes(routes!(get_saved_job, delete_saved_job))
        .routes(routes!(retry_saved_job))
        .route("/api/v1/saved-jobs/{id}/markdown", get(get_saved_job_markdown))
        .routes(routes!(list_saved_job_events))
        // Bot internal
        .routes(routes!(parse_jd_from_bot))
        .with_state(state)
}

#[derive(Clone)]
struct ManagementState {
    service:      JobService,
    object_store: Operator,
}

// -- Tracker handlers -------------------------------------------------------

#[utoipa::path(
    post,
    path = "/api/v1/saved-jobs",
    tag = "saved-jobs",
    request_body = CreateSavedJobRequest,
    responses(
        (status = 201, description = "Saved job created", body = SavedJob),
    )
)]
#[instrument(skip(state, req))]
async fn create_saved_job(
    State(state): State<ManagementState>,
    Json(req): Json<CreateSavedJobRequest>,
) -> Result<(StatusCode, Json<SavedJob>), SavedJobError> {
    let saved_job = state.service.create(&req.url).await?;
    Ok((StatusCode::CREATED, Json(saved_job)))
}

#[utoipa::path(
    get,
    path = "/api/v1/saved-jobs",
    tag = "saved-jobs",
    params(
        ("status" = Option<String>, Query, description = "Filter by pipeline status name"),
    ),
    responses(
        (status = 200, description = "List of saved jobs", body = Vec<SavedJob>),
    )
)]
#[instrument(skip(state))]
async fn list_saved_jobs(
    State(state): State<ManagementState>,
    Query(filter): Query<SavedJobFilter>,
) -> Result<Json<Vec<SavedJob>>, SavedJobError> {
    let status = filter.status.and_then(|s| s.parse().ok());
    let jobs = state.service.list(status).await?;
    Ok(Json(jobs))
}

#[utoipa::path(
    get,
    path = "/api/v1/saved-jobs/{id}",
    tag = "saved-jobs",
    params(("id" = Uuid, Path, description = "Saved job ID")),
    responses(
        (status = 200, description = "Saved job found", body = SavedJob),
        (status = 404, description = "Saved job not found"),
    )
)]
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

#[utoipa::path(
    delete,
    path = "/api/v1/saved-jobs/{id}",
    tag = "saved-jobs",
    params(("id" = Uuid, Path, description = "Saved job ID")),
    responses(
        (status = 204, description = "Saved job deleted"),
    )
)]
#[instrument(skip(state))]
async fn delete_saved_job(
    State(state): State<ManagementState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, SavedJobError> {
    state.service.delete(id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/api/v1/saved-jobs/{id}/retry",
    tag = "saved-jobs",
    params(("id" = Uuid, Path, description = "Saved job ID")),
    responses(
        (status = 204, description = "Retry queued"),
    )
)]
#[instrument(skip(state))]
async fn retry_saved_job(
    State(state): State<ManagementState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, SavedJobError> {
    state.service.retry(id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    get,
    path = "/api/v1/saved-jobs/{id}/events",
    tag = "saved-jobs",
    params(("id" = Uuid, Path, description = "Saved job ID")),
    responses(
        (status = 200, description = "Pipeline events", body = Vec<PipelineEvent>),
    )
)]
#[instrument(skip(state))]
async fn list_saved_job_events(
    State(state): State<ManagementState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<PipelineEvent>>, SavedJobError> {
    let events = state.service.list_events(id).await?;
    Ok(Json(events))
}

#[utoipa::path(
    get,
    path = "/api/v1/saved-jobs/{id}/markdown",
    tag = "saved-jobs",
    params(("id" = Uuid, Path, description = "Saved job ID")),
    responses(
        (status = 200, description = "Markdown content", content_type = "text/markdown"),
    )
)]
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

#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
struct BotJdParseRequest {
    text: String,
}

#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
struct BotJdParseResponse {
    id:      Uuid,
    title:   String,
    company: String,
}

#[utoipa::path(
    post,
    path = "/api/v1/internal/bot/jd-parse",
    tag = "internal",
    request_body = BotJdParseRequest,
    responses(
        (status = 200, description = "Parsed job description", body = BotJdParseResponse),
    )
)]
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
