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

use axum::{Json, extract::State, http::StatusCode, routing::{get, post}};
use serde::Serialize;
use utoipa_axum::router::OpenApiRouter;

use crate::service::PipelineService;

/// Build `/api/v1/pipeline/...` routes.
pub fn routes(service: PipelineService) -> OpenApiRouter {
    OpenApiRouter::new()
        .route("/api/v1/pipeline/run", post(trigger_run))
        .route("/api/v1/pipeline/cancel", post(cancel_run))
        .route("/api/v1/pipeline/status", get(get_status))
        .with_state(service)
}

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, utoipa::ToSchema)]
struct PipelineActionResponse {
    status:  String,
    message: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
struct PipelineStatusResponse {
    running: bool,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `POST /api/v1/pipeline/run` -- trigger a pipeline run.
async fn trigger_run(
    State(service): State<PipelineService>,
) -> Result<Json<PipelineActionResponse>, (StatusCode, Json<PipelineActionResponse>)> {
    match service.run().await {
        Ok(()) => Ok(Json(PipelineActionResponse {
            status:  "ok".to_owned(),
            message: "Pipeline run started".to_owned(),
        })),
        Err(e) => {
            let status_code = match &e {
                crate::service::PipelineError::AlreadyRunning => StatusCode::CONFLICT,
                crate::service::PipelineError::AiNotConfigured => {
                    StatusCode::PRECONDITION_FAILED
                }
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };
            Err((
                status_code,
                Json(PipelineActionResponse {
                    status:  "error".to_owned(),
                    message: e.to_string(),
                }),
            ))
        }
    }
}

/// `POST /api/v1/pipeline/cancel` -- cancel a running pipeline.
async fn cancel_run(
    State(service): State<PipelineService>,
) -> Result<Json<PipelineActionResponse>, (StatusCode, Json<PipelineActionResponse>)> {
    match service.cancel() {
        Ok(()) => Ok(Json(PipelineActionResponse {
            status:  "ok".to_owned(),
            message: "Pipeline cancellation requested".to_owned(),
        })),
        Err(e) => Err((
            StatusCode::CONFLICT,
            Json(PipelineActionResponse {
                status:  "error".to_owned(),
                message: e.to_string(),
            }),
        )),
    }
}

/// `GET /api/v1/pipeline/status` -- check pipeline status.
async fn get_status(State(service): State<PipelineService>) -> Json<PipelineStatusResponse> {
    Json(PipelineStatusResponse {
        running: service.is_running(),
    })
}
