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
//! in-memory [`DataFeedRegistry`]. When an active feed is created and
//! enabled, a background task is spawned automatically.

use std::{collections::HashMap, sync::Arc};

use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post, put},
};
use jiff::Timestamp;
use rara_kernel::{
    data_feed::{
        DataFeed, DataFeedConfig, DataFeedRegistry, FeedStatus, FeedType, parse_duration_ago,
        polling::PollingSource,
    },
    identity::{Principal, Resolved, UserId},
};
use rara_trading::{
    feed::{
        catalog::{DefaultFeedSource, default_finance_feed_sources},
        market_candle::MarketCandleSource,
        rss::RssSource,
    },
    finance::registry::{FinanceSubscription, FinanceSubscriptionRegistry},
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
    pub svc:              DataFeedSvc,
    /// In-memory registry (also holds cancellation tokens for running tasks).
    pub registry:         Arc<DataFeedRegistry>,
    /// Finance subscription registry used to annotate catalog source status.
    pub finance_registry: Arc<FinanceSubscriptionRegistry>,
}

/// Build the `/api/v1/data-feeds/...` router.
pub fn data_feed_routes(state: DataFeedRouterState) -> Router {
    Router::new()
        .route("/api/v1/data-feeds", get(list_feeds).post(create_feed))
        .route("/api/v1/data-feeds/summary", get(list_feed_summaries))
        .route("/api/v1/data-feeds/catalog", get(list_feed_catalog))
        .route(
            "/api/v1/data-feeds/catalog/{id}/enable",
            post(enable_catalog_feed),
        )
        .route(
            "/api/v1/data-feeds/catalog/{id}/disable",
            post(disable_catalog_feed),
        )
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

/// Optional request body for enabling a built-in catalog feed.
///
/// Ready sources can still be enabled with an empty body. Provider presets can
/// be materialized by supplying the operator-owned transport/auth details in
/// this body.
#[derive(Debug, Default, Deserialize)]
struct EnableCatalogFeedRequest {
    transport: Option<serde_json::Value>,
    auth:      Option<serde_json::Value>,
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

/// Runtime read-model for a feed's persisted event stream.
#[derive(Debug, Serialize)]
struct FeedSummaryResponse {
    feed_id:       String,
    source_name:   String,
    event_count:   i64,
    last_event_at: Option<Timestamp>,
    lag_seconds:   Option<i64>,
}

/// Built-in feed catalog entry plus current materialized state.
#[derive(Debug, Serialize)]
struct FeedCatalogEntryResponse {
    id:                     String,
    name:                   String,
    description:            String,
    feed_type:              FeedType,
    tags:                   Vec<String>,
    source_name:            String,
    enabled:                bool,
    feed_id:                Option<String>,
    requires_configuration: bool,
    setup_hint:             Option<String>,
    transport_template:     Option<serde_json::Value>,
    venue:                  Option<String>,
    configured_symbols:     Vec<String>,
    configured_timeframes:  Vec<String>,
    subscriptions:          FeedCatalogSubscriptionResponse,
}

/// Current user's finance subscription status for a built-in feed source.
#[derive(Debug, Serialize)]
struct FeedCatalogSubscriptionResponse {
    user_subscribed:       bool,
    user_subscription_ids: Vec<Uuid>,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /api/v1/data-feeds` — list all feeds with runtime status.
async fn list_feeds(
    State(state): State<DataFeedRouterState>,
) -> Result<Json<Vec<DataFeedConfig>>, ProblemDetails> {
    let feeds = state.svc.list_feeds().await?;
    Ok(Json(feeds))
}

/// `GET /api/v1/data-feeds/catalog` — list built-in finance feed sources.
async fn list_feed_catalog(
    State(state): State<DataFeedRouterState>,
    axum::Extension(principal): axum::Extension<Principal<Resolved>>,
) -> Result<Json<Vec<FeedCatalogEntryResponse>>, ProblemDetails> {
    let feeds = state.svc.list_feeds().await?;
    let owner = UserId(principal.user_id.0);
    let subscriptions = state.finance_registry.list_for_owner(&owner).await;
    Ok(Json(catalog_response(&feeds, &subscriptions)))
}

/// `GET /api/v1/data-feeds/summary` — list per-feed persisted-event health.
async fn list_feed_summaries(
    State(state): State<DataFeedRouterState>,
) -> Result<Json<Vec<FeedSummaryResponse>>, ProblemDetails> {
    let feeds = state.svc.list_feeds().await?;
    let summaries = state
        .svc
        .event_summaries()
        .await?
        .into_iter()
        .map(|summary| (summary.source_name.clone(), summary))
        .collect::<HashMap<_, _>>();
    let now = Timestamp::now();

    Ok(Json(
        feeds
            .into_iter()
            .map(|feed| {
                let summary = summaries.get(&feed.name);
                let last_event_at = summary.and_then(|summary| summary.last_event_at);
                let lag_seconds = last_event_at.map(|last_event_at| {
                    let lag = now.duration_since(last_event_at);
                    lag.as_secs().max(0)
                });

                FeedSummaryResponse {
                    feed_id: feed.id,
                    source_name: feed.name,
                    event_count: summary.map_or(0, |summary| summary.event_count),
                    last_event_at,
                    lag_seconds,
                }
            })
            .collect(),
    ))
}

/// `POST /api/v1/data-feeds/catalog/{id}/enable` — materialize a default feed.
async fn enable_catalog_feed(
    State(state): State<DataFeedRouterState>,
    axum::Extension(principal): axum::Extension<
        rara_kernel::identity::Principal<rara_kernel::identity::Resolved>,
    >,
    Path(id): Path<String>,
    body: Bytes,
) -> Result<(StatusCode, Json<DataFeedConfig>), ProblemDetails> {
    if !principal.is_admin() {
        return Err(ProblemDetails::forbidden(
            "enabling data feeds requires admin role",
        ));
    }

    let source = find_catalog_source(&id)?;
    let request = parse_enable_catalog_feed_request(&body)?;
    if !source.can_enable() && request.transport.is_none() {
        return Err(ProblemDetails::bad_request(format!(
            "catalog source requires configuration before it can be enabled: {id}"
        )));
    }
    let feed_name = source.feed_name();
    let transport = catalog_transport(source.transport.clone(), request.transport, &id)?;
    let auth_value = match request.auth {
        Some(auth) => Some(auth),
        None => source
            .auth
            .clone()
            .map(serde_json::to_value)
            .transpose()
            .map_err(|e| ProblemDetails::bad_request(format!("invalid auth config: {e}")))?,
    };
    let auth = auth_value
        .map(serde_json::from_value)
        .transpose()
        .map_err(|e| ProblemDetails::bad_request(format!("invalid auth config: {e}")))?;
    let existing = state
        .svc
        .list_feeds()
        .await?
        .into_iter()
        .find(|feed| feed.name == feed_name);

    let now = Timestamp::now();
    let (status, config) = if let Some(existing) = existing {
        let mut updated = DataFeedConfig::builder()
            .id(existing.id)
            .name(feed_name)
            .feed_type(source.feed_type.clone())
            .tags(source.tags.clone())
            .transport(transport.clone())
            .maybe_auth(auth.clone())
            .enabled(true)
            .status(FeedStatus::Idle)
            .created_at(existing.created_at)
            .updated_at(now)
            .build();
        normalize_active_feed_config(&mut updated)?;
        state.svc.update_feed(&updated).await?;
        (StatusCode::OK, updated)
    } else {
        let mut created = DataFeedConfig::builder()
            .id(Uuid::new_v4().to_string())
            .name(feed_name)
            .feed_type(source.feed_type.clone())
            .tags(source.tags.clone())
            .transport(transport)
            .maybe_auth(auth)
            .enabled(true)
            .status(FeedStatus::Idle)
            .created_at(now)
            .updated_at(now)
            .build();
        normalize_active_feed_config(&mut created)?;
        state.svc.create_feed(&created).await?;
        (StatusCode::CREATED, created)
    };

    sync_registry_and_maybe_start(&config, &state.registry);

    Ok((status, Json(config)))
}

/// `POST /api/v1/data-feeds/catalog/{id}/disable` — turn off a materialized
/// built-in feed without deleting its config row.
async fn disable_catalog_feed(
    State(state): State<DataFeedRouterState>,
    axum::Extension(principal): axum::Extension<
        rara_kernel::identity::Principal<rara_kernel::identity::Resolved>,
    >,
    Path(id): Path<String>,
) -> Result<Json<DataFeedConfig>, ProblemDetails> {
    if !principal.is_admin() {
        return Err(ProblemDetails::forbidden(
            "disabling data feeds requires admin role",
        ));
    }

    let source = find_catalog_source(&id)?;
    let feed_name = source.feed_name();
    let existing = state
        .svc
        .list_feeds()
        .await?
        .into_iter()
        .find(|feed| feed.name == feed_name)
        .ok_or_else(|| {
            ProblemDetails::not_found(
                "Feed Not Found",
                format!("catalog source is not enabled: {id}"),
            )
        })?;

    let updated = DataFeedConfig::builder()
        .id(existing.id)
        .name(existing.name)
        .feed_type(existing.feed_type)
        .tags(existing.tags)
        .transport(existing.transport)
        .maybe_auth(existing.auth)
        .enabled(false)
        .status(FeedStatus::Idle)
        .created_at(existing.created_at)
        .updated_at(Timestamp::now())
        .build();

    state.svc.update_feed(&updated).await?;
    sync_registry_and_maybe_start(&updated, &state.registry);

    Ok(Json(updated))
}

/// `POST /api/v1/data-feeds` — create a new feed, sync registry, start task.
async fn create_feed(
    State(state): State<DataFeedRouterState>,
    axum::Extension(principal): axum::Extension<
        rara_kernel::identity::Principal<rara_kernel::identity::Resolved>,
    >,
    Json(body): Json<CreateFeedRequest>,
) -> Result<(StatusCode, Json<DataFeedConfig>), ProblemDetails> {
    if !principal.is_admin() {
        return Err(ProblemDetails::forbidden(
            "creating data feeds requires admin role",
        ));
    }
    info!(
        actor = %principal.user_id,
        name = %body.name,
        "create_feed"
    );

    let auth = body
        .auth
        .map(serde_json::from_value)
        .transpose()
        .map_err(|e| ProblemDetails::bad_request(format!("invalid auth config: {e}")))?;

    let now = Timestamp::now();
    let mut config = DataFeedConfig::builder()
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
    normalize_active_feed_config(&mut config)?;

    // 1. Persist to database.
    state.svc.create_feed(&config).await?;

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
    let feed = state.svc.get_feed(&id).await?.ok_or_else(|| {
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
    axum::Extension(principal): axum::Extension<
        rara_kernel::identity::Principal<rara_kernel::identity::Resolved>,
    >,
    Path(id): Path<String>,
    Json(body): Json<UpdateFeedRequest>,
) -> Result<Json<DataFeedConfig>, ProblemDetails> {
    if !principal.is_admin() {
        return Err(ProblemDetails::forbidden(
            "updating data feeds requires admin role",
        ));
    }
    info!(
        actor = %principal.user_id,
        feed_id = %id,
        "update_feed"
    );

    let existing = state.svc.get_feed(&id).await?.ok_or_else(|| {
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

    let mut updated = DataFeedConfig::builder()
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
    normalize_active_feed_config(&mut updated)?;

    // 1. Persist to database.
    state.svc.update_feed(&updated).await?;

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
    let deleted = state.svc.delete_feed(&id).await?;

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
    axum::Extension(principal): axum::Extension<
        rara_kernel::identity::Principal<rara_kernel::identity::Resolved>,
    >,
    Path(id): Path<String>,
) -> Result<Json<DataFeedConfig>, ProblemDetails> {
    if !principal.is_admin() {
        return Err(ProblemDetails::forbidden(
            "toggling data feeds requires admin role",
        ));
    }
    info!(
        actor = %principal.user_id,
        feed_id = %id,
        "toggle_feed"
    );

    let mut feed = state.svc.get_feed(&id).await?.ok_or_else(|| {
        ProblemDetails::not_found("Feed Not Found", format!("no feed with id: {id}"))
    })?;
    feed.enabled = !feed.enabled;
    feed.updated_at = Timestamp::now();

    if feed.enabled {
        normalize_active_feed_config(&mut feed)?;
    }
    if !state.svc.update_feed(&feed).await? {
        return Err(ProblemDetails::internal(format!(
            "failed to update data feed: {}",
            feed.name
        )));
    }

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
    let feed = state.svc.get_feed(&id).await?.ok_or_else(|| {
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
        .await?;

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
    let feed = state.svc.get_feed(&id).await?.ok_or_else(|| {
        ProblemDetails::not_found("Feed Not Found", format!("no feed with id: {id}"))
    })?;

    let event = state
        .svc
        .get_event(&feed.name, &event_id)
        .await?
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
/// Active feeds spawn a background tokio task. Webhook feeds are passive
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
            let run_id = registry.set_running(config.name.clone(), cancel.clone());

            let event_tx = registry.event_tx();
            let name = config.name.clone();
            let registry = registry.clone();

            tokio::spawn(async move {
                match source.run(event_tx, cancel).await {
                    Ok(()) => {
                        registry.clear_running_if_current(&name, run_id);
                    }
                    Err(e) => {
                        warn!(feed = %name, error = %e, "feed task exited with error");
                        registry.report_error_if_current(&name, run_id, e.to_string());
                    }
                }
                info!(feed = %name, "polling feed task stopped");
            });
        }
        FeedType::Rss => {
            let source = match RssSource::from_config(config) {
                Ok(source) => source,
                Err(err) => {
                    warn!(
                        feed = %config.name, error = %err,
                        "failed to create rss source from config"
                    );
                    registry.report_error(&config.name, format!("config parse failed: {err}"));
                    return;
                }
            };

            let source = match registry.reporter() {
                Some(reporter) => source.with_reporter(reporter),
                None => source,
            };

            let cancel = CancellationToken::new();
            let run_id = registry.set_running(config.name.clone(), cancel.clone());

            let event_tx = registry.event_tx();
            let name = config.name.clone();
            let registry = registry.clone();

            tokio::spawn(async move {
                match source.run(event_tx, cancel).await {
                    Ok(()) => {
                        registry.clear_running_if_current(&name, run_id);
                    }
                    Err(err) => {
                        warn!(feed = %name, error = %err, "feed task exited with error");
                        registry.report_error_if_current(&name, run_id, err.to_string());
                    }
                }
                info!(feed = %name, "rss feed task stopped");
            });
        }
        FeedType::MarketCandle => {
            let source = match MarketCandleSource::from_config(config) {
                Ok(source) => source,
                Err(err) => {
                    warn!(
                        feed = %config.name, error = %err,
                        "failed to create market candle source from config"
                    );
                    registry.report_error(&config.name, format!("config parse failed: {err}"));
                    return;
                }
            };

            let source = match registry.reporter() {
                Some(reporter) => source.with_reporter(reporter),
                None => source,
            };

            let cancel = CancellationToken::new();
            let run_id = registry.set_running(config.name.clone(), cancel.clone());

            let event_tx = registry.event_tx();
            let name = config.name.clone();
            let registry = registry.clone();

            tokio::spawn(async move {
                match source.run(event_tx, cancel).await {
                    Ok(()) => {
                        registry.clear_running_if_current(&name, run_id);
                    }
                    Err(err) => {
                        warn!(feed = %name, error = %err, "feed task exited with error");
                        registry.report_error_if_current(&name, run_id, err.to_string());
                    }
                }
                info!(feed = %name, "market candle feed task stopped");
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

fn catalog_response(
    feeds: &[DataFeedConfig],
    subscriptions: &[FinanceSubscription],
) -> Vec<FeedCatalogEntryResponse> {
    default_finance_feed_sources()
        .into_iter()
        .map(|source| {
            let feed_name = source.feed_name();
            let feed = feeds.iter().find(|feed| feed.name == feed_name);
            let venue = catalog_transport_string(source.transport.as_ref(), "venue");
            let configured_symbols =
                catalog_transport_string_list(source.transport.as_ref(), "symbols");
            let configured_timeframes =
                catalog_transport_string_list(source.transport.as_ref(), "timeframes");
            FeedCatalogEntryResponse {
                id: source.id,
                name: source.name,
                description: source.description,
                feed_type: source.feed_type,
                tags: source.tags,
                source_name: feed_name.clone(),
                enabled: feed.is_some_and(|feed| feed.enabled),
                feed_id: feed.map(|feed| feed.id.clone()),
                requires_configuration: source.requires_configuration,
                setup_hint: source.setup_hint,
                transport_template: source.transport,
                venue,
                configured_symbols,
                configured_timeframes,
                subscriptions: catalog_subscription_response(subscriptions, &feed_name),
            }
        })
        .collect()
}

fn catalog_subscription_response(
    subscriptions: &[FinanceSubscription],
    source_name: &str,
) -> FeedCatalogSubscriptionResponse {
    let user_subscription_ids = subscriptions
        .iter()
        .filter(|subscription| {
            subscription
                .source_names
                .iter()
                .any(|name| name == source_name)
        })
        .map(|subscription| subscription.id)
        .collect::<Vec<_>>();

    FeedCatalogSubscriptionResponse {
        user_subscribed: !user_subscription_ids.is_empty(),
        user_subscription_ids,
    }
}

fn catalog_transport_string(transport: Option<&serde_json::Value>, key: &str) -> Option<String> {
    transport
        .and_then(|transport| transport.get(key))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn catalog_transport_string_list(transport: Option<&serde_json::Value>, key: &str) -> Vec<String> {
    transport
        .and_then(|transport| transport.get(key))
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect()
}

fn find_catalog_source(id: &str) -> Result<DefaultFeedSource, ProblemDetails> {
    default_finance_feed_sources()
        .into_iter()
        .find(|source| source.id == id)
        .ok_or_else(|| {
            ProblemDetails::not_found(
                "Catalog Source Not Found",
                format!("no built-in data feed source with id: {id}"),
            )
        })
}

fn parse_enable_catalog_feed_request(
    body: &[u8],
) -> Result<EnableCatalogFeedRequest, ProblemDetails> {
    if body.iter().all(u8::is_ascii_whitespace) {
        return Ok(EnableCatalogFeedRequest::default());
    }
    serde_json::from_slice(body)
        .map_err(|e| ProblemDetails::bad_request(format!("invalid catalog enable body: {e}")))
}

fn catalog_transport(
    template: Option<serde_json::Value>,
    override_value: Option<serde_json::Value>,
    id: &str,
) -> Result<serde_json::Value, ProblemDetails> {
    match (template, override_value) {
        (Some(serde_json::Value::Object(mut base)), Some(serde_json::Value::Object(overrides))) => {
            for (key, value) in overrides {
                base.insert(key, value);
            }
            Ok(serde_json::Value::Object(base))
        }
        (Some(_), Some(override_value @ serde_json::Value::Object(_))) => Ok(override_value),
        (None, Some(override_value @ serde_json::Value::Object(_))) => Ok(override_value),
        (Some(template), None) => Ok(template),
        (None, None) => Err(ProblemDetails::bad_request(format!(
            "catalog source has no transport template: {id}"
        ))),
        (_, Some(_)) => Err(ProblemDetails::bad_request(
            "catalog enable transport must be a JSON object",
        )),
    }
}

fn normalize_active_feed_config(config: &mut DataFeedConfig) -> Result<(), ProblemDetails> {
    match config.feed_type {
        FeedType::Polling => PollingSource::from_config(config)
            .map(|_| ())
            .map_err(|e| ProblemDetails::bad_request(format!("invalid polling feed config: {e}"))),
        FeedType::Rss => RssSource::from_config(config)
            .map(|_| ())
            .map_err(|e| ProblemDetails::bad_request(format!("invalid rss feed config: {e}"))),
        FeedType::MarketCandle => MarketCandleSource::normalize_config(config).map_err(|e| {
            ProblemDetails::bad_request(format!("invalid market candle feed config: {e}"))
        }),
        FeedType::Webhook | FeedType::WebSocket => Ok(()),
    }
}

fn sync_registry_and_maybe_start(config: &DataFeedConfig, registry: &Arc<DataFeedRegistry>) {
    let _ = registry.remove(&config.name);
    if let Err(e) = registry.register(config.clone()) {
        warn!(name = %config.name, error = %e, "registry sync failed for catalog feed");
    }
    if config.enabled {
        start_feed_task(config, registry);
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use axum::{
        Router,
        body::Body,
        http::{Request, StatusCode},
        middleware,
    };
    use diesel_async::RunQueryDsl;
    use rara_kernel::{
        data_feed::DataFeedRegistry,
        error::Result as KernelResult,
        identity::{KernelUser, Permission, Role, UserStore},
        security::{ApprovalManager, ApprovalPolicy, SecuritySubsystem},
        session::SessionKey,
        testing::build_memory_diesel_pools,
    };
    use rara_trading::finance::registry::{FinanceDelivery, FinanceEventKind};
    use tower::ServiceExt;

    use super::*;
    use crate::auth::{AuthState, auth_layer};

    struct TestUserStore {
        user: KernelUser,
    }

    #[async_trait]
    impl UserStore for TestUserStore {
        async fn get_by_name(&self, name: &str) -> KernelResult<Option<KernelUser>> {
            Ok((name == self.user.name).then(|| self.user.clone()))
        }

        async fn list(&self) -> KernelResult<Vec<KernelUser>> { Ok(vec![self.user.clone()]) }
    }

    fn user_of(role: Role) -> KernelUser {
        KernelUser {
            name: match role {
                Role::Admin | Role::Root => "admin".into(),
                Role::User => "alice".into(),
            },
            role,
            permissions: match role {
                Role::Admin | Role::Root => vec![Permission::All],
                // Non-admin callers still need Spawn to resolve through the
                // security subsystem — matches production user seeding.
                Role::User => vec![Permission::Spawn],
            },
            enabled: true,
        }
    }

    fn auth_state_direct(user: KernelUser) -> AuthState {
        let name = user.name.clone();
        let store: Arc<dyn UserStore> = Arc::new(TestUserStore { user });
        let approval = Arc::new(ApprovalManager::new(ApprovalPolicy::default()));
        let security = Arc::new(SecuritySubsystem::new(store, approval));
        AuthState::for_tests("s3cret", &name, security)
    }

    fn test_finance_registry() -> Arc<FinanceSubscriptionRegistry> {
        Arc::new(FinanceSubscriptionRegistry::load(
            std::env::temp_dir().join(format!(
                "rara-test-finance-subscriptions-{}.json",
                Uuid::new_v4()
            )),
        ))
    }

    /// Build a router whose handlers use a real (but empty / schema-less)
    /// diesel pool. The non-admin Principal guard runs before any DB query,
    /// so the pool is never hit on the 403 path these tests exercise.
    async fn app_with_user(user: KernelUser) -> Router {
        let pools = build_memory_diesel_pools().await;
        bootstrap_data_feed_schema(&pools).await;
        let svc = DataFeedSvc::new(pools);
        let (event_tx, _event_rx) = tokio::sync::mpsc::channel(16);
        let registry = Arc::new(DataFeedRegistry::new(event_tx));
        let state = DataFeedRouterState {
            svc,
            registry,
            finance_registry: test_finance_registry(),
        };
        let auth = auth_state_direct(user);
        data_feed_routes(state).layer(middleware::from_fn_with_state(auth, auth_layer))
    }

    async fn app_with_user_and_pools(
        user: KernelUser,
    ) -> (Router, yunara_store::diesel_pool::DieselSqlitePools) {
        let pools = build_memory_diesel_pools().await;
        bootstrap_data_feed_schema(&pools).await;
        let svc = DataFeedSvc::new(pools.clone());
        let (event_tx, _event_rx) = tokio::sync::mpsc::channel(16);
        let registry = Arc::new(DataFeedRegistry::new(event_tx));
        let state = DataFeedRouterState {
            svc,
            registry,
            finance_registry: test_finance_registry(),
        };
        let auth = auth_state_direct(user);
        (
            data_feed_routes(state).layer(middleware::from_fn_with_state(auth, auth_layer)),
            pools,
        )
    }

    async fn app_with_user_and_finance_registry(
        user: KernelUser,
        finance_registry: Arc<FinanceSubscriptionRegistry>,
    ) -> Router {
        let pools = build_memory_diesel_pools().await;
        bootstrap_data_feed_schema(&pools).await;
        let svc = DataFeedSvc::new(pools);
        let (event_tx, _event_rx) = tokio::sync::mpsc::channel(16);
        let registry = Arc::new(DataFeedRegistry::new(event_tx));
        let state = DataFeedRouterState {
            svc,
            registry,
            finance_registry,
        };
        let auth = auth_state_direct(user);
        data_feed_routes(state).layer(middleware::from_fn_with_state(auth, auth_layer))
    }

    async fn bootstrap_data_feed_schema(pools: &yunara_store::diesel_pool::DieselSqlitePools) {
        let mut conn = pools.writer.get().await.expect("pool conn");
        for ddl in [
            "CREATE TABLE data_feeds (
                id TEXT PRIMARY KEY NOT NULL,
                name TEXT NOT NULL UNIQUE,
                feed_type TEXT NOT NULL,
                tags TEXT NOT NULL DEFAULT '[]',
                transport TEXT NOT NULL DEFAULT '{}',
                auth TEXT,
                enabled INTEGER NOT NULL DEFAULT 1,
                status TEXT NOT NULL DEFAULT 'idle',
                last_error TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
                updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
            )",
            "CREATE INDEX idx_data_feeds_name ON data_feeds(name)",
            "CREATE INDEX idx_data_feeds_type ON data_feeds(feed_type)",
            "CREATE TABLE data_feed_events (
                id TEXT PRIMARY KEY NOT NULL,
                source_name TEXT NOT NULL,
                event_type TEXT NOT NULL,
                tags TEXT NOT NULL DEFAULT '[]',
                payload TEXT NOT NULL DEFAULT '{}',
                received_at TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
            )",
            "CREATE INDEX idx_data_feed_events_source ON data_feed_events(source_name)",
            "CREATE INDEX idx_data_feed_events_received ON data_feed_events(received_at)",
        ] {
            diesel::sql_query(ddl)
                .execute(&mut *conn)
                .await
                .expect("bootstrap data feed schema");
        }
    }

    #[tokio::test]
    async fn non_admin_cannot_create_feed() {
        let app = app_with_user(user_of(Role::User)).await;
        let body = serde_json::json!({
            "name": "x",
            "feed_type": "polling",
            "tags": [],
            "transport": {},
        });
        let res = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/data-feeds")
                    .header("Authorization", "Bearer s3cret")
                    .header("Content-Type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn non_admin_cannot_update_feed() {
        let app = app_with_user(user_of(Role::User)).await;
        let body = serde_json::json!({ "name": "new" });
        let res = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/v1/data-feeds/some-id")
                    .header("Authorization", "Bearer s3cret")
                    .header("Content-Type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn non_admin_cannot_toggle_feed() {
        let app = app_with_user(user_of(Role::User)).await;
        let res = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/v1/data-feeds/some-id/toggle")
                    .header("Authorization", "Bearer s3cret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn non_admin_cannot_enable_catalog_feed() {
        let app = app_with_user(user_of(Role::User)).await;
        let res = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/data-feeds/catalog/fed-press-releases/enable")
                    .header("Authorization", "Bearer s3cret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn admin_create_market_candle_feed_normalizes_transport() {
        let app = app_with_user(user_of(Role::Admin)).await;
        let body = serde_json::json!({
            "name": "binance-spot",
            "feed_type": "market_candle",
            "tags": ["finance", "market-data"],
            "transport": {
                "provider": " BINANCE ",
                "base_url": "https://api.binance.com",
                "interval_secs": 60,
                "headers": {},
                "venue": " BINANCE ",
                "symbols": [" btcusdt ", "BTCUSDT", "ethusdt"],
                "timeframes": [" 15M ", "15m", "1H"],
                "max_candles_per_poll": 1000
            }
        });
        let res = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/data-feeds")
                    .header("Authorization", "Bearer s3cret")
                    .header("Content-Type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(res.status(), StatusCode::CREATED);

        let body = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let feed: DataFeedConfig = serde_json::from_slice(&body).unwrap();

        assert_eq!(feed.feed_type, FeedType::MarketCandle);
        assert_eq!(feed.transport["provider"], "binance");
        assert_eq!(feed.transport["venue"], "binance");
        assert_eq!(
            feed.transport["symbols"],
            serde_json::json!(["BTCUSDT", "ETHUSDT"])
        );
        assert_eq!(
            feed.transport["timeframes"],
            serde_json::json!(["15m", "1h"])
        );
    }

    #[tokio::test]
    async fn toggle_market_candle_feed_normalizes_legacy_transport_before_enabling() {
        let (app, pools) = app_with_user_and_pools(user_of(Role::Admin)).await;
        let svc = DataFeedSvc::new(pools);
        let now = Timestamp::now();
        let legacy = DataFeedConfig::builder()
            .id("legacy-binance".to_owned())
            .name("finance-binance-market-candles".to_owned())
            .feed_type(FeedType::MarketCandle)
            .tags(vec!["finance".to_owned(), "market-data".to_owned()])
            .transport(serde_json::json!({
                "provider": " BINANCE ",
                "base_url": "https://api.binance.com",
                "interval_secs": 60,
                "headers": {},
                "venue": " BINANCE ",
                "symbols": [" ethusdt ", "BTCUSDT", "btcusdt"],
                "timeframes": [" 15M ", "1m", "15m"],
                "max_candles_per_poll": 1000
            }))
            .enabled(false)
            .status(FeedStatus::Idle)
            .created_at(now)
            .updated_at(now)
            .build();
        svc.create_feed(&legacy).await.unwrap();

        let res = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/v1/data-feeds/legacy-binance/toggle")
                    .header("Authorization", "Bearer s3cret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(res.status(), StatusCode::OK);

        let body = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let feed: DataFeedConfig = serde_json::from_slice(&body).unwrap();

        assert!(feed.enabled);
        assert_eq!(feed.transport["provider"], "binance");
        assert_eq!(feed.transport["venue"], "binance");
        assert_eq!(
            feed.transport["symbols"],
            serde_json::json!(["BTCUSDT", "ETHUSDT"])
        );
        assert_eq!(
            feed.transport["timeframes"],
            serde_json::json!(["15m", "1m"])
        );

        let persisted = svc.get_feed("legacy-binance").await.unwrap().unwrap();
        assert!(persisted.enabled);
        assert_eq!(persisted.transport, feed.transport);
    }

    #[tokio::test]
    async fn finance_catalog_lists_default_sources() {
        let app = app_with_user(user_of(Role::Admin)).await;
        let res = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/data-feeds/catalog")
                    .header("Authorization", "Bearer s3cret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(res.status(), StatusCode::OK);

        let body = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let entries: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let ids = entries
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["id"].as_str().unwrap())
            .collect::<Vec<_>>();

        assert!(ids.contains(&"fed-press-releases"));
        assert!(ids.contains(&"sec-press-releases"));
        assert!(ids.contains(&"binance-market-candles"));

        let binance = entries
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["id"] == "binance-market-candles")
            .unwrap();
        assert_eq!(binance["source_name"], "finance-binance-market-candles");
        assert_eq!(binance["requires_configuration"], false);
        assert_eq!(binance["feed_type"], "market_candle");
        assert_eq!(
            binance["transport_template"]["provider"].as_str().unwrap(),
            "binance"
        );
        assert_eq!(binance["venue"], "binance");
        assert_eq!(
            binance["configured_symbols"],
            serde_json::json!(["BTCUSDT", "ETHUSDT"])
        );
        assert_eq!(binance["configured_timeframes"], serde_json::json!(["1m"]));
    }

    #[tokio::test]
    async fn finance_catalog_reports_current_user_subscription_status() {
        let finance_registry = test_finance_registry();
        let subscription_id = Uuid::new_v4();
        finance_registry
            .upsert(FinanceSubscription {
                id:                     subscription_id,
                owner:                  UserId("admin".to_owned()),
                session_key:            SessionKey::new(),
                event_kinds:            vec![FinanceEventKind::RssArticle],
                source_names:           vec!["finance-fed-press-releases".to_owned()],
                category_tags:          Vec::new(),
                watch_terms:            Vec::new(),
                venues:                 Vec::new(),
                symbols:                Vec::new(),
                timeframes:             Vec::new(),
                delivery:               FinanceDelivery::Silent,
                cooldown_secs:          900,
                max_immediate_per_hour: 6,
            })
            .await
            .unwrap();

        let app = app_with_user_and_finance_registry(user_of(Role::Admin), finance_registry).await;
        let res = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/data-feeds/catalog")
                    .header("Authorization", "Bearer s3cret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(res.status(), StatusCode::OK);

        let body = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let entries: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let fed = entries
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["id"] == "fed-press-releases")
            .unwrap();
        let sec = entries
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["id"] == "sec-press-releases")
            .unwrap();

        assert_eq!(fed["subscriptions"]["user_subscribed"], true);
        assert_eq!(
            fed["subscriptions"]["user_subscription_ids"],
            serde_json::json!([subscription_id])
        );
        assert_eq!(sec["subscriptions"]["user_subscribed"], false);
        assert_eq!(
            sec["subscriptions"]["user_subscription_ids"],
            serde_json::json!([])
        );
    }

    #[tokio::test]
    async fn finance_catalog_enable_creates_enabled_feed() {
        let app = app_with_user(user_of(Role::Admin)).await;
        let res = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/data-feeds/catalog/fed-press-releases/enable")
                    .header("Authorization", "Bearer s3cret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(res.status(), StatusCode::CREATED);

        let body = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let feed: DataFeedConfig = serde_json::from_slice(&body).unwrap();

        assert_eq!(feed.name, "finance-fed-press-releases");
        assert_eq!(feed.feed_type, FeedType::Rss);
        assert!(feed.enabled);
        assert!(feed.tags.contains(&"finance".to_owned()));
        assert!(feed.tags.contains(&"news".to_owned()));
        assert!(feed.tags.contains(&"fed".to_owned()));
    }

    #[tokio::test]
    async fn finance_catalog_enable_creates_default_binance_feed() {
        let app = app_with_user(user_of(Role::Admin)).await;
        let res = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/data-feeds/catalog/binance-market-candles/enable")
                    .header("Authorization", "Bearer s3cret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(res.status(), StatusCode::CREATED);

        let body = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let feed: DataFeedConfig = serde_json::from_slice(&body).unwrap();

        assert_eq!(feed.name, "finance-binance-market-candles");
        assert_eq!(feed.feed_type, FeedType::MarketCandle);
        assert_eq!(feed.transport["provider"], "binance");
        assert_eq!(feed.transport["venue"], "binance");
        assert_eq!(feed.transport["symbols"][0], "BTCUSDT");
        assert!(feed.enabled);
    }

    #[tokio::test]
    async fn finance_catalog_enable_rejects_provider_preset_without_config() {
        let app = app_with_user(user_of(Role::Admin)).await;
        let res = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/data-feeds/catalog/longbridge-market-candles/enable")
                    .header("Authorization", "Bearer s3cret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn finance_catalog_enable_accepts_market_preset_configuration() {
        let app = app_with_user(user_of(Role::Admin)).await;
        let res = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/data-feeds/catalog/longbridge-market-candles/enable")
                    .header("Authorization", "Bearer s3cret")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "transport": {
                                "url": "https://market-data.local/longbridge/candles/latest",
                                "interval_secs": 120,
                                "headers": {},
                                "venue": " LONGBRIDGE ",
                                "symbols": [" nvda.us ", "AAPL.US", "aapl.us"],
                                "timeframes": [" 1D ", "1d"],
                                "max_candles_per_poll": 500
                            }
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(res.status(), StatusCode::CREATED);

        let body = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let feed: DataFeedConfig = serde_json::from_slice(&body).unwrap();

        assert_eq!(feed.name, "finance-longbridge-market-candles");
        assert_eq!(feed.feed_type, FeedType::MarketCandle);
        assert_eq!(feed.transport["venue"], "longbridge");
        assert_eq!(
            feed.transport["symbols"],
            serde_json::json!(["AAPL.US", "NVDA.US"])
        );
        assert_eq!(feed.transport["timeframes"], serde_json::json!(["1d"]));
        assert_eq!(feed.transport["interval_secs"], 120);
        assert!(feed.enabled);
    }

    #[tokio::test]
    async fn finance_catalog_disable_turns_existing_feed_off() {
        let app = app_with_user(user_of(Role::Admin)).await;
        let enable_res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/data-feeds/catalog/sec-press-releases/enable")
                    .header("Authorization", "Bearer s3cret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(enable_res.status(), StatusCode::CREATED);

        let disable_res = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/data-feeds/catalog/sec-press-releases/disable")
                    .header("Authorization", "Bearer s3cret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(disable_res.status(), StatusCode::OK);

        let body = axum::body::to_bytes(disable_res.into_body(), usize::MAX)
            .await
            .unwrap();
        let feed: DataFeedConfig = serde_json::from_slice(&body).unwrap();

        assert_eq!(feed.name, "finance-sec-press-releases");
        assert!(!feed.enabled);
        assert_eq!(feed.status, FeedStatus::Idle);
    }

    #[tokio::test]
    async fn feed_summary_reports_event_count_and_lag() {
        let (app, pools) = app_with_user_and_pools(user_of(Role::Admin)).await;
        let create_body = serde_json::json!({
            "name": "summary-feed",
            "feed_type": "polling",
            "tags": ["finance"],
            "transport": {
                "url": "https://example.com/feed",
                "interval_secs": 60,
                "headers": {},
                "method": "GET"
            },
        });
        let create_res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/data-feeds")
                    .header("Authorization", "Bearer s3cret")
                    .header("Content-Type", "application/json")
                    .body(Body::from(create_body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(create_res.status(), StatusCode::CREATED);

        let body = axum::body::to_bytes(create_res.into_body(), usize::MAX)
            .await
            .unwrap();
        let feed: DataFeedConfig = serde_json::from_slice(&body).unwrap();
        let event_at = Timestamp::now()
            .checked_sub(jiff::SignedDuration::from_secs(60))
            .unwrap();
        let event_id = rara_kernel::data_feed::FeedEventId::deterministic("summary-feed:event");

        let mut conn = pools.writer.get().await.expect("pool conn");
        diesel::sql_query(
            "INSERT INTO data_feed_events (id, source_name, event_type, tags, payload, \
             received_at) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind::<diesel::sql_types::Text, _>(event_id.to_string())
        .bind::<diesel::sql_types::Text, _>("summary-feed")
        .bind::<diesel::sql_types::Text, _>("poll_response")
        .bind::<diesel::sql_types::Text, _>("[\"finance\"]")
        .bind::<diesel::sql_types::Text, _>("{\"ok\":true}")
        .bind::<diesel::sql_types::Text, _>(event_at.to_string())
        .execute(&mut *conn)
        .await
        .expect("insert event");
        drop(conn);

        let res = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/data-feeds/summary")
                    .header("Authorization", "Bearer s3cret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        let body = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let summaries: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let summary = summaries
            .as_array()
            .unwrap()
            .iter()
            .find(|summary| summary["feed_id"] == feed.id)
            .unwrap();

        assert_eq!(summary["source_name"], "summary-feed");
        assert_eq!(summary["event_count"], 1);
        assert_eq!(summary["last_event_at"], event_at.to_string());
        let lag_seconds = summary["lag_seconds"].as_i64().unwrap();
        assert!((58..=62).contains(&lag_seconds), "lag={lag_seconds}");
    }

    #[tokio::test]
    async fn feed_summary_includes_zero_event_feeds() {
        let app = app_with_user(user_of(Role::Admin)).await;
        let create_body = serde_json::json!({
            "name": "empty-feed",
            "feed_type": "polling",
            "tags": [],
            "transport": {
                "url": "https://example.com/feed",
                "interval_secs": 60,
                "headers": {},
                "method": "GET"
            },
        });
        let create_res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/data-feeds")
                    .header("Authorization", "Bearer s3cret")
                    .header("Content-Type", "application/json")
                    .body(Body::from(create_body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(create_res.status(), StatusCode::CREATED);

        let res = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/data-feeds/summary")
                    .header("Authorization", "Bearer s3cret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        let body = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let summaries: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let summary = summaries
            .as_array()
            .unwrap()
            .iter()
            .find(|summary| summary["source_name"] == "empty-feed")
            .unwrap();

        assert_eq!(summary["event_count"], 0);
        assert!(summary["last_event_at"].is_null());
        assert!(summary["lag_seconds"].is_null());
    }

    #[tokio::test]
    async fn start_feed_task_starts_rss_feed() {
        let (event_tx, _event_rx) = tokio::sync::mpsc::channel(16);
        let registry = Arc::new(DataFeedRegistry::new(event_tx));
        let config = DataFeedConfig::builder()
            .id("rss-feed-id".to_owned())
            .name("fed-news".to_owned())
            .feed_type(FeedType::Rss)
            .tags(vec!["finance".to_owned()])
            .transport(serde_json::json!({
                "url": "https://example.com/feed.xml",
                "interval_secs": 3600,
                "max_entries_per_poll": 20
            }))
            .enabled(true)
            .status(FeedStatus::Idle)
            .created_at(Timestamp::UNIX_EPOCH)
            .updated_at(Timestamp::UNIX_EPOCH)
            .build();

        registry
            .register(config.clone())
            .expect("test config should register");
        start_feed_task(&config, &registry);

        assert!(registry.is_running("fed-news"));

        registry
            .remove("fed-news")
            .expect("test cleanup should cancel");
    }

    #[tokio::test]
    async fn start_feed_task_starts_market_candle_feed() {
        let (event_tx, _event_rx) = tokio::sync::mpsc::channel(16);
        let registry = Arc::new(DataFeedRegistry::new(event_tx));
        let config = DataFeedConfig::builder()
            .id("market-candle-feed-id".to_owned())
            .name("binance-spot".to_owned())
            .feed_type(FeedType::MarketCandle)
            .tags(vec!["finance".to_owned(), "market-data".to_owned()])
            .transport(serde_json::json!({
                "url": "https://market-data.example/candles/latest",
                "interval_secs": 60,
                "venue": "binance",
                "symbols": ["BTCUSDT", "ETHUSDT"],
                "timeframes": ["15m", "1h"],
                "max_candles_per_poll": 1000
            }))
            .enabled(true)
            .status(FeedStatus::Idle)
            .created_at(Timestamp::UNIX_EPOCH)
            .updated_at(Timestamp::UNIX_EPOCH)
            .build();

        registry
            .register(config.clone())
            .expect("test config should register");
        start_feed_task(&config, &registry);

        assert!(registry.is_running("binance-spot"));

        registry
            .remove("binance-spot")
            .expect("test cleanup should cancel");
    }
}
