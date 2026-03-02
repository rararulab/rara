// Copyright 2025 Rararulab
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

//! HTTP API routes for job discovery.

use std::collections::HashSet;

use axum::{Json, extract::State, http::StatusCode};
use utoipa_axum::{router::OpenApiRouter, routes};

use super::{
    error::SourceError,
    service::JobService,
    types::{DiscoveryCriteria, DiscoveryJobResponse},
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
            tracing::warn!(%err, "discover returning error -- no jobs collected");
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
