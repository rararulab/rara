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

//! HTTP routes for the job pipeline service.

use axum::{
    Json,
    extract::{Path, Query, State},
    response::sse::{Event, KeepAlive, Sse},
    routing::{get, patch, post},
};
use futures::stream::StreamExt;
use serde::{Deserialize, Serialize};
use utoipa_axum::router::OpenApiRouter;
use uuid::Uuid;

use super::{
    pg_repository::PgPipelineRepository,
    repository::PipelineRepository,
    service::{PipelineError, PipelineService},
    types::{
        DiscoveredJob, DiscoveredJobAction, DiscoveredJobWithDetails, DiscoveredJobsStats,
        PipelineEvent, PipelineRun,
    },
};

/// Build `/api/v1/pipeline/...` routes.
pub fn routes(service: PipelineService) -> OpenApiRouter {
    OpenApiRouter::new()
        .route("/api/v1/pipeline/run", post(trigger_run))
        .route("/api/v1/pipeline/cancel", post(cancel_run))
        .route("/api/v1/pipeline/status", get(get_status))
        .route("/api/v1/pipeline/stream", get(stream_events))
        .route("/api/v1/pipeline/runs", get(list_runs))
        .route("/api/v1/pipeline/runs/{id}", get(get_run))
        .route("/api/v1/pipeline/runs/{id}/events", get(get_run_events))
        .route("/api/v1/pipeline/runs/{id}/jobs", get(get_run_jobs))
        .route(
            "/api/v1/pipeline/discovered-jobs",
            get(list_discovered_jobs),
        )
        .route(
            "/api/v1/pipeline/discovered-jobs/stats",
            get(get_discovered_jobs_stats),
        )
        .route(
            "/api/v1/pipeline/discovered-jobs/{id}",
            patch(update_discovered_job_action),
        )
        .with_state(service)
}

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, utoipa::ToSchema)]
struct PipelineActionResponse {
    message: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
struct PipelineStatusResponse {
    running: bool,
}

#[derive(Debug, Deserialize)]
struct ListRunsQuery {
    limit:  Option<i64>,
    offset: Option<i64>,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `POST /api/v1/pipeline/run` -- trigger a pipeline run.
async fn trigger_run(
    State(service): State<PipelineService>,
) -> Result<Json<PipelineActionResponse>, PipelineError> {
    service.run().await?;
    Ok(Json(PipelineActionResponse {
        message: "Pipeline run started".to_owned(),
    }))
}

/// `POST /api/v1/pipeline/cancel` -- cancel a running pipeline.
async fn cancel_run(
    State(service): State<PipelineService>,
) -> Result<Json<PipelineActionResponse>, PipelineError> {
    service.cancel()?;
    Ok(Json(PipelineActionResponse {
        message: "Pipeline cancellation requested".to_owned(),
    }))
}

/// `GET /api/v1/pipeline/status` -- check pipeline status.
async fn get_status(State(service): State<PipelineService>) -> Json<PipelineStatusResponse> {
    service.reconcile_stale_runs_if_needed().await;
    Json(PipelineStatusResponse {
        running: service.is_running(),
    })
}

/// `GET /api/v1/pipeline/stream` -- SSE stream of pipeline events.
async fn stream_events(
    State(service): State<PipelineService>,
) -> Sse<impl futures::Stream<Item = Result<Event, axum::Error>>> {
    let rx = service.subscribe();
    let stream = tokio_stream::wrappers::BroadcastStream::new(rx).filter_map(|result| async move {
        match result {
            Ok(event) => {
                let name = event.event_type_name();
                let data = serde_json::to_string(&event).unwrap_or_default();
                Some(Ok::<_, axum::Error>(
                    Event::default().event(name).data(data),
                ))
            }
            Err(_) => None, // lagged, skip
        }
    });
    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(std::time::Duration::from_secs(15))
            .text("keep-alive"),
    )
}

/// `GET /api/v1/pipeline/runs` -- list historical pipeline runs.
async fn list_runs(
    State(service): State<PipelineService>,
    Query(q): Query<ListRunsQuery>,
) -> Result<Json<Vec<PipelineRun>>, PipelineError> {
    service.reconcile_stale_runs_if_needed().await;
    let repo = PgPipelineRepository::new(service.pool());
    let runs = repo
        .list_runs(q.limit.unwrap_or(20), q.offset.unwrap_or(0))
        .await
        .map_err(|e| PipelineError::RunFailed {
            message: e.to_string(),
        })?;
    Ok(Json(runs))
}

/// `GET /api/v1/pipeline/runs/{id}` -- get a single pipeline run.
async fn get_run(
    State(service): State<PipelineService>,
    Path(id): Path<Uuid>,
) -> Result<Json<PipelineRun>, PipelineError> {
    service.reconcile_stale_runs_if_needed().await;
    let repo = PgPipelineRepository::new(service.pool());
    let run = repo
        .get_run(id)
        .await
        .map_err(|e| PipelineError::RunFailed {
            message: e.to_string(),
        })?
        .ok_or_else(|| PipelineError::RunFailed {
            message: format!("run not found: {id}"),
        })?;
    Ok(Json(run))
}

/// `GET /api/v1/pipeline/runs/{id}/events` -- get events for a pipeline run.
async fn get_run_events(
    State(service): State<PipelineService>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<PipelineEvent>>, PipelineError> {
    let repo = PgPipelineRepository::new(service.pool());
    let events = repo
        .get_events(id)
        .await
        .map_err(|e| PipelineError::RunFailed {
            message: e.to_string(),
        })?;
    Ok(Json(events))
}

/// `GET /api/v1/pipeline/runs/{id}/jobs` -- get discovered jobs for a pipeline
/// run.
async fn get_run_jobs(
    State(service): State<PipelineService>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<DiscoveredJobWithDetails>>, PipelineError> {
    let repo = PgPipelineRepository::new(service.pool());
    let jobs = repo
        .list_discovered_jobs(id)
        .await
        .map_err(|e| PipelineError::RunFailed {
            message: e.to_string(),
        })?;
    Ok(Json(jobs))
}

// ---------------------------------------------------------------------------
// Discovered Jobs (global view)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct ListDiscoveredJobsQuery {
    action:    Option<String>,
    min_score: Option<i32>,
    max_score: Option<i32>,
    run_id:    Option<Uuid>,
    sort_by:   Option<String>,
    limit:     Option<i64>,
    offset:    Option<i64>,
}

#[derive(Debug, Serialize)]
struct PaginatedDiscoveredJobs {
    items:  Vec<DiscoveredJobWithDetails>,
    total:  i64,
    limit:  i64,
    offset: i64,
}

#[derive(Debug, Deserialize)]
struct UpdateDiscoveredJobActionRequest {
    action: String,
}

fn parse_action(s: &str) -> Result<DiscoveredJobAction, PipelineError> {
    match s {
        "discovered" => Ok(DiscoveredJobAction::Discovered),
        "notified" => Ok(DiscoveredJobAction::Notified),
        "applied" => Ok(DiscoveredJobAction::Applied),
        "skipped" => Ok(DiscoveredJobAction::Skipped),
        other => Err(PipelineError::RunFailed {
            message: format!(
                "invalid action: {other} (expected discovered|notified|applied|skipped)"
            ),
        }),
    }
}

/// `GET /api/v1/pipeline/discovered-jobs` -- list all discovered jobs with
/// filters.
async fn list_discovered_jobs(
    State(service): State<PipelineService>,
    Query(q): Query<ListDiscoveredJobsQuery>,
) -> Result<Json<PaginatedDiscoveredJobs>, PipelineError> {
    let action = q.action.as_deref().map(parse_action).transpose()?;
    let limit = q.limit.unwrap_or(20);
    let offset = q.offset.unwrap_or(0);

    let repo = PgPipelineRepository::new(service.pool());
    let items = repo
        .list_all_discovered_jobs(
            action,
            q.min_score,
            q.max_score,
            q.run_id,
            q.sort_by.as_deref(),
            limit,
            offset,
        )
        .await
        .map_err(|e| PipelineError::RunFailed {
            message: e.to_string(),
        })?;

    let total = repo
        .count_discovered_jobs(action, q.min_score, q.max_score, q.run_id)
        .await
        .map_err(|e| PipelineError::RunFailed {
            message: e.to_string(),
        })?;

    Ok(Json(PaginatedDiscoveredJobs {
        items,
        total,
        limit,
        offset,
    }))
}

/// `GET /api/v1/pipeline/discovered-jobs/stats` -- aggregated stats.
async fn get_discovered_jobs_stats(
    State(service): State<PipelineService>,
) -> Result<Json<DiscoveredJobsStats>, PipelineError> {
    let repo = PgPipelineRepository::new(service.pool());
    let stats = repo
        .discovered_jobs_stats()
        .await
        .map_err(|e| PipelineError::RunFailed {
            message: e.to_string(),
        })?;
    Ok(Json(stats))
}

/// `PATCH /api/v1/pipeline/discovered-jobs/{id}` -- update action on a
/// discovered job.
async fn update_discovered_job_action(
    State(service): State<PipelineService>,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateDiscoveredJobActionRequest>,
) -> Result<Json<DiscoveredJob>, PipelineError> {
    let action = parse_action(&body.action)?;
    let repo = PgPipelineRepository::new(service.pool());
    let job = repo
        .update_discovered_job_score_action(id, None, Some(action))
        .await
        .map_err(|e| PipelineError::RunFailed {
            message: e.to_string(),
        })?
        .ok_or_else(|| PipelineError::RunFailed {
            message: format!("discovered job not found: {id}"),
        })?;
    Ok(Json(job))
}
