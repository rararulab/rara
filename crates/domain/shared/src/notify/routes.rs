// Copyright 2026 Crrow
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

use axum::{Json, Router, extract::Query, extract::State, http::StatusCode};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};

use crate::notify::client::TELEGRAM_NOTIFY_QUEUE_NAME;

const TELEGRAM_QUEUE_TABLE: &str = "pgmq.q_notification_telegram_dispatch";
const TELEGRAM_ARCHIVE_TABLE: &str = "pgmq.a_notification_telegram_dispatch";
const DEFAULT_LIMIT: i64 = 50;
const MAX_LIMIT: i64 = 200;

#[derive(Clone)]
struct RouteState {
    pool: PgPool,
}

#[derive(Debug, Clone, Serialize)]
pub struct QueueOverviewResponse {
    pub queue_name:     String,
    pub ready_count:    i64,
    pub inflight_count: i64,
    pub archived_count: i64,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum QueueMessageState {
    Ready,
    Inflight,
    Archived,
}

#[derive(Debug, Clone, Serialize)]
pub struct QueueMessageView {
    pub state:       QueueMessageState,
    pub msg_id:      i64,
    pub read_ct:     i32,
    pub enqueued_at: DateTime<Utc>,
    pub vt:          DateTime<Utc>,
    pub archived_at: Option<DateTime<Utc>>,
    pub payload:     serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct QueueMessagesResponse {
    pub state:  QueueMessageState,
    pub limit:  i64,
    pub offset: i64,
    pub items:  Vec<QueueMessageView>,
}

#[derive(Debug, Deserialize)]
struct QueueMessagesQuery {
    state:  Option<String>,
    limit:  Option<i64>,
    offset: Option<i64>,
}

/// Build notification observability routes.
pub fn routes(pool: PgPool) -> Router {
    Router::new()
        .route(
            "/api/v1/notifications/queues/telegram/overview",
            axum::routing::get(get_telegram_queue_overview),
        )
        .route(
            "/api/v1/notifications/queues/telegram/messages",
            axum::routing::get(list_telegram_queue_messages),
        )
        .with_state(RouteState { pool })
}

async fn get_telegram_queue_overview(
    State(state): State<RouteState>,
) -> Result<Json<QueueOverviewResponse>, (StatusCode, String)> {
    let ready_count = sqlx::query_scalar::<_, i64>(&format!(
        "SELECT COUNT(*) FROM {TELEGRAM_QUEUE_TABLE} WHERE vt <= now()"
    ))
    .fetch_one(&state.pool)
    .await
    .map_err(internal_err)?;

    let inflight_count = sqlx::query_scalar::<_, i64>(&format!(
        "SELECT COUNT(*) FROM {TELEGRAM_QUEUE_TABLE} WHERE vt > now()"
    ))
    .fetch_one(&state.pool)
    .await
    .map_err(internal_err)?;

    let archived_count = sqlx::query_scalar::<_, i64>(&format!(
        "SELECT COUNT(*) FROM {TELEGRAM_ARCHIVE_TABLE}"
    ))
    .fetch_one(&state.pool)
    .await
    .map_err(internal_err)?;

    Ok(Json(QueueOverviewResponse {
        queue_name: TELEGRAM_NOTIFY_QUEUE_NAME.to_owned(),
        ready_count,
        inflight_count,
        archived_count,
    }))
}

async fn list_telegram_queue_messages(
    State(state): State<RouteState>,
    Query(query): Query<QueueMessagesQuery>,
) -> Result<Json<QueueMessagesResponse>, (StatusCode, String)> {
    let state_filter = parse_state(query.state.as_deref())?;
    let limit = query
        .limit
        .unwrap_or(DEFAULT_LIMIT)
        .clamp(1_i64, MAX_LIMIT);
    let offset = query.offset.unwrap_or(0_i64).max(0_i64);

    let items = match state_filter {
        QueueMessageState::Ready => {
            let rows = sqlx::query(&format!(
                "SELECT msg_id, read_ct, enqueued_at, vt, message \
                 FROM {TELEGRAM_QUEUE_TABLE} \
                 WHERE vt <= now() \
                 ORDER BY msg_id DESC \
                 LIMIT $1 OFFSET $2"
            ))
            .bind(limit)
            .bind(offset)
            .fetch_all(&state.pool)
            .await
            .map_err(internal_err)?;

            rows.into_iter()
                .map(|row| queue_row_to_view(row, QueueMessageState::Ready))
                .collect::<Result<Vec<_>, _>>()
                .map_err(internal_err)?
        }
        QueueMessageState::Inflight => {
            let rows = sqlx::query(&format!(
                "SELECT msg_id, read_ct, enqueued_at, vt, message \
                 FROM {TELEGRAM_QUEUE_TABLE} \
                 WHERE vt > now() \
                 ORDER BY msg_id DESC \
                 LIMIT $1 OFFSET $2"
            ))
            .bind(limit)
            .bind(offset)
            .fetch_all(&state.pool)
            .await
            .map_err(internal_err)?;

            rows.into_iter()
                .map(|row| queue_row_to_view(row, QueueMessageState::Inflight))
                .collect::<Result<Vec<_>, _>>()
                .map_err(internal_err)?
        }
        QueueMessageState::Archived => {
            let rows = sqlx::query(&format!(
                "SELECT msg_id, read_ct, enqueued_at, vt, archived_at, message \
                 FROM {TELEGRAM_ARCHIVE_TABLE} \
                 ORDER BY msg_id DESC \
                 LIMIT $1 OFFSET $2"
            ))
            .bind(limit)
            .bind(offset)
            .fetch_all(&state.pool)
            .await
            .map_err(internal_err)?;

            rows.into_iter()
                .map(archive_row_to_view)
                .collect::<Result<Vec<_>, _>>()
                .map_err(internal_err)?
        }
    };

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

fn internal_err(err: sqlx::Error) -> (StatusCode, String) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("notification queue query failed: {err}"),
    )
}

fn queue_row_to_view(row: sqlx::postgres::PgRow, state: QueueMessageState) -> Result<QueueMessageView, sqlx::Error> {
    Ok(QueueMessageView {
        state,
        msg_id: row.try_get("msg_id")?,
        read_ct: row.try_get("read_ct")?,
        enqueued_at: row.try_get("enqueued_at")?,
        vt: row.try_get("vt")?,
        archived_at: None,
        payload: row.try_get("message")?,
    })
}

fn archive_row_to_view(row: sqlx::postgres::PgRow) -> Result<QueueMessageView, sqlx::Error> {
    Ok(QueueMessageView {
        state: QueueMessageState::Archived,
        msg_id: row.try_get("msg_id")?,
        read_ct: row.try_get("read_ct")?,
        enqueued_at: row.try_get("enqueued_at")?,
        vt: row.try_get("vt")?,
        archived_at: row.try_get("archived_at")?,
        payload: row.try_get("message")?,
    })
}
