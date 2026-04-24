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

//! Data Feed REST API handlers.
//!
//! | Method | Path                                        | Description         |
//! |--------|---------------------------------------------|---------------------|
//! | GET    | `/api/v1/data-feeds`                        | list all feeds      |
//! | POST   | `/api/v1/data-feeds`                        | create feed         |
//! | GET    | `/api/v1/data-feeds/{id}`                   | get feed detail     |
//! | PUT/PATCH | `/api/v1/data-feeds/{id}`                | partial update feed |
//! | DELETE | `/api/v1/data-feeds/{id}`                   | delete feed         |
//! | PUT    | `/api/v1/data-feeds/{id}/toggle`             | enable/disable feed |
//! | GET    | `/api/v1/data-feeds/{id}/events`             | query feed events   |
//! | GET    | `/api/v1/data-feeds/{id}/events/{event_id}` | get single event    |
//!
//! All mutations synchronise both the database (via [`DataFeedSvc`]) and the
//! in-memory [`DataFeedRegistry`]. When a polling-type feed is created and
//! enabled, a background task is spawned automatically.

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, put},
};
use jiff::Timestamp;
use rara_kernel::data_feed::{
    DataFeed, DataFeedConfig, DataFeedRegistry, FeedStatus, FeedType, parse_duration_ago,
    polling::PollingSource,
};
use serde::{Deserialize, Deserializer, Serialize};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};
use uuid::Uuid;

use super::service::DataFeedSvc;
use crate::kernel::problem::ProblemDetails;

/// Shared state for data feed routes.
///
/// Contains both the persistence service and the in-memory registry so that
/// mutations can synchronise both layers.
#[derive(Clone)]
pub struct DataFeedRouterState {
    /// Persistence service for feed configs and events.
    pub svc:      DataFeedSvc,
    /// In-memory registry (also holds cancellation tokens for running tasks).
    pub registry: Arc<DataFeedRegistry>,
}

/// Build the `/api/v1/data-feeds/...` router.
pub fn data_feed_routes(state: DataFeedRouterState) -> Router {
    Router::new()
        .route("/api/v1/data-feeds", get(list_feeds).post(create_feed))
        .route(
            "/api/v1/data-feeds/{id}",
            get(get_feed)
                .put(update_feed)
                .patch(update_feed)
                .delete(delete_feed),
        )
        .route("/api/v1/data-feeds/{id}/toggle", put(toggle_feed))
        .route("/api/v1/data-feeds/{id}/events", get(query_events))
        .route("/api/v1/data-feeds/{id}/events/{event_id}", get(get_event))
        .with_state(state)
}

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

/// Request body for creating a new data feed.
#[derive(Debug, Deserialize)]
struct CreateFeedRequest {
    name:      String,
    feed_type: FeedType,
    tags:      Vec<String>,
    transport: serde_json::Value,
    auth:      Option<serde_json::Value>,
}

/// Request body for updating an existing data feed.
///
/// All fields are optional — only supplied fields are updated, the rest
/// keep their current values (partial update / PATCH semantics).
#[derive(Debug, Deserialize)]
struct UpdateFeedRequest {
    name:      Option<String>,
    feed_type: Option<FeedType>,
    tags:      Option<Vec<String>>,
    transport: Option<serde_json::Value>,
    /// Pass `null` to clear auth, omit the field to keep existing auth.
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    auth:      Option<Option<serde_json::Value>>,
}

/// Deserialize a double-`Option` field so that:
/// - field absent   → outer `None`  (keep existing value)
/// - field is `null` → `Some(None)` (explicitly clear)
/// - field has value → `Some(Some(v))`
fn deserialize_optional_field<'de, T, D>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: Deserializer<'de>,
{
    Ok(Some(Option::deserialize(deserializer)?))
}

/// Query parameters for event listing.
#[derive(Debug, Deserialize)]
struct EventQueryParams {
    /// Duration string: `"1h"`, `"24h"`, `"7d"`, etc.
    since:  Option<String>,
    /// Maximum events to return (default: 50, max: 200).
    limit:  Option<i64>,
    /// Offset for pagination (default: 0).
    offset: Option<i64>,
}

/// Paginated event response.
#[derive(Debug, Serialize)]
struct EventListResponse {
    events:   Vec<rara_kernel::data_feed::FeedEvent>,
    total:    i64,
    has_more: bool,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /api/v1/data-feeds` — list all feeds with runtime status.
async fn list_feeds(
    State(state): State<DataFeedRouterState>,
) -> Result<Json<Vec<DataFeedConfig>>, ProblemDetails> {
    let feeds = state
        .svc
        .list_feeds()
        .await
        .map_err(|e| ProblemDetails::internal(e.to_string()))?;
    Ok(Json(feeds))
}

/// `POST /api/v1/data-feeds` — create a new feed, sync registry, start task.
async fn create_feed(
    State(state): State<DataFeedRouterState>,
    Json(body): Json<CreateFeedRequest>,
) -> Result<(StatusCode, Json<DataFeedConfig>), ProblemDetails> {
    let auth = body
        .auth
        .map(serde_json::from_value)
        .transpose()
        .map_err(|e| ProblemDetails::bad_request(format!("invalid auth config: {e}")))?;

    let now = Timestamp::now();
    let config = DataFeedConfig::builder()
        .id(Uuid::new_v4().to_string())
        .name(body.name)
        .feed_type(body.feed_type)
        .tags(body.tags)
        .transport(body.transport)
        .maybe_auth(auth)
        .enabled(true)
        .status(FeedStatus::Idle)
        .created_at(now)
        .updated_at(now)
        .build();

    // 1. Persist to database.
    state
        .svc
        .create_feed(&config)
        .await
        .map_err(|e| ProblemDetails::internal(e.to_string()))?;

    // 2. Sync to in-memory registry.
    if let Err(e) = state.registry.register(config.clone()) {
        warn!(name = %config.name, error = %e, "registry sync failed on create");
    }

    // 3. Start feed task if enabled.
    if config.enabled {
        start_feed_task(&config, &state.registry);
    }

    info!(name = %config.name, "data feed created via admin API");
    Ok((StatusCode::CREATED, Json(config)))
}

/// `GET /api/v1/data-feeds/{id}` — get a single feed.
async fn get_feed(
    State(state): State<DataFeedRouterState>,
    Path(id): Path<String>,
) -> Result<Json<DataFeedConfig>, ProblemDetails> {
    let feed = state
        .svc
        .get_feed(&id)
        .await
        .map_err(|e| ProblemDetails::internal(e.to_string()))?
        .ok_or_else(|| {
            ProblemDetails::not_found("Feed Not Found", format!("no feed with id: {id}"))
        })?;
    Ok(Json(feed))
}

/// `PUT /api/v1/data-feeds/{id}` — partial update of an existing feed.
///
/// Only fields present in the request body are changed; omitted fields
/// keep their current values.
async fn update_feed(
    State(state): State<DataFeedRouterState>,
    Path(id): Path<String>,
    Json(body): Json<UpdateFeedRequest>,
) -> Result<Json<DataFeedConfig>, ProblemDetails> {
    let existing = state
        .svc
        .get_feed(&id)
        .await
        .map_err(|e| ProblemDetails::internal(e.to_string()))?
        .ok_or_else(|| {
            ProblemDetails::not_found("Feed Not Found", format!("no feed with id: {id}"))
        })?;

    // Merge: supplied field wins, otherwise keep existing.
    let new_name = body.name.unwrap_or(existing.name.clone());

    let auth = match body.auth {
        // Field omitted → keep existing auth.
        None => existing.auth.clone(),
        // Explicit `null` → clear auth.
        Some(None) => None,
        // New value → parse and replace.
        Some(Some(v)) => Some(
            serde_json::from_value(v)
                .map_err(|e| ProblemDetails::bad_request(format!("invalid auth config: {e}")))?,
        ),
    };

    let updated = DataFeedConfig::builder()
        .id(id)
        .name(new_name.clone())
        .feed_type(body.feed_type.unwrap_or(existing.feed_type))
        .tags(body.tags.unwrap_or(existing.tags))
        .transport(body.transport.unwrap_or(existing.transport))
        .maybe_auth(auth)
        .enabled(existing.enabled)
        .status(existing.status)
        .maybe_last_error(existing.last_error)
        .created_at(existing.created_at)
        .updated_at(Timestamp::now())
        .build();

    // 1. Persist to database.
    state
        .svc
        .update_feed(&updated)
        .await
        .map_err(|e| ProblemDetails::internal(e.to_string()))?;

    // 2. Sync registry: remove old entry (cancels running task), re-register.
    let _ = state.registry.remove(&existing.name);
    if let Err(e) = state.registry.register(updated.clone()) {
        warn!(name = %new_name, error = %e, "registry sync failed on update");
    }

    // 3. Restart feed task if enabled.
    if updated.enabled {
        start_feed_task(&updated, &state.registry);
    }

    Ok(Json(updated))
}

/// `DELETE /api/v1/data-feeds/{id}` — stop task, remove from registry and DB.
async fn delete_feed(
    State(state): State<DataFeedRouterState>,
    axum::Extension(principal): axum::Extension<
        rara_kernel::identity::Principal<rara_kernel::identity::Resolved>,
    >,
    Path(id): Path<String>,
) -> Result<StatusCode, ProblemDetails> {
    // Destructive operation — require admin and audit the acting principal.
    if !principal.is_admin() {
        return Err(ProblemDetails::forbidden(
            "deleting data feeds requires admin role",
        ));
    }
    info!(
        actor = %principal.user_id,
        feed_id = %id,
        "delete_feed"
    );

    // Look up the feed name for registry removal.
    let feed = state.svc.get_feed(&id).await.ok().flatten();

    // 1. Remove from registry (cancels running task if any).
    if let Some(ref f) = feed {
        if let Err(e) = state.registry.remove(&f.name) {
            warn!(name = %f.name, error = %e, "feed not found in registry during delete");
        }
    }

    // 2. Delete from database.
    let deleted = state
        .svc
        .delete_feed(&id)
        .await
        .map_err(|e| ProblemDetails::internal(e.to_string()))?;

    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ProblemDetails::not_found(
            "Feed Not Found",
            format!("no feed with id: {id}"),
        ))
    }
}

/// `PUT /api/v1/data-feeds/{id}/toggle` — flip the enabled state (no body
/// needed).
async fn toggle_feed(
    State(state): State<DataFeedRouterState>,
    Path(id): Path<String>,
) -> Result<Json<DataFeedConfig>, ProblemDetails> {
    let toggled = state
        .svc
        .toggle_feed(&id)
        .await
        .map_err(|e| ProblemDetails::internal(e.to_string()))?;

    if !toggled {
        return Err(ProblemDetails::not_found(
            "Feed Not Found",
            format!("no feed with id: {id}"),
        ));
    }

    // Fetch updated config.
    let feed = state
        .svc
        .get_feed(&id)
        .await
        .map_err(|e| ProblemDetails::internal(e.to_string()))?
        .ok_or_else(|| {
            ProblemDetails::not_found("Feed Not Found", format!("no feed with id: {id}"))
        })?;

    // Sync registry: remove (cancels running task), re-register with new state.
    let _ = state.registry.remove(&feed.name);
    if let Err(e) = state.registry.register(feed.clone()) {
        warn!(name = %feed.name, error = %e, "registry sync failed on toggle");
    }

    // Start task if now enabled, stop already handled by remove above.
    if feed.enabled {
        start_feed_task(&feed, &state.registry);
    } else {
        // Disabled: explicitly drop runtime status to Idle and clear any
        // stale last_error so the UI doesn't show "error" on a feed the
        // user just turned off.
        if let Err(e) = state
            .svc
            .update_status(&feed.name, FeedStatus::Idle, None)
            .await
        {
            warn!(name = %feed.name, error = %e, "failed to persist idle status on toggle-off");
        }
    }

    Ok(Json(feed))
}

/// `GET /api/v1/data-feeds/{id}/events` — query events for a feed.
async fn query_events(
    State(state): State<DataFeedRouterState>,
    Path(id): Path<String>,
    Query(params): Query<EventQueryParams>,
) -> Result<Json<EventListResponse>, ProblemDetails> {
    let feed = state
        .svc
        .get_feed(&id)
        .await
        .map_err(|e| ProblemDetails::internal(e.to_string()))?
        .ok_or_else(|| {
            ProblemDetails::not_found("Feed Not Found", format!("no feed with id: {id}"))
        })?;

    let since = params
        .since
        .as_deref()
        .map(parse_duration_ago)
        .transpose()
        .map_err(|e| ProblemDetails::bad_request(format!("invalid 'since' parameter: {e}")))?;

    let limit = params.limit.unwrap_or(50).clamp(1, 200);
    let offset = params.offset.unwrap_or(0).max(0);

    let page = state
        .svc
        .query_events(&feed.name, since, limit, offset)
        .await
        .map_err(|e| ProblemDetails::internal(e.to_string()))?;

    Ok(Json(EventListResponse {
        events:   page.events,
        total:    page.total,
        has_more: page.has_more,
    }))
}

/// `GET /api/v1/data-feeds/{id}/events/{event_id}` — get a single event.
async fn get_event(
    State(state): State<DataFeedRouterState>,
    Path((id, event_id)): Path<(String, String)>,
) -> Result<Json<rara_kernel::data_feed::FeedEvent>, ProblemDetails> {
    let feed = state
        .svc
        .get_feed(&id)
        .await
        .map_err(|e| ProblemDetails::internal(e.to_string()))?
        .ok_or_else(|| {
            ProblemDetails::not_found("Feed Not Found", format!("no feed with id: {id}"))
        })?;

    let event = state
        .svc
        .get_event(&feed.name, &event_id)
        .await
        .map_err(|e| ProblemDetails::internal(e.to_string()))?
        .ok_or_else(|| {
            ProblemDetails::not_found("Event Not Found", format!("no event with id: {event_id}"))
        })?;

    Ok(Json(event))
}

// ---------------------------------------------------------------------------
// Feed task lifecycle
// ---------------------------------------------------------------------------

/// Start a feed source task if the config type supports active operation.
///
/// Polling feeds spawn a background tokio task. Webhook feeds are passive
/// (handled by the webhook axum route). WebSocket feeds are not yet
/// implemented.
pub fn start_feed_task(config: &DataFeedConfig, registry: &Arc<DataFeedRegistry>) {
    match config.feed_type {
        FeedType::Polling => {
            let source = match PollingSource::from_config(config) {
                Ok(s) => s,
                Err(e) => {
                    warn!(
                        feed = %config.name, error = %e,
                        "failed to create polling source from config"
                    );
                    // Surface config-parse failures so the UI reflects reality.
                    registry.report_error(&config.name, format!("config parse failed: {e}"));
                    return;
                }
            };

            // Attach the registry's reporter (if any) so transient fetch
            // errors land in the `data_feeds` row.
            let source = match registry.reporter() {
                Some(r) => source.with_reporter(r),
                None => source,
            };

            let cancel = CancellationToken::new();
            // set_running also fires a Running transition through the
            // reporter, so GET /api/v1/data-feeds reflects the spawn.
            registry.set_running(config.name.clone(), cancel.clone());

            let event_tx = registry.event_tx();
            let name = config.name.clone();
            let registry = registry.clone();

            tokio::spawn(async move {
                match source.run(event_tx, cancel).await {
                    Ok(()) => registry.clear_running(&name),
                    Err(e) => {
                        warn!(feed = %name, error = %e, "feed task exited with error");
                        registry.report_error(&name, e.to_string());
                    }
                }
                info!(feed = %name, "polling feed task stopped");
            });
        }
        FeedType::Webhook => {
            // Webhook is passive — handled by the webhook axum route.
        }
        FeedType::WebSocket => {
            // TODO: Phase 2 — WebSocket client feed.
            warn!(
                feed = %config.name,
                "websocket feed type not yet implemented, skipping task start"
            );
        }
    }
}
