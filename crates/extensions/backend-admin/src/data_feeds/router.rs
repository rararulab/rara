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
//! | PUT    | `/api/v1/data-feeds/{id}`                   | update feed         |
//! | DELETE | `/api/v1/data-feeds/{id}`                   | delete feed         |
//! | PUT    | `/api/v1/data-feeds/{id}/toggle`             | enable/disable feed |
//! | GET    | `/api/v1/data-feeds/{id}/events`             | query feed events   |
//! | GET    | `/api/v1/data-feeds/{id}/events/{event_id}` | get single event    |

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, put},
};
use jiff::{Timestamp, ToSpan};
use rara_kernel::data_feed::{DataFeedConfig, FeedStatus, FeedType};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::service::DataFeedSvc;
use crate::kernel::problem::ProblemDetails;

/// Build the `/api/v1/data-feeds/...` router.
pub fn data_feed_routes(svc: DataFeedSvc) -> Router {
    Router::new()
        .route("/api/v1/data-feeds", get(list_feeds).post(create_feed))
        .route(
            "/api/v1/data-feeds/{id}",
            get(get_feed).put(update_feed).delete(delete_feed),
        )
        .route("/api/v1/data-feeds/{id}/toggle", put(toggle_feed))
        .route("/api/v1/data-feeds/{id}/events", get(query_events))
        .route("/api/v1/data-feeds/{id}/events/{event_id}", get(get_event))
        .with_state(svc)
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
#[derive(Debug, Deserialize)]
struct UpdateFeedRequest {
    name:      String,
    feed_type: FeedType,
    tags:      Vec<String>,
    transport: serde_json::Value,
    auth:      Option<serde_json::Value>,
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

/// `GET /api/v1/data-feeds` — list all feeds.
async fn list_feeds(
    State(svc): State<DataFeedSvc>,
) -> Result<Json<Vec<DataFeedConfig>>, ProblemDetails> {
    let feeds = svc
        .list_feeds()
        .await
        .map_err(|e| ProblemDetails::internal(e.to_string()))?;
    Ok(Json(feeds))
}

/// `POST /api/v1/data-feeds` — create a new feed.
async fn create_feed(
    State(svc): State<DataFeedSvc>,
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

    svc.create_feed(&config)
        .await
        .map_err(|e| ProblemDetails::internal(e.to_string()))?;

    Ok((StatusCode::CREATED, Json(config)))
}

/// `GET /api/v1/data-feeds/{id}` — get a single feed.
async fn get_feed(
    State(svc): State<DataFeedSvc>,
    Path(id): Path<String>,
) -> Result<Json<DataFeedConfig>, ProblemDetails> {
    let feed = svc
        .get_feed(&id)
        .await
        .map_err(|e| ProblemDetails::internal(e.to_string()))?
        .ok_or_else(|| {
            ProblemDetails::not_found("Feed Not Found", format!("no feed with id: {id}"))
        })?;
    Ok(Json(feed))
}

/// `PUT /api/v1/data-feeds/{id}` — update an existing feed.
async fn update_feed(
    State(svc): State<DataFeedSvc>,
    Path(id): Path<String>,
    Json(body): Json<UpdateFeedRequest>,
) -> Result<Json<DataFeedConfig>, ProblemDetails> {
    // Fetch existing to preserve status / timestamps / enabled.
    let existing = svc
        .get_feed(&id)
        .await
        .map_err(|e| ProblemDetails::internal(e.to_string()))?
        .ok_or_else(|| {
            ProblemDetails::not_found("Feed Not Found", format!("no feed with id: {id}"))
        })?;

    let auth = body
        .auth
        .map(serde_json::from_value)
        .transpose()
        .map_err(|e| ProblemDetails::bad_request(format!("invalid auth config: {e}")))?;

    let updated = DataFeedConfig::builder()
        .id(id)
        .name(body.name)
        .feed_type(body.feed_type)
        .tags(body.tags)
        .transport(body.transport)
        .maybe_auth(auth)
        .enabled(existing.enabled)
        .status(existing.status)
        .maybe_last_error(existing.last_error)
        .created_at(existing.created_at)
        .updated_at(Timestamp::now())
        .build();

    svc.update_feed(&updated)
        .await
        .map_err(|e| ProblemDetails::internal(e.to_string()))?;

    Ok(Json(updated))
}

/// `DELETE /api/v1/data-feeds/{id}` — delete a feed.
async fn delete_feed(
    State(svc): State<DataFeedSvc>,
    Path(id): Path<String>,
) -> Result<StatusCode, ProblemDetails> {
    let deleted = svc
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

/// `PUT /api/v1/data-feeds/{id}/toggle` — enable or disable a feed.
async fn toggle_feed(
    State(svc): State<DataFeedSvc>,
    Path(id): Path<String>,
) -> Result<Json<DataFeedConfig>, ProblemDetails> {
    let toggled = svc
        .toggle_feed(&id)
        .await
        .map_err(|e| ProblemDetails::internal(e.to_string()))?;

    if !toggled {
        return Err(ProblemDetails::not_found(
            "Feed Not Found",
            format!("no feed with id: {id}"),
        ));
    }

    // Return the updated config.
    let feed = svc
        .get_feed(&id)
        .await
        .map_err(|e| ProblemDetails::internal(e.to_string()))?
        .ok_or_else(|| {
            ProblemDetails::not_found("Feed Not Found", format!("no feed with id: {id}"))
        })?;

    Ok(Json(feed))
}

/// `GET /api/v1/data-feeds/{id}/events` — query events for a feed.
async fn query_events(
    State(svc): State<DataFeedSvc>,
    Path(id): Path<String>,
    Query(params): Query<EventQueryParams>,
) -> Result<Json<EventListResponse>, ProblemDetails> {
    // Resolve the feed to get its source_name.
    let feed = svc
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

    let page = svc
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
    State(svc): State<DataFeedSvc>,
    Path((id, event_id)): Path<(String, String)>,
) -> Result<Json<rara_kernel::data_feed::FeedEvent>, ProblemDetails> {
    // Resolve feed name first.
    let feed = svc
        .get_feed(&id)
        .await
        .map_err(|e| ProblemDetails::internal(e.to_string()))?
        .ok_or_else(|| {
            ProblemDetails::not_found("Feed Not Found", format!("no feed with id: {id}"))
        })?;

    let event = svc
        .get_event(&feed.name, &event_id)
        .await
        .map_err(|e| ProblemDetails::internal(e.to_string()))?
        .ok_or_else(|| {
            ProblemDetails::not_found("Event Not Found", format!("no event with id: {event_id}"))
        })?;

    Ok(Json(event))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Parse a human-friendly duration string (e.g. `"1h"`, `"24h"`, `"7d"`)
/// and return the timestamp that many units ago from now.
fn parse_duration_ago(s: &str) -> anyhow::Result<Timestamp> {
    let s = s.trim();
    if s.is_empty() {
        anyhow::bail!("empty duration string");
    }

    let (num_str, unit) = s.split_at(s.len() - 1);
    let n: i64 = num_str
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid number in duration: {s}"))?;

    let span = match unit {
        "s" => n.seconds(),
        "m" => n.minutes(),
        "h" => n.hours(),
        "d" => n.days(),
        _ => anyhow::bail!("unsupported duration unit '{unit}', expected s/m/h/d"),
    };

    let now = Timestamp::now();
    let past = now.checked_sub(span)?;
    Ok(past)
}
