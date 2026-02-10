//! HTTP API routes for notification management.

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{get, post},
};
use serde::Deserialize;
use tracing::instrument;
use uuid::Uuid;

use crate::{
    error::NotifyError,
    service::NotificationService,
    types::{
        Notification, NotificationChannel, NotificationFilter, NotificationStatistics,
        NotificationStatus,
    },
};

#[derive(Debug, Deserialize)]
pub struct NotificationListQuery {
    pub channel:        Option<String>,
    pub status:         Option<String>,
    pub created_after:  Option<String>,
    pub created_before: Option<String>,
}

/// Register all notification routes on a new router with shared state.
pub fn routes(service: Arc<NotificationService>) -> Router {
    Router::new()
        .route("/api/v1/notifications", get(list_notifications))
        .route("/api/v1/notifications/stats", get(get_statistics))
        .route("/api/v1/notifications/{id}", get(get_notification))
        .route(
            "/api/v1/notifications/{id}/retry",
            post(retry_notification),
        )
        .with_state(service)
}

#[instrument(skip(service))]
async fn list_notifications(
    State(service): State<Arc<NotificationService>>,
    Query(query): Query<NotificationListQuery>,
) -> Result<Json<Vec<Notification>>, NotifyError> {
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

    let notifications = service.list(&filter).await?;
    Ok(Json(notifications))
}

#[instrument(skip(service))]
async fn get_statistics(
    State(service): State<Arc<NotificationService>>,
) -> Result<Json<NotificationStatistics>, NotifyError> {
    let stats = service.get_statistics().await?;
    Ok(Json(stats))
}

#[instrument(skip(service))]
async fn get_notification(
    State(service): State<Arc<NotificationService>>,
    Path(id): Path<Uuid>,
) -> Result<Json<Notification>, NotifyError> {
    let notification = service.get_by_id(id).await?.ok_or(NotifyError::NotFound { id })?;
    Ok(Json(notification))
}

#[instrument(skip(service))]
async fn retry_notification(
    State(service): State<Arc<NotificationService>>,
    Path(id): Path<Uuid>,
) -> Result<Json<Notification>, NotifyError> {
    let notification = service.retry(id).await?;
    Ok(Json(notification))
}

fn parse_channel(s: &str) -> Option<NotificationChannel> {
    match s.to_lowercase().as_str() {
        "telegram" => Some(NotificationChannel::Telegram),
        "email" => Some(NotificationChannel::Email),
        "webhook" => Some(NotificationChannel::Webhook),
        _ => None,
    }
}

fn parse_status(s: &str) -> Option<NotificationStatus> {
    match s.to_lowercase().as_str() {
        "pending" => Some(NotificationStatus::Pending),
        "sent" => Some(NotificationStatus::Sent),
        "failed" => Some(NotificationStatus::Failed),
        "retrying" => Some(NotificationStatus::Retrying),
        _ => None,
    }
}
