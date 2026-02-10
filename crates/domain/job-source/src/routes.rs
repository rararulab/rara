//! HTTP API routes for job source discovery.

use std::{collections::HashSet, sync::Arc};

use axum::{Json, Router, extract::State, http::StatusCode, routing::post};

use crate::{
    err::SourceError,
    service::JobSourceService,
    types::{DiscoveryCriteria, DiscoveryJobResponse},
};

/// Register all job source routes on a new router with shared state.
pub fn routes(service: Arc<JobSourceService>) -> Router {
    Router::new()
        .route("/api/v1/jobs/discover", post(discover_jobs))
        .with_state(service)
}

#[tracing::instrument(skip(service, criteria), fields(keywords = ?criteria.keywords))]
async fn discover_jobs(
    State(service): State<Arc<JobSourceService>>,
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
