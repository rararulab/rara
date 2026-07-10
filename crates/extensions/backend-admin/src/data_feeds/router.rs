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

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post, put},
};
use jiff::Timestamp;
use rara_kernel::data_feed::{
    DataFeed, DataFeedConfig, DataFeedRegistry, FeedStatus, FeedType, parse_duration_ago,
    polling::PollingSource,
};
use rara_trading::feed::{
    catalog::{DefaultFeedSource, default_finance_feed_sources},
    market_candle::MarketCandleSource,
    rss::RssSource,
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

/// Built-in feed catalog entry plus current materialized state.
#[derive(Debug, Serialize)]
struct FeedCatalogEntryResponse {
    id:          String,
    name:        String,
    description: String,
    feed_type:   FeedType,
    tags:        Vec<String>,
    enabled:     bool,
    feed_id:     Option<String>,
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
) -> Result<Json<Vec<FeedCatalogEntryResponse>>, ProblemDetails> {
    let feeds = state.svc.list_feeds().await?;
    Ok(Json(catalog_response(&feeds)))
}

/// `POST /api/v1/data-feeds/catalog/{id}/enable` — materialize a default feed.
async fn enable_catalog_feed(
    State(state): State<DataFeedRouterState>,
    axum::Extension(principal): axum::Extension<
        rara_kernel::identity::Principal<rara_kernel::identity::Resolved>,
    >,
    Path(id): Path<String>,
) -> Result<(StatusCode, Json<DataFeedConfig>), ProblemDetails> {
    if !principal.is_admin() {
        return Err(ProblemDetails::forbidden(
            "enabling data feeds requires admin role",
        ));
    }

    let source = find_catalog_source(&id)?;
    let feed_name = source.feed_name();
    let existing = state
        .svc
        .list_feeds()
        .await?
        .into_iter()
        .find(|feed| feed.name == feed_name);

    let now = Timestamp::now();
    let (status, config) = if let Some(existing) = existing {
        let updated = DataFeedConfig::builder()
            .id(existing.id)
            .name(feed_name)
            .feed_type(source.feed_type)
            .tags(source.tags)
            .transport(source.transport)
            .maybe_auth(source.auth)
            .enabled(true)
            .status(FeedStatus::Idle)
            .created_at(existing.created_at)
            .updated_at(now)
            .build();
        state.svc.update_feed(&updated).await?;
        (StatusCode::OK, updated)
    } else {
        let created = DataFeedConfig::builder()
            .id(Uuid::new_v4().to_string())
            .name(feed_name)
            .feed_type(source.feed_type)
            .tags(source.tags)
            .transport(source.transport)
            .maybe_auth(source.auth)
            .enabled(true)
            .status(FeedStatus::Idle)
            .created_at(now)
            .updated_at(now)
            .build();
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

    let toggled = state.svc.toggle_feed(&id).await?;

    if !toggled {
        return Err(ProblemDetails::not_found(
            "Feed Not Found",
            format!("no feed with id: {id}"),
        ));
    }

    // Fetch updated config.
    let feed = state.svc.get_feed(&id).await?.ok_or_else(|| {
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
            registry.set_running(config.name.clone(), cancel.clone());

            let event_tx = registry.event_tx();
            let name = config.name.clone();
            let registry = registry.clone();

            tokio::spawn(async move {
                match source.run(event_tx, cancel).await {
                    Ok(()) => registry.clear_running(&name),
                    Err(err) => {
                        warn!(feed = %name, error = %err, "feed task exited with error");
                        registry.report_error(&name, err.to_string());
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
            registry.set_running(config.name.clone(), cancel.clone());

            let event_tx = registry.event_tx();
            let name = config.name.clone();
            let registry = registry.clone();

            tokio::spawn(async move {
                match source.run(event_tx, cancel).await {
                    Ok(()) => registry.clear_running(&name),
                    Err(err) => {
                        warn!(feed = %name, error = %err, "feed task exited with error");
                        registry.report_error(&name, err.to_string());
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

fn catalog_response(feeds: &[DataFeedConfig]) -> Vec<FeedCatalogEntryResponse> {
    default_finance_feed_sources()
        .into_iter()
        .map(|source| {
            let feed_name = source.feed_name();
            let feed = feeds.iter().find(|feed| feed.name == feed_name);
            FeedCatalogEntryResponse {
                id:          source.id,
                name:        source.name,
                description: source.description,
                feed_type:   source.feed_type,
                tags:        source.tags,
                enabled:     feed.is_some_and(|feed| feed.enabled),
                feed_id:     feed.map(|feed| feed.id.clone()),
            }
        })
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
        testing::build_memory_diesel_pools,
    };
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

    /// Build a router whose handlers use a real (but empty / schema-less)
    /// diesel pool. The non-admin Principal guard runs before any DB query,
    /// so the pool is never hit on the 403 path these tests exercise.
    async fn app_with_user(user: KernelUser) -> Router {
        let pools = build_memory_diesel_pools().await;
        bootstrap_data_feed_schema(&pools).await;
        let svc = DataFeedSvc::new(pools);
        let (event_tx, _event_rx) = tokio::sync::mpsc::channel(16);
        let registry = Arc::new(DataFeedRegistry::new(event_tx));
        let state = DataFeedRouterState { svc, registry };
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
