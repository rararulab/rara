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
    routing::{get, post},
};
use futures::stream::StreamExt;
use serde::{Deserialize, Serialize};
use utoipa_axum::router::OpenApiRouter;
use uuid::Uuid;

use crate::pg_repository::PgPipelineRepository;
use crate::repository::PipelineRepository;
use crate::service::{PipelineError, PipelineService};
use crate::types::{PipelineEvent, PipelineRun};

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
    limit: Option<i64>,
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
    Json(PipelineStatusResponse {
        running: service.is_running(),
    })
}

/// `GET /api/v1/pipeline/stream` -- SSE stream of pipeline events.
async fn stream_events(
    State(service): State<PipelineService>,
) -> Sse<impl futures::Stream<Item = Result<Event, axum::Error>>> {
    let rx = service.subscribe();
    let stream =
        tokio_stream::wrappers::BroadcastStream::new(rx).filter_map(|result| async move {
            match result {
                Ok(event) => {
                    let name = event.event_type_name();
                    let data = serde_json::to_string(&event).unwrap_or_default();
                    Some(Ok::<_, axum::Error>(Event::default().event(name).data(data)))
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
