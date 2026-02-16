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

//! Read-only HTTP routes for viewing agent-scheduled jobs.

use std::sync::Arc;

use axum::{Json, extract::State, routing::get};
use utoipa_axum::router::OpenApiRouter;

use crate::agent_scheduler::{AgentJob, AgentScheduler};

/// Build `/api/v1/agent-scheduler/...` routes.
pub fn routes(scheduler: Arc<AgentScheduler>) -> OpenApiRouter {
    OpenApiRouter::new()
        .route("/api/v1/agent-scheduler/jobs", get(list_jobs))
        .with_state(scheduler)
}

/// `GET /api/v1/agent-scheduler/jobs` — list all agent-scheduled jobs.
async fn list_jobs(State(scheduler): State<Arc<AgentScheduler>>) -> Json<Vec<AgentJob>> {
    Json(scheduler.list().await)
}
