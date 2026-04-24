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

//! HTTP handlers for the scheduler admin API.
//!
//! | Method | Path                                     | Description                      |
//! |--------|------------------------------------------|----------------------------------|
//! | GET    | `/api/v1/scheduler/jobs`                 | list jobs across all sessions    |
//! | GET    | `/api/v1/scheduler/jobs/{id}`            | get a single job                 |
//! | DELETE | `/api/v1/scheduler/jobs/{id}`            | remove a job                     |
//! | POST   | `/api/v1/scheduler/jobs/{id}/trigger`    | fire a job without advancing     |
//! | GET    | `/api/v1/scheduler/jobs/{id}/history`    | paged execution history          |
//!
//! No `POST /jobs` route — creation is intentionally agent-tool-only so
//! every scheduled job stays attributable to a real session principal.

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use rara_kernel::{handle::KernelHandle, schedule::JobId};
use serde::Deserialize;

use super::{
    dto::{JobResultView, JobView, TriggerJobView},
    service::{SchedulerError, SchedulerSvc},
};
use crate::kernel::problem::ProblemDetails;

/// Default history page size when `?limit=` is omitted.
const HISTORY_LIMIT_DEFAULT: usize = 50;
/// Hard cap on history page size — prevents the admin from paging a huge
/// result set in one request.
const HISTORY_LIMIT_MAX: usize = 200;

/// Build the `/api/v1/scheduler/*` router.
///
/// Auth is applied by the upstream backend-admin layer — this router
/// trusts its caller is already authenticated and authorised.
pub fn scheduler_routes(handle: KernelHandle) -> Router {
    let svc = SchedulerSvc::new(handle);
    Router::new()
        .route("/api/v1/scheduler/jobs", get(list_jobs))
        .route(
            "/api/v1/scheduler/jobs/{id}",
            get(get_job).delete(delete_job),
        )
        .route("/api/v1/scheduler/jobs/{id}/trigger", post(trigger_job))
        .route("/api/v1/scheduler/jobs/{id}/history", get(job_history))
        .with_state(svc)
}

// ---------------------------------------------------------------------------
// Request types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct HistoryQuery {
    /// Requested page size — clamped to `[1, HISTORY_LIMIT_MAX]` and
    /// defaulted to [`HISTORY_LIMIT_DEFAULT`] when absent.
    limit: Option<usize>,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn list_jobs(State(svc): State<SchedulerSvc>) -> Json<Vec<JobView>> {
    Json(svc.list_jobs().await)
}

async fn get_job(
    State(svc): State<SchedulerSvc>,
    Path(id): Path<String>,
) -> Result<Json<JobView>, ProblemDetails> {
    let job_id = parse_job_id(&id)?;
    svc.get_job(&job_id).await.map(Json).map_err(into_problem)
}

async fn delete_job(
    State(svc): State<SchedulerSvc>,
    Path(id): Path<String>,
) -> Result<StatusCode, ProblemDetails> {
    let job_id = parse_job_id(&id)?;
    svc.delete_job(&job_id).await.map_err(into_problem)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn trigger_job(
    State(svc): State<SchedulerSvc>,
    Path(id): Path<String>,
) -> Result<Json<TriggerJobView>, ProblemDetails> {
    let job_id = parse_job_id(&id)?;
    svc.trigger_job(&job_id)
        .await
        .map(Json)
        .map_err(into_problem)
}

async fn job_history(
    State(svc): State<SchedulerSvc>,
    Path(id): Path<String>,
    Query(q): Query<HistoryQuery>,
) -> Result<Json<Vec<JobResultView>>, ProblemDetails> {
    let job_id = parse_job_id(&id)?;
    let limit = clamp_history_limit(q.limit);
    Ok(Json(svc.history(&job_id, limit).await))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Parse a stringified [`JobId`] or produce a 400 Problem.
fn parse_job_id(raw: &str) -> Result<JobId, ProblemDetails> {
    JobId::try_from_raw(raw)
        .map_err(|e| ProblemDetails::bad_request(format!("invalid job id: {e}")))
}

/// Translate service errors into RFC 9457 problem responses.
///
/// `JobNotFound` → 404 so the admin UI can render a clean empty state
/// instead of an opaque 500.
fn into_problem(err: SchedulerError) -> ProblemDetails {
    match err {
        SchedulerError::JobNotFound { ref job_id } => {
            ProblemDetails::not_found("Job Not Found", format!("no job with id: {job_id}"))
        }
        other => ProblemDetails::internal(other.to_string()),
    }
}

/// Apply the documented default + clamp rules to a history-limit query
/// parameter. Extracted so callers and tests share one definition.
fn clamp_history_limit(raw: Option<usize>) -> usize {
    raw.unwrap_or(HISTORY_LIMIT_DEFAULT)
        .clamp(1, HISTORY_LIMIT_MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_limit_defaults_when_absent() {
        assert_eq!(clamp_history_limit(None), HISTORY_LIMIT_DEFAULT);
    }

    #[test]
    fn history_limit_is_clamped_high() {
        assert_eq!(clamp_history_limit(Some(10_000)), HISTORY_LIMIT_MAX);
    }

    #[test]
    fn history_limit_is_clamped_low() {
        // 0 is nonsensical — admin UIs that accidentally send it still
        // get at least one row rather than an empty page masquerading as
        // "no history."
        assert_eq!(clamp_history_limit(Some(0)), 1);
    }

    #[test]
    fn history_limit_passes_through_valid_values() {
        assert_eq!(clamp_history_limit(Some(25)), 25);
        assert_eq!(
            clamp_history_limit(Some(HISTORY_LIMIT_MAX)),
            HISTORY_LIMIT_MAX
        );
    }
}
