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

use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State},
    routing::{get, post},
};
use rara_kernel::dispatcher::{
    AgentDispatcher, AgentTaskKind, DispatcherStatus, TaskRecord, TaskStatus,
    log_store::LogFilter,
};
use serde::{Deserialize, Serialize};

pub fn dispatcher_router(dispatcher: Arc<AgentDispatcher>) -> axum::Router {
    axum::Router::new()
        .route("/api/dispatcher/status", get(get_status))
        .route("/api/dispatcher/history", get(get_history))
        .route("/api/dispatcher/cancel/{id}", post(cancel_task))
        .with_state(dispatcher)
}

async fn get_status(State(dispatcher): State<Arc<AgentDispatcher>>) -> Json<DispatcherStatus> {
    Json(dispatcher.status().await)
}

#[derive(Deserialize)]
struct HistoryParams {
    limit:  Option<usize>,
    kind:   Option<String>,
    status: Option<String>,
    since:  Option<String>,
}

async fn get_history(
    State(dispatcher): State<Arc<AgentDispatcher>>,
    Query(params): Query<HistoryParams>,
) -> Json<Vec<TaskRecord>> {
    let kind = params.kind.and_then(|k| match k.as_str() {
        "proactive" => Some(AgentTaskKind::Proactive),
        "scheduled" => Some(AgentTaskKind::Scheduled {
            job_id: String::new(),
        }),
        "pipeline" => Some(AgentTaskKind::Pipeline),
        _ => None,
    });

    let status = params.status.and_then(|s| match s.as_str() {
        "queued" => Some(TaskStatus::Queued),
        "running" => Some(TaskStatus::Running),
        "completed" => Some(TaskStatus::Completed),
        "error" => Some(TaskStatus::Error),
        "cancelled" => Some(TaskStatus::Cancelled),
        "deduped" => Some(TaskStatus::Deduped),
        _ => None,
    });

    let since = params.since.and_then(|s| s.parse::<jiff::Timestamp>().ok());

    let filter = LogFilter {
        limit: params.limit.unwrap_or(50),
        kind,
        status,
        since,
    };

    Json(dispatcher.history(filter).await)
}

#[derive(Serialize)]
struct CancelResponse {
    success: bool,
}

async fn cancel_task(
    State(dispatcher): State<Arc<AgentDispatcher>>,
    Path(id): Path<String>,
) -> Json<CancelResponse> {
    let success = dispatcher.cancel(&id).await.is_ok();
    Json(CancelResponse { success })
}
