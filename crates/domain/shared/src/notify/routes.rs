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

//! HTTP routes for notification queue observability.

use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
};
use utoipa_axum::router::OpenApiRouter;
use serde::Deserialize;

use crate::notify::{
    client::NotifyClient,
    error::NotifyError,
    types::{NotificationQueueMessage, NotificationQueueOverview, QueueMessageState},
};

const DEFAULT_LIMIT: i64 = 50;
const MAX_LIMIT: i64 = 200;

#[derive(Clone)]
struct RouteState {
    client: NotifyClient,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct QueueMessagesResponse {
    pub state:  QueueMessageState,
    pub limit:  i64,
    pub offset: i64,
    pub items:  Vec<NotificationQueueMessage>,
}

#[derive(Debug, Deserialize)]
struct QueueMessagesQuery {
    state:  Option<String>,
    limit:  Option<i64>,
    offset: Option<i64>,
}

/// Build notification observability routes.
pub fn routes(client: NotifyClient) -> OpenApiRouter {
    OpenApiRouter::new()
        .route(
            "/api/v1/notifications/queues/telegram/overview",
            axum::routing::get(get_telegram_queue_overview),
        )
        .route(
            "/api/v1/notifications/queues/telegram/messages",
            axum::routing::get(list_telegram_queue_messages),
        )
        .with_state(RouteState { client })
}

async fn get_telegram_queue_overview(
    State(state): State<RouteState>,
) -> Result<Json<NotificationQueueOverview>, (StatusCode, String)> {
    let overview = state
        .client
        .telegram_overview()
        .await
        .map_err(internal_err)?;
    Ok(Json(overview))
}

async fn list_telegram_queue_messages(
    State(state): State<RouteState>,
    Query(query): Query<QueueMessagesQuery>,
) -> Result<Json<QueueMessagesResponse>, (StatusCode, String)> {
    let state_filter = parse_state(query.state.as_deref())?;
    let limit = query.limit.unwrap_or(DEFAULT_LIMIT).clamp(1_i64, MAX_LIMIT);
    let offset = query.offset.unwrap_or(0_i64).max(0_i64);

    let items = state
        .client
        .list_telegram_messages(state_filter, limit, offset)
        .await
        .map_err(internal_err)?;

    Ok(Json(QueueMessagesResponse {
        state: state_filter,
        limit,
        offset,
        items,
    }))
}

fn parse_state(input: Option<&str>) -> Result<QueueMessageState, (StatusCode, String)> {
    match input.unwrap_or("ready") {
        "ready" => Ok(QueueMessageState::Ready),
        "inflight" => Ok(QueueMessageState::Inflight),
        "archived" => Ok(QueueMessageState::Archived),
        other => Err((
            StatusCode::BAD_REQUEST,
            format!("invalid state '{other}', expected ready|inflight|archived"),
        )),
    }
}

fn internal_err(err: NotifyError) -> (StatusCode, String) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("notification queue query failed: {err}"),
    )
}
