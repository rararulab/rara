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

//! HTTP API routes for notification management.

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{get, post},
};
use job_domain_notify::types::{
    Notification, NotificationChannel, NotificationFilter, NotificationStatistics,
    NotificationStatus,
};
use job_domain_resume::repository::ResumeRepository;
use serde::Deserialize;
use uuid::Uuid;

use crate::{api::error::ApiError, state::AppState};

/// Register all notification routes on a new router with shared state.
pub fn notification_routes<R: ResumeRepository + 'static>(state: Arc<AppState<R>>) -> Router {
    Router::new()
        .route("/api/v1/notifications", get(list_notifications::<R>))
        .route("/api/v1/notifications/stats", get(get_statistics::<R>))
        .route("/api/v1/notifications/{id}", get(get_notification::<R>))
        .route(
            "/api/v1/notifications/{id}/retry",
            post(retry_notification::<R>),
        )
        .with_state(state)
}

/// Query parameters for listing notifications.
#[derive(Debug, Deserialize)]
pub struct NotificationListQuery {
    pub channel:        Option<String>,
    pub status:         Option<String>,
    pub created_after:  Option<String>,
    pub created_before: Option<String>,
}

/// GET /api/v1/notifications
async fn list_notifications<R: ResumeRepository + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Query(query): Query<NotificationListQuery>,
) -> Result<Json<Vec<Notification>>, ApiError> {
    let channel = query.channel.and_then(|c| parse_channel(&c));
    let status = query.status.and_then(|s| parse_status(&s));
    let created_after = query
        .created_after
        .and_then(|s| s.parse::<jiff::Timestamp>().ok());
    let created_before = query
        .created_before
        .and_then(|s| s.parse::<jiff::Timestamp>().ok());

    let filter = NotificationFilter {
        channel,
        status,
        recipient: None,
        reference_type: None,
        reference_id: None,
        created_after,
        created_before,
    };

    let notifications = state.notification_service.list(&filter).await?;
    Ok(Json(notifications))
}

/// GET /api/v1/notifications/stats
async fn get_statistics<R: ResumeRepository + 'static>(
    State(state): State<Arc<AppState<R>>>,
) -> Result<Json<NotificationStatistics>, ApiError> {
    let stats = state.notification_service.get_statistics().await?;
    Ok(Json(stats))
}

/// GET /api/v1/notifications/:id
async fn get_notification<R: ResumeRepository + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Path(id): Path<Uuid>,
) -> Result<Json<Notification>, ApiError> {
    let notification = state
        .notification_service
        .get_by_id(id)
        .await?
        .ok_or_else(|| ApiError {
            status:  axum::http::StatusCode::NOT_FOUND,
            message: format!("notification not found: {id}"),
        })?;
    Ok(Json(notification))
}

/// POST /api/v1/notifications/:id/retry
async fn retry_notification<R: ResumeRepository + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Path(id): Path<Uuid>,
) -> Result<Json<Notification>, ApiError> {
    let notification = state.notification_service.retry(id).await?;
    Ok(Json(notification))
}

/// Parse a channel string into a `NotificationChannel` enum value.
fn parse_channel(s: &str) -> Option<NotificationChannel> {
    match s.to_lowercase().as_str() {
        "telegram" => Some(NotificationChannel::Telegram),
        "email" => Some(NotificationChannel::Email),
        "webhook" => Some(NotificationChannel::Webhook),
        _ => None,
    }
}

/// Parse a status string into a `NotificationStatus` enum value.
fn parse_status(s: &str) -> Option<NotificationStatus> {
    match s.to_lowercase().as_str() {
        "pending" => Some(NotificationStatus::Pending),
        "sent" => Some(NotificationStatus::Sent),
        "failed" => Some(NotificationStatus::Failed),
        "retrying" => Some(NotificationStatus::Retrying),
        _ => None,
    }
}
