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

//! HTTP API routes for job discovery and bot integration.

use std::collections::HashSet;

use axum::{
    Json,
    extract::State,
    http::StatusCode,
};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::{
    error::SourceError,
    service::JobService,
    types::{DiscoveryCriteria, DiscoveryJobResponse, NormalizedJob},
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
// Bot internal routes
// ===========================================================================

/// Bot internal routes (JD parsing).
pub fn bot_routes(service: JobService) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(parse_jd_from_bot))
        .with_state(service)
}

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
    State(service): State<JobService>,
    Json(req): Json<BotJdParseRequest>,
) -> Result<(StatusCode, Json<BotJdParseResponse>), (StatusCode, String)> {
    if req.text.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "text must not be empty".to_owned()));
    }

    let saved: NormalizedJob = service.parse_jd(&req.text).await.map_err(|e| {
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
