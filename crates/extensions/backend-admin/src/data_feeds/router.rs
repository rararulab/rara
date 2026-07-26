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
//! | POST   | `/api/v1/data-feeds/catalog/{id}/unsubscribe` | remove current user's catalog subscriptions |
//! | GET    | `/api/v1/data-feeds/finance/bundles` | list curated finance feed bundles |
//! | GET    | `/api/v1/data-feeds/finance/subscriptions` | list current user's finance subscriptions |
//! | POST   | `/api/v1/data-feeds/finance/subscriptions` | create/update current user's finance subscription |
//! | GET    | `/api/v1/data-feeds/market-data/candle-streams` | list stored market-data candle streams |
//! | GET    | `/api/v1/data-feeds/market-data/candles/recent` | query newest stored candles |
//! | GET    | `/api/v1/data-feeds/market-data/candles/freshness` | check stored candle freshness |
//! | GET    | `/api/v1/data-feeds/market-data/candles/gaps` | find missing stored candle open times |
//!
//! All mutations synchronise both the database (via [`DataFeedSvc`]) and the
//! in-memory [`DataFeedRegistry`]. When an active feed is created and
//! enabled, a background task is spawned automatically.

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

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
    session::SessionKey,
};
use rara_trading::{
    feed::{
        catalog::{
            DefaultFeedBundle, DefaultFeedSource, default_finance_feed_bundles,
            default_finance_feed_sources,
        },
        market_candle::{
            DEFAULT_MARKET_CANDLE_REQUEST_BUDGET_PER_SECOND, MarketCandleSource,
            market_candle_fanout_safety, unsafe_market_candle_fanout_message,
        },
        rss::RssSource,
    },
    finance::registry::{
        FinanceDelivery, FinanceEventKind, FinanceSubscription, FinanceSubscriptionRegistry,
    },
    market_data::{
        CandleLatestQuery, CandleRangeQuery, CandleRecentQuery, CandleStreamListQuery,
        CandleStreamSummary, MarketCandle, MarketDataRepositoryRef, Timeframe,
    },
};
use serde::{Deserialize, Deserializer, Serialize};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};
use uuid::Uuid;

use super::service::DataFeedSvc;
use crate::kernel::problem::ProblemDetails;

const DEFAULT_FINANCE_COOLDOWN_SECS: u64 = 900;
const DEFAULT_FINANCE_MAX_IMMEDIATE_PER_HOUR: u16 = 6;
const DEFAULT_MARKET_DATA_STREAM_LIMIT: usize = 500;
const MAX_MARKET_DATA_STREAM_LIMIT: usize = 10_000;
const DEFAULT_MARKET_DATA_CANDLE_LIMIT: usize = 500;
const MAX_MARKET_DATA_CANDLE_LIMIT: usize = 10_000;
const MAX_MARKET_DATA_CANDLE_RANGE_LIMIT: usize = MAX_MARKET_DATA_CANDLE_LIMIT - 1;
const MAX_MARKET_DATA_FRESHNESS_STALE_AFTER_SECS: u64 = 31_536_000;
const MAX_MARKET_DATA_SELECTOR_LEN: usize = 128;

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
    /// Shared market-data repository for closed OHLCV candle history.
    pub market_data_repo: MarketDataRepositoryRef,
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
            "/api/v1/data-feeds/catalog/{id}/unsubscribe",
            post(unsubscribe_catalog_feed),
        )
        .route(
            "/api/v1/data-feeds/finance/subscriptions",
            get(list_finance_subscriptions).post(create_finance_subscription),
        )
        .route(
            "/api/v1/data-feeds/finance/bundles",
            get(list_finance_feed_bundles),
        )
        .route(
            "/api/v1/data-feeds/finance/subscriptions/{id}",
            get(get_finance_subscription).delete(delete_finance_subscription),
        )
        .route(
            "/api/v1/data-feeds/market-data/candle-streams",
            get(list_market_data_candle_streams),
        )
        .route(
            "/api/v1/data-feeds/market-data/candles/latest",
            get(get_latest_market_data_candle),
        )
        .route(
            "/api/v1/data-feeds/market-data/candles/recent",
            get(get_recent_market_data_candles),
        )
        .route(
            "/api/v1/data-feeds/market-data/candles/freshness",
            get(get_market_data_candle_freshness),
        )
        .route(
            "/api/v1/data-feeds/market-data/candles/gaps",
            get(find_market_data_candle_gaps),
        )
        .route(
            "/api/v1/data-feeds/market-data/candles",
            get(query_market_data_candles),
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

/// Optional request body for removing current user's subscriptions to a
/// built-in catalog source. Empty means remove every current-user subscription
/// that explicitly names the source.
#[derive(Debug, Default, Deserialize)]
struct UnsubscribeCatalogFeedRequest {
    #[serde(default)]
    subscription_ids: Vec<Uuid>,
}

#[derive(Debug, Serialize)]
struct UnsubscribeCatalogFeedResponse {
    catalog_source_id:          String,
    source_name:                String,
    removed_subscription_ids:   Vec<Uuid>,
    removed_count:              usize,
    remaining_subscription_ids: Vec<Uuid>,
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
    since:       Option<String>,
    /// Optional comma-separated event-kind filter, e.g. rss_article or
    /// market_candle_closed.
    event_kinds: Option<String>,
    /// Maximum events to return (default: 50, max: 200).
    limit:       Option<i64>,
    /// Offset for pagination (default: 0).
    offset:      Option<i64>,
}

/// Query parameters for stored market-data candle streams.
#[derive(Debug, Deserialize)]
struct CandleStreamQueryParams {
    source_name: Option<String>,
    venue:       Option<String>,
    symbol:      Option<String>,
    timeframe:   Option<String>,
    limit:       Option<usize>,
    offset:      Option<usize>,
}

/// Stored market-data candle stream inventory response.
#[derive(Debug, Serialize)]
struct CandleStreamListResponse {
    streams:      Vec<CandleStreamResponse>,
    count:        usize,
    query_limit:  usize,
    query_offset: usize,
    has_more:     bool,
}

#[derive(Debug, Serialize)]
struct CandleStreamResponse {
    source_name:        String,
    venue:              String,
    symbol:             String,
    timeframe:          String,
    candle_count:       usize,
    first_open_time:    String,
    latest_open_time:   String,
    latest_close_time:  String,
    latest_ingested_at: String,
}

/// Query parameters for the latest stored closed candle.
#[derive(Debug, Deserialize)]
struct CandleLatestQueryParams {
    source_name: Option<String>,
    venue:       String,
    symbol:      String,
    timeframe:   String,
}

/// Query parameters for the newest stored closed candles.
#[derive(Debug, Deserialize)]
struct CandleRecentQueryParams {
    source_name: Option<String>,
    venue:       String,
    symbol:      String,
    timeframe:   String,
    limit:       Option<usize>,
    end:         Option<String>,
}

/// Query parameters for a bounded stored closed-candle range.
#[derive(Debug, Deserialize)]
struct CandleRangeQueryParams {
    source_name: Option<String>,
    venue:       String,
    symbol:      String,
    timeframe:   String,
    start:       String,
    end:         String,
    limit:       Option<usize>,
}

/// Query parameters for stored closed-candle freshness.
#[derive(Debug, Deserialize)]
struct CandleFreshnessQueryParams {
    source_name:      Option<String>,
    venue:            String,
    symbol:           String,
    timeframe:        String,
    as_of:            Option<String>,
    stale_after_secs: Option<u64>,
}

/// Query parameters for stored closed-candle gap detection.
#[derive(Debug, Deserialize)]
struct CandleGapsQueryParams {
    source_name: Option<String>,
    venue:       String,
    symbol:      String,
    timeframe:   String,
    start:       String,
    end:         String,
}

#[derive(Debug, Serialize)]
struct CandleLatestResponse {
    candle: Option<CandleResponse>,
}

#[derive(Debug, Serialize)]
struct CandleRangeResponse {
    candles:     Vec<CandleResponse>,
    count:       usize,
    query_limit: usize,
    has_more:    bool,
    next_start:  Option<String>,
}

#[derive(Debug, Serialize)]
struct CandleRecentResponse {
    candles:     Vec<CandleResponse>,
    count:       usize,
    query_limit: usize,
    has_more:    bool,
    next_end:    Option<String>,
}

#[derive(Debug, Serialize)]
struct CandleFreshnessResponse {
    latest:           Option<CandleResponse>,
    as_of:            String,
    stale_after_secs: u64,
    lag_secs:         Option<i64>,
    is_stale:         bool,
    status:           String,
}

#[derive(Debug, Serialize)]
struct CandleGapsResponse {
    missing_open_times: Vec<String>,
    missing_count:      usize,
    expected_count:     usize,
    complete:           bool,
}

#[derive(Debug, Serialize)]
struct CandleResponse {
    source_name:       String,
    venue:             String,
    symbol:            String,
    timeframe:         String,
    open_time:         String,
    close_time:        String,
    open:              String,
    high:              String,
    low:               String,
    close:             String,
    volume:            String,
    ingested_at:       String,
    provider_sequence: Option<String>,
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
    feed_id:         String,
    source_name:     String,
    event_count:     i64,
    last_event_type: Option<String>,
    last_event_at:   Option<Timestamp>,
    lag_seconds:     Option<i64>,
}

/// Built-in feed catalog entry plus current materialized state.
#[derive(Debug, Clone, Serialize)]
struct FeedCatalogEntryResponse {
    id:                     String,
    name:                   String,
    description:            String,
    feed_type:              FeedType,
    provider:               Option<String>,
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
    load:                   FeedCatalogLoadResponse,
}

/// Current user's finance subscription status for a built-in feed source.
#[derive(Debug, Clone, Serialize)]
struct FeedCatalogSubscriptionResponse {
    user_subscribed:       bool,
    user_subscription_ids: Vec<Uuid>,
}

#[derive(Debug, Clone, Serialize)]
struct FeedCatalogLoadResponse {
    user_subscription_count: usize,
    subscribed_market_stream_count: usize,
    configured_market_stream_count: Option<usize>,
    configured_market_poll_request_count: Option<usize>,
    configured_market_requests_per_second: Option<f64>,
    configured_market_request_budget_per_second: Option<f64>,
    configured_market_minimum_safe_interval_secs: Option<u64>,
    configured_market_fanout_safe_to_start: Option<bool>,
    configured_market_fanout_diagnostic: Option<String>,
}

#[derive(Debug, Serialize)]
struct FinanceFeedBundleListResponse {
    bundles: Vec<FinanceFeedBundleResponse>,
    count:   usize,
}

#[derive(Debug, Serialize)]
struct FinanceFeedBundleResponse {
    id:                     String,
    name:                   String,
    description:            String,
    tags:                   Vec<String>,
    catalog_source_ids:     Vec<String>,
    feed_types:             Vec<FeedType>,
    providers:              Vec<String>,
    source_count:           usize,
    enabled_source_count:   usize,
    ready_source_count:     usize,
    requires_configuration: bool,
    can_enable:             bool,
    sources:                Vec<FeedCatalogEntryResponse>,
    subscriptions:          FeedCatalogSubscriptionResponse,
}

#[derive(Debug, Serialize)]
struct FinanceSubscriptionListResponse {
    subscriptions: Vec<FinanceSubscriptionResponse>,
    count:         usize,
}

#[derive(Debug, Deserialize)]
struct CreateFinanceSubscriptionRequest {
    session_key:            String,
    #[serde(default)]
    event_kinds:            Vec<FinanceEventKind>,
    #[serde(default)]
    catalog_source_ids:     Vec<String>,
    #[serde(default)]
    source_names:           Vec<String>,
    #[serde(default)]
    match_all_sources:      bool,
    #[serde(default)]
    category_tags:          Vec<String>,
    #[serde(default)]
    watch_terms:            Vec<String>,
    #[serde(default)]
    venues:                 Vec<String>,
    #[serde(default)]
    symbols:                Vec<String>,
    #[serde(default)]
    timeframes:             Vec<String>,
    delivery:               Option<FinanceDelivery>,
    cooldown_secs:          Option<u64>,
    max_immediate_per_hour: Option<u16>,
}

#[derive(Debug, Serialize)]
struct CreateFinanceSubscriptionResponse {
    subscription: FinanceSubscriptionResponse,
    created:      bool,
}

#[derive(Debug, Serialize)]
struct FinanceSubscriptionResponse {
    subscription_id:        Uuid,
    session_key:            String,
    event_kinds:            Vec<FinanceEventKind>,
    source_names:           Vec<String>,
    matches_all_sources:    bool,
    sources:                Vec<FinanceSubscriptionSourceResponse>,
    category_tags:          Vec<String>,
    watch_terms:            Vec<String>,
    venues:                 Vec<String>,
    symbols:                Vec<String>,
    timeframes:             Vec<String>,
    delivery:               FinanceDelivery,
    cooldown_secs:          u64,
    max_immediate_per_hour: u16,
}

#[derive(Debug, Serialize)]
struct FinanceSubscriptionSourceResponse {
    source_name:       String,
    catalog_source_id: Option<String>,
    catalog_name:      Option<String>,
    provider:          Option<String>,
    feed_id:           Option<String>,
    feed_type:         Option<FeedType>,
    enabled:           Option<bool>,
    status:            Option<FeedStatus>,
}

#[derive(Debug, Serialize)]
struct DeleteFinanceSubscriptionResponse {
    subscription_id: Uuid,
    removed:         bool,
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

/// `GET /api/v1/data-feeds/finance/bundles` — list curated finance feed
/// bundles expanded to catalog source entries plus current-user subscription
/// status.
async fn list_finance_feed_bundles(
    State(state): State<DataFeedRouterState>,
    axum::Extension(principal): axum::Extension<Principal<Resolved>>,
) -> Result<Json<FinanceFeedBundleListResponse>, ProblemDetails> {
    let feeds = state.svc.list_feeds().await?;
    let owner = UserId(principal.user_id.0);
    let subscriptions = state.finance_registry.list_for_owner(&owner).await;
    let catalog = catalog_response(&feeds, &subscriptions)
        .into_iter()
        .map(|entry| (entry.id.clone(), entry))
        .collect::<HashMap<_, _>>();
    let bundles = default_finance_feed_bundles()
        .into_iter()
        .map(|bundle| finance_feed_bundle_response(bundle, &catalog, &subscriptions))
        .collect::<Vec<_>>();

    Ok(Json(FinanceFeedBundleListResponse {
        count: bundles.len(),
        bundles,
    }))
}

/// `GET /api/v1/data-feeds/finance/subscriptions` — list current user's
/// finance information subscriptions.
async fn list_finance_subscriptions(
    State(state): State<DataFeedRouterState>,
    axum::Extension(principal): axum::Extension<Principal<Resolved>>,
) -> Result<Json<FinanceSubscriptionListResponse>, ProblemDetails> {
    let feeds = state.svc.list_feeds().await?;
    let catalog_by_source_name = catalog_by_source_name();
    let owner = UserId(principal.user_id.0);
    let subscriptions = state
        .finance_registry
        .list_for_owner(&owner)
        .await
        .into_iter()
        .map(|subscription| {
            finance_subscription_response(subscription, &feeds, &catalog_by_source_name)
        })
        .collect::<Vec<_>>();

    Ok(Json(FinanceSubscriptionListResponse {
        count: subscriptions.len(),
        subscriptions,
    }))
}

/// `POST /api/v1/data-feeds/finance/subscriptions` — create or update a
/// current-user finance subscription for an explicit session key.
async fn create_finance_subscription(
    State(state): State<DataFeedRouterState>,
    axum::Extension(principal): axum::Extension<Principal<Resolved>>,
    Json(request): Json<CreateFinanceSubscriptionRequest>,
) -> Result<(StatusCode, Json<CreateFinanceSubscriptionResponse>), ProblemDetails> {
    let session_key = SessionKey::try_from_raw(&request.session_key)
        .map_err(|e| ProblemDetails::bad_request(format!("invalid session_key: {e}")))?;
    let owner = UserId(principal.user_id.0);
    let source_names = resolve_finance_subscription_source_names(
        &request.catalog_source_ids,
        &request.source_names,
        request.match_all_sources,
    )?;
    let event_kinds = resolve_finance_subscription_event_kinds(&request)?;
    let delivery = request.delivery.unwrap_or(FinanceDelivery::Silent);
    let cooldown_secs = request
        .cooldown_secs
        .unwrap_or(DEFAULT_FINANCE_COOLDOWN_SECS);
    if cooldown_secs > 86_400 {
        return Err(ProblemDetails::bad_request(
            "cooldown_secs must be <= 86400",
        ));
    }
    let max_immediate_per_hour = request
        .max_immediate_per_hour
        .unwrap_or(DEFAULT_FINANCE_MAX_IMMEDIATE_PER_HOUR);
    if max_immediate_per_hour > 60 {
        return Err(ProblemDetails::bad_request(
            "max_immediate_per_hour must be <= 60",
        ));
    }

    let existing = state
        .finance_registry
        .list_for_owner(&owner)
        .await
        .into_iter()
        .find(|subscription| {
            subscription.session_key == session_key
                && same_finance_event_kinds(&subscription.event_kinds, &event_kinds)
                && same_string_set(&subscription.source_names, &source_names)
                && same_string_set(&subscription.category_tags, &request.category_tags)
                && same_string_set(&subscription.watch_terms, &request.watch_terms)
                && same_string_set(&subscription.venues, &request.venues)
                && same_string_set(&subscription.symbols, &request.symbols)
                && same_string_set(&subscription.timeframes, &request.timeframes)
        });
    let created = existing.is_none();
    let id = existing.as_ref().map_or_else(Uuid::new_v4, |sub| sub.id);
    let subscription = FinanceSubscription {
        id,
        owner: owner.clone(),
        session_key,
        event_kinds,
        source_names,
        category_tags: request.category_tags,
        watch_terms: request.watch_terms,
        venues: request.venues,
        symbols: request.symbols,
        timeframes: request.timeframes,
        delivery,
        cooldown_secs,
        max_immediate_per_hour,
    };

    let id = state
        .finance_registry
        .upsert(subscription)
        .await
        .map_err(|err| {
            ProblemDetails::internal(format!("failed to create finance subscription: {err}"))
        })?;
    let feeds = state.svc.list_feeds().await?;
    let catalog_by_source_name = catalog_by_source_name();
    let subscription = state
        .finance_registry
        .list_for_owner(&owner)
        .await
        .into_iter()
        .find(|subscription| subscription.id == id)
        .ok_or_else(|| {
            ProblemDetails::internal("created finance subscription was not persisted")
        })?;
    let status = if created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };

    Ok((
        status,
        Json(CreateFinanceSubscriptionResponse {
            subscription: finance_subscription_response(
                subscription,
                &feeds,
                &catalog_by_source_name,
            ),
            created,
        }),
    ))
}

/// `GET /api/v1/data-feeds/finance/subscriptions/{id}` — fetch one current-user
/// finance information subscription.
async fn get_finance_subscription(
    State(state): State<DataFeedRouterState>,
    axum::Extension(principal): axum::Extension<Principal<Resolved>>,
    Path(id): Path<Uuid>,
) -> Result<Json<FinanceSubscriptionResponse>, ProblemDetails> {
    let feeds = state.svc.list_feeds().await?;
    let catalog_by_source_name = catalog_by_source_name();
    let owner = UserId(principal.user_id.0);
    let subscription = state
        .finance_registry
        .list_for_owner(&owner)
        .await
        .into_iter()
        .find(|subscription| subscription.id == id)
        .ok_or_else(|| finance_subscription_not_found(id))?;

    Ok(Json(finance_subscription_response(
        subscription,
        &feeds,
        &catalog_by_source_name,
    )))
}

/// `DELETE /api/v1/data-feeds/finance/subscriptions/{id}` — remove one
/// current-user finance information subscription.
async fn delete_finance_subscription(
    State(state): State<DataFeedRouterState>,
    axum::Extension(principal): axum::Extension<Principal<Resolved>>,
    Path(id): Path<Uuid>,
) -> Result<Json<DeleteFinanceSubscriptionResponse>, ProblemDetails> {
    let owner = UserId(principal.user_id.0);
    let removed = state
        .finance_registry
        .remove(&owner, id)
        .await
        .map_err(|err| {
            ProblemDetails::internal(format!("failed to remove finance subscription: {err}"))
        })?;

    if !removed {
        return Err(finance_subscription_not_found(id));
    }

    Ok(Json(DeleteFinanceSubscriptionResponse {
        subscription_id: id,
        removed,
    }))
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
                    last_event_type: summary.and_then(|summary| summary.last_event_type.clone()),
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

/// `POST /api/v1/data-feeds/catalog/{id}/unsubscribe` — remove the current
/// user's finance subscriptions that explicitly name this built-in source.
async fn unsubscribe_catalog_feed(
    State(state): State<DataFeedRouterState>,
    axum::Extension(principal): axum::Extension<Principal<Resolved>>,
    Path(id): Path<String>,
    body: Bytes,
) -> Result<Json<UnsubscribeCatalogFeedResponse>, ProblemDetails> {
    let source = find_catalog_source(&id)?;
    let source_name = source.feed_name();
    let request = parse_unsubscribe_catalog_feed_request(&body)?;
    let owner = UserId(principal.user_id.0);
    let subscriptions = state.finance_registry.list_for_owner(&owner).await;
    let mut matched_ids = subscriptions
        .iter()
        .filter(|subscription| {
            subscription
                .source_names
                .iter()
                .any(|name| name == &source_name)
        })
        .map(|subscription| subscription.id)
        .collect::<Vec<_>>();

    if !request.subscription_ids.is_empty() {
        matched_ids.retain(|id| request.subscription_ids.contains(id));
    }

    let mut removed_subscription_ids = Vec::new();
    for subscription_id in matched_ids {
        if state
            .finance_registry
            .remove(&owner, subscription_id)
            .await
            .map_err(|err| {
                ProblemDetails::internal(format!("failed to remove finance subscription: {err}"))
            })?
        {
            removed_subscription_ids.push(subscription_id);
        }
    }

    let remaining_subscription_ids = state
        .finance_registry
        .list_for_owner(&owner)
        .await
        .into_iter()
        .filter(|subscription| {
            subscription
                .source_names
                .iter()
                .any(|name| name == &source_name)
        })
        .map(|subscription| subscription.id)
        .collect::<Vec<_>>();

    Ok(Json(UnsubscribeCatalogFeedResponse {
        catalog_source_id: id,
        source_name,
        removed_count: removed_subscription_ids.len(),
        removed_subscription_ids,
        remaining_subscription_ids,
    }))
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
    let event_types = parse_event_kind_filter(params.event_kinds.as_deref())?
        .into_iter()
        .map(finance_event_kind_type)
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();

    let page = state
        .svc
        .query_events(&feed.name, since, &event_types, limit, offset)
        .await?;

    Ok(Json(EventListResponse {
        events:   page.events,
        total:    page.total,
        has_more: page.has_more,
    }))
}

/// `GET /api/v1/data-feeds/market-data/candle-streams` — list stored closed
/// candle streams from the market-data repository.
async fn list_market_data_candle_streams(
    State(state): State<DataFeedRouterState>,
    Query(params): Query<CandleStreamQueryParams>,
) -> Result<Json<CandleStreamListResponse>, ProblemDetails> {
    let limit = validate_market_data_stream_limit(params.limit)?;
    let offset = params.offset.unwrap_or(0);
    let probe_limit = limit.saturating_add(1);
    let query = CandleStreamListQuery {
        source_name: normalize_optional_market_data_selector("source_name", params.source_name)?,
        venue: normalize_optional_market_data_venue(params.venue)?,
        symbol: normalize_optional_market_data_symbol(params.symbol)?,
        timeframe: normalize_optional_market_data_timeframe(params.timeframe)?,
        limit: probe_limit,
        offset,
    };

    let mut streams = state
        .market_data_repo
        .candle_streams(query)
        .await
        .map_err(|err| ProblemDetails::internal(format!("failed to list candle streams: {err}")))?;
    let has_more = streams.len() > limit;
    streams.truncate(limit);
    let count = streams.len();

    Ok(Json(CandleStreamListResponse {
        streams: streams
            .into_iter()
            .map(CandleStreamResponse::from)
            .collect(),
        count,
        query_limit: limit,
        query_offset: offset,
        has_more,
    }))
}

/// `GET /api/v1/data-feeds/market-data/candles/latest` — fetch the newest
/// stored closed candle for a stream.
async fn get_latest_market_data_candle(
    State(state): State<DataFeedRouterState>,
    Query(params): Query<CandleLatestQueryParams>,
) -> Result<Json<CandleLatestResponse>, ProblemDetails> {
    let query = CandleLatestQuery {
        source_name: normalize_optional_market_data_selector("source_name", params.source_name)?,
        venue:       normalize_required_market_data_venue(params.venue)?,
        symbol:      normalize_required_market_data_symbol(params.symbol)?,
        timeframe:   normalize_required_market_data_timeframe(params.timeframe)?,
    };

    let candle = state
        .market_data_repo
        .latest_closed_candle(query)
        .await
        .map_err(|err| ProblemDetails::internal(format!("failed to get latest candle: {err}")))?;

    Ok(Json(CandleLatestResponse {
        candle: candle.map(CandleResponse::from),
    }))
}

/// `GET /api/v1/data-feeds/market-data/candles/recent` — fetch the newest
/// stored closed candles for a stream, ordered oldest to newest.
async fn get_recent_market_data_candles(
    State(state): State<DataFeedRouterState>,
    Query(params): Query<CandleRecentQueryParams>,
) -> Result<Json<CandleRecentResponse>, ProblemDetails> {
    let limit = validate_market_data_candle_range_limit(params.limit)?;
    let probe_limit = limit.saturating_add(1);
    let end = params
        .end
        .as_deref()
        .map(|value| parse_market_data_timestamp("end", value.to_owned()))
        .transpose()?;
    let query = CandleRecentQuery {
        source_name: normalize_optional_market_data_selector("source_name", params.source_name)?,
        venue: normalize_required_market_data_venue(params.venue)?,
        symbol: normalize_required_market_data_symbol(params.symbol)?,
        timeframe: normalize_required_market_data_timeframe(params.timeframe)?,
        limit: probe_limit,
        end,
    };

    let mut candles = state
        .market_data_repo
        .recent_candles(query)
        .await
        .map_err(|err| ProblemDetails::internal(format!("failed to get recent candles: {err}")))?;
    let has_more = candles.len() > limit;
    if has_more {
        candles.remove(0);
    }
    let next_end = candles
        .first()
        .filter(|_| has_more)
        .map(|candle| candle.open_time.to_string());
    let count = candles.len();

    Ok(Json(CandleRecentResponse {
        candles: candles.into_iter().map(CandleResponse::from).collect(),
        count,
        query_limit: limit,
        has_more,
        next_end,
    }))
}

/// `GET /api/v1/data-feeds/market-data/candles/freshness` — report whether
/// the newest stored closed candle is fresh relative to `as_of`.
async fn get_market_data_candle_freshness(
    State(state): State<DataFeedRouterState>,
    Query(params): Query<CandleFreshnessQueryParams>,
) -> Result<Json<CandleFreshnessResponse>, ProblemDetails> {
    let timeframe = normalize_required_market_data_timeframe(params.timeframe)?;
    let default_stale_after_secs = u64::try_from(
        timeframe
            .step()
            .map_err(|err| ProblemDetails::bad_request(format!("invalid timeframe: {err}")))?
            .as_secs(),
    )
    .map_err(|err| ProblemDetails::bad_request(format!("invalid timeframe step: {err}")))?
    .saturating_mul(2);
    let stale_after_secs = params.stale_after_secs.unwrap_or(default_stale_after_secs);
    validate_market_data_stale_after_secs(stale_after_secs)?;
    let as_of = params
        .as_of
        .map(|value| parse_market_data_timestamp("as_of", value))
        .transpose()?
        .unwrap_or_else(Timestamp::now);

    let query = CandleLatestQuery {
        source_name: normalize_optional_market_data_selector("source_name", params.source_name)?,
        venue: normalize_required_market_data_venue(params.venue)?,
        symbol: normalize_required_market_data_symbol(params.symbol)?,
        timeframe,
    };
    let latest = state
        .market_data_repo
        .latest_closed_candle(query)
        .await
        .map_err(|err| {
            ProblemDetails::internal(format!("failed to get candle freshness: {err}"))
        })?;
    let Some(candle) = latest else {
        return Ok(Json(CandleFreshnessResponse {
            latest: None,
            as_of: as_of.to_string(),
            stale_after_secs,
            lag_secs: None,
            is_stale: true,
            status: "missing".to_owned(),
        }));
    };

    let lag_secs = as_of.as_second() - candle.close_time.as_second();
    let is_stale = lag_secs >= 0 && lag_secs as u64 > stale_after_secs;
    let status = if lag_secs < 0 {
        "future"
    } else if is_stale {
        "stale"
    } else {
        "fresh"
    };

    Ok(Json(CandleFreshnessResponse {
        latest: Some(CandleResponse::from(candle)),
        as_of: as_of.to_string(),
        stale_after_secs,
        lag_secs: Some(lag_secs),
        is_stale,
        status: status.to_owned(),
    }))
}

/// `GET /api/v1/data-feeds/market-data/candles/gaps` — find missing expected
/// open times for a stored closed-candle stream.
async fn find_market_data_candle_gaps(
    State(state): State<DataFeedRouterState>,
    Query(params): Query<CandleGapsQueryParams>,
) -> Result<Json<CandleGapsResponse>, ProblemDetails> {
    let timeframe = normalize_required_market_data_timeframe(params.timeframe)?;
    let start = parse_market_data_timestamp("start", params.start)?;
    let end = parse_market_data_timestamp("end", params.end)?;
    if end <= start {
        return Err(ProblemDetails::bad_request("end must be after start"));
    }
    let expected_count = expected_market_data_open_time_count(&timeframe, start, end)?;

    let query = CandleRangeQuery {
        source_name: normalize_optional_market_data_selector("source_name", params.source_name)?,
        venue: normalize_required_market_data_venue(params.venue)?,
        symbol: normalize_required_market_data_symbol(params.symbol)?,
        timeframe,
        start,
        end,
        limit: expected_count,
    };
    let missing = state
        .market_data_repo
        .missing_open_times(query)
        .await
        .map_err(|err| ProblemDetails::internal(format!("failed to find candle gaps: {err}")))?;
    let missing_count = missing.len();

    Ok(Json(CandleGapsResponse {
        missing_open_times: missing.into_iter().map(|ts| ts.to_string()).collect(),
        missing_count,
        expected_count,
        complete: missing_count == 0,
    }))
}

/// `GET /api/v1/data-feeds/market-data/candles` — query a bounded ordered
/// range of stored closed candles for a stream.
async fn query_market_data_candles(
    State(state): State<DataFeedRouterState>,
    Query(params): Query<CandleRangeQueryParams>,
) -> Result<Json<CandleRangeResponse>, ProblemDetails> {
    let limit = validate_market_data_candle_range_limit(params.limit)?;
    let probe_limit = limit.saturating_add(1);
    let start = parse_market_data_timestamp("start", params.start)?;
    let end = parse_market_data_timestamp("end", params.end)?;
    if end <= start {
        return Err(ProblemDetails::bad_request("end must be after start"));
    }

    let query = CandleRangeQuery {
        source_name: normalize_optional_market_data_selector("source_name", params.source_name)?,
        venue: normalize_required_market_data_venue(params.venue)?,
        symbol: normalize_required_market_data_symbol(params.symbol)?,
        timeframe: normalize_required_market_data_timeframe(params.timeframe)?,
        start,
        end,
        limit: probe_limit,
    };

    let mut candles = state
        .market_data_repo
        .candles(query)
        .await
        .map_err(|err| ProblemDetails::internal(format!("failed to query candles: {err}")))?;
    let has_more = candles.len() > limit;
    let next_start = candles
        .get(limit)
        .map(|candle| candle.open_time.to_string());
    candles.truncate(limit);
    let count = candles.len();

    Ok(Json(CandleRangeResponse {
        candles: candles.into_iter().map(CandleResponse::from).collect(),
        count,
        query_limit: limit,
        has_more,
        next_start,
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

impl From<CandleStreamSummary> for CandleStreamResponse {
    fn from(summary: CandleStreamSummary) -> Self {
        Self {
            source_name:        summary.source_name,
            venue:              summary.venue,
            symbol:             summary.symbol,
            timeframe:          summary.timeframe.to_string(),
            candle_count:       summary.candle_count,
            first_open_time:    summary.first_open_time.to_string(),
            latest_open_time:   summary.latest_open_time.to_string(),
            latest_close_time:  summary.latest_close_time.to_string(),
            latest_ingested_at: summary.latest_ingested_at.to_string(),
        }
    }
}

impl From<MarketCandle> for CandleResponse {
    fn from(candle: MarketCandle) -> Self {
        Self {
            source_name:       candle.source_name,
            venue:             candle.venue,
            symbol:            candle.symbol,
            timeframe:         candle.timeframe.to_string(),
            open_time:         candle.open_time.to_string(),
            close_time:        candle.close_time.to_string(),
            open:              candle.open.to_string(),
            high:              candle.high.to_string(),
            low:               candle.low.to_string(),
            close:             candle.close.to_string(),
            volume:            candle.volume.to_string(),
            ingested_at:       candle.ingested_at.to_string(),
            provider_sequence: candle.provider_sequence,
        }
    }
}

fn normalize_market_data_selector(name: &str, value: String) -> Result<String, ProblemDetails> {
    let value = value.trim();
    if value.is_empty() {
        return Err(ProblemDetails::bad_request(format!(
            "{name} must not be empty"
        )));
    }
    if value.chars().count() > MAX_MARKET_DATA_SELECTOR_LEN {
        return Err(ProblemDetails::bad_request(format!("{name} is too long")));
    }
    Ok(value.to_owned())
}

fn normalize_optional_market_data_selector(
    name: &str,
    value: Option<String>,
) -> Result<Option<String>, ProblemDetails> {
    value
        .map(|value| normalize_market_data_selector(name, value))
        .transpose()
}

fn normalize_required_market_data_venue(value: String) -> Result<String, ProblemDetails> {
    normalize_market_data_selector("venue", value).map(|value| value.to_ascii_lowercase())
}

fn normalize_required_market_data_symbol(value: String) -> Result<String, ProblemDetails> {
    normalize_market_data_selector("symbol", value).map(|value| value.to_ascii_uppercase())
}

fn normalize_required_market_data_timeframe(value: String) -> Result<Timeframe, ProblemDetails> {
    let value = normalize_market_data_selector("timeframe", value)?;
    Timeframe::parse(value.to_ascii_lowercase())
        .map_err(|err| ProblemDetails::bad_request(format!("invalid timeframe: {err}")))
}

fn normalize_optional_market_data_venue(
    value: Option<String>,
) -> Result<Option<String>, ProblemDetails> {
    value
        .map(|value| normalize_market_data_selector("venue", value).map(|v| v.to_ascii_lowercase()))
        .transpose()
}

fn normalize_optional_market_data_symbol(
    value: Option<String>,
) -> Result<Option<String>, ProblemDetails> {
    value
        .map(|value| {
            normalize_market_data_selector("symbol", value).map(|v| v.to_ascii_uppercase())
        })
        .transpose()
}

fn normalize_optional_market_data_timeframe(
    value: Option<String>,
) -> Result<Option<Timeframe>, ProblemDetails> {
    value
        .map(|value| {
            let value = normalize_market_data_selector("timeframe", value)?;
            Timeframe::parse(value.to_ascii_lowercase())
                .map_err(|err| ProblemDetails::bad_request(format!("invalid timeframe: {err}")))
        })
        .transpose()
}

fn validate_market_data_stream_limit(limit: Option<usize>) -> Result<usize, ProblemDetails> {
    let limit = limit.unwrap_or(DEFAULT_MARKET_DATA_STREAM_LIMIT);
    if limit == 0 {
        return Err(ProblemDetails::bad_request("limit must be positive"));
    }
    if limit > MAX_MARKET_DATA_STREAM_LIMIT {
        return Err(ProblemDetails::bad_request(format!(
            "limit must be <= {MAX_MARKET_DATA_STREAM_LIMIT}"
        )));
    }
    Ok(limit)
}

fn validate_market_data_candle_range_limit(limit: Option<usize>) -> Result<usize, ProblemDetails> {
    let limit = limit.unwrap_or(DEFAULT_MARKET_DATA_CANDLE_LIMIT);
    if limit == 0 {
        return Err(ProblemDetails::bad_request("limit must be positive"));
    }
    if limit > MAX_MARKET_DATA_CANDLE_RANGE_LIMIT {
        return Err(ProblemDetails::bad_request(format!(
            "limit must be <= {MAX_MARKET_DATA_CANDLE_RANGE_LIMIT}"
        )));
    }
    Ok(limit)
}

fn validate_market_data_stale_after_secs(value: u64) -> Result<(), ProblemDetails> {
    if value == 0 {
        return Err(ProblemDetails::bad_request(
            "stale_after_secs must be positive",
        ));
    }
    if value > MAX_MARKET_DATA_FRESHNESS_STALE_AFTER_SECS {
        return Err(ProblemDetails::bad_request(format!(
            "stale_after_secs must be <= {MAX_MARKET_DATA_FRESHNESS_STALE_AFTER_SECS}"
        )));
    }
    Ok(())
}

fn expected_market_data_open_time_count(
    timeframe: &Timeframe,
    start: Timestamp,
    end: Timestamp,
) -> Result<usize, ProblemDetails> {
    let step = timeframe
        .step()
        .map_err(|err| ProblemDetails::bad_request(format!("invalid timeframe: {err}")))?;
    let mut cursor = start;
    let mut count = 0usize;

    while cursor < end {
        count = count.saturating_add(1);
        if count > MAX_MARKET_DATA_CANDLE_LIMIT {
            return Err(ProblemDetails::bad_request(format!(
                "range contains more than {MAX_MARKET_DATA_CANDLE_LIMIT} expected candles"
            )));
        }
        cursor = cursor.checked_add(step).map_err(|err| {
            ProblemDetails::bad_request(format!("timeframe addition overflowed: {err}"))
        })?;
    }

    Ok(count)
}

fn parse_market_data_timestamp(name: &str, value: String) -> Result<Timestamp, ProblemDetails> {
    let value = normalize_market_data_selector(name, value)?;
    value
        .parse::<Timestamp>()
        .map_err(|err| ProblemDetails::bad_request(format!("invalid {name}: {err}")))
}
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
            let provider = source
                .provider
                .clone()
                .or_else(|| catalog_transport_string(source.transport.as_ref(), "provider"));
            let venue = catalog_transport_string(source.transport.as_ref(), "venue");
            let configured_symbols =
                catalog_transport_string_list(source.transport.as_ref(), "symbols");
            let configured_timeframes =
                catalog_transport_string_list(source.transport.as_ref(), "timeframes");
            let load = catalog_load_response(
                subscriptions,
                &feed_name,
                source.feed_type,
                feed.map(|feed| &feed.transport)
                    .or(source.transport.as_ref()),
            );
            FeedCatalogEntryResponse {
                id: source.id,
                name: source.name,
                description: source.description,
                feed_type: source.feed_type,
                provider,
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
                load,
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

fn catalog_load_response(
    subscriptions: &[FinanceSubscription],
    source_name: &str,
    feed_type: FeedType,
    transport: Option<&serde_json::Value>,
) -> FeedCatalogLoadResponse {
    let mut user_subscription_count = 0;
    let mut market_streams = HashSet::new();

    for subscription in subscriptions {
        if !subscription
            .source_names
            .iter()
            .any(|name| name == source_name)
        {
            continue;
        }

        user_subscription_count += 1;
        if feed_type == FeedType::MarketCandle
            && subscription
                .event_kinds
                .contains(&FinanceEventKind::MarketCandleClosed)
        {
            insert_market_subscription_streams(subscription, &mut market_streams);
        }
    }

    let mut load = FeedCatalogLoadResponse {
        user_subscription_count,
        subscribed_market_stream_count: market_streams.len(),
        configured_market_stream_count: None,
        configured_market_poll_request_count: None,
        configured_market_requests_per_second: None,
        configured_market_request_budget_per_second: None,
        configured_market_minimum_safe_interval_secs: None,
        configured_market_fanout_safe_to_start: None,
        configured_market_fanout_diagnostic: None,
    };

    if feed_type == FeedType::MarketCandle {
        apply_configured_market_fanout_load(&mut load, transport);
    }

    load
}

fn insert_market_subscription_streams(
    subscription: &FinanceSubscription,
    market_streams: &mut HashSet<(String, String, String)>,
) {
    if subscription.symbols.is_empty() || subscription.timeframes.is_empty() {
        return;
    }
    let venues = if subscription.venues.is_empty() {
        vec![String::new()]
    } else {
        subscription.venues.clone()
    };
    for venue in &venues {
        for symbol in &subscription.symbols {
            for timeframe in &subscription.timeframes {
                market_streams.insert((venue.clone(), symbol.clone(), timeframe.clone()));
            }
        }
    }
}

fn apply_configured_market_fanout_load(
    load: &mut FeedCatalogLoadResponse,
    transport: Option<&serde_json::Value>,
) {
    let Some(transport) = transport else {
        load.configured_market_fanout_diagnostic =
            Some("market candle source has no transport config".to_owned());
        return;
    };
    let provider = transport
        .get("provider")
        .and_then(serde_json::Value::as_str);
    let symbols = catalog_transport_string_list(Some(transport), "symbols")
        .into_iter()
        .map(|symbol| symbol.to_ascii_uppercase())
        .collect::<Vec<_>>();
    let timeframes = catalog_transport_string_list(Some(transport), "timeframes")
        .into_iter()
        .map(|timeframe| timeframe.to_ascii_lowercase())
        .collect::<Vec<_>>();
    match market_candle_fanout_safety(
        provider,
        transport,
        &symbols,
        &timeframes,
        DEFAULT_MARKET_CANDLE_REQUEST_BUDGET_PER_SECOND,
    ) {
        Ok(safety) => {
            load.configured_market_stream_count = Some(safety.stream_count);
            load.configured_market_poll_request_count = Some(safety.poll_request_count);
            load.configured_market_requests_per_second = Some(safety.estimated_requests_per_second);
            load.configured_market_request_budget_per_second =
                Some(safety.request_budget_per_second);
            load.configured_market_minimum_safe_interval_secs =
                Some(safety.minimum_safe_interval_secs);
            load.configured_market_fanout_safe_to_start = Some(safety.safe_to_start);
            if !safety.safe_to_start {
                load.configured_market_fanout_diagnostic =
                    Some(unsafe_market_candle_fanout_message(&safety));
            }
        }
        Err(err) => {
            load.configured_market_fanout_diagnostic = Some(err.to_string());
        }
    }
}

fn finance_feed_bundle_response(
    bundle: DefaultFeedBundle,
    catalog: &HashMap<String, FeedCatalogEntryResponse>,
    subscriptions: &[FinanceSubscription],
) -> FinanceFeedBundleResponse {
    let sources = bundle
        .catalog_source_ids
        .iter()
        .filter_map(|source_id| catalog.get(source_id).cloned())
        .collect::<Vec<_>>();
    let feed_types =
        sources
            .iter()
            .map(|source| source.feed_type)
            .fold(Vec::new(), |mut values, feed_type| {
                if !values.contains(&feed_type) {
                    values.push(feed_type);
                }
                values
            });
    let providers = sources
        .iter()
        .filter_map(|source| source.provider.clone())
        .fold(Vec::new(), |mut values, provider| {
            if !values.contains(&provider) {
                values.push(provider);
            }
            values
        });
    let enabled_source_count = sources.iter().filter(|source| source.enabled).count();
    let ready_source_count = sources
        .iter()
        .filter(|source| !source.requires_configuration && source.transport_template.is_some())
        .count();
    let requires_configuration = sources.iter().any(|source| source.requires_configuration);
    let can_enable = sources.len() == bundle.catalog_source_ids.len()
        && sources.iter().all(|source| {
            source.enabled
                || (!source.requires_configuration && source.transport_template.is_some())
        });
    let source_names = sources
        .iter()
        .map(|source| source.source_name.as_str())
        .collect::<Vec<_>>();

    FinanceFeedBundleResponse {
        id: bundle.id,
        name: bundle.name,
        description: bundle.description,
        tags: bundle.tags,
        catalog_source_ids: bundle.catalog_source_ids,
        feed_types,
        providers,
        source_count: sources.len(),
        enabled_source_count,
        ready_source_count,
        requires_configuration,
        can_enable,
        subscriptions: bundle_subscription_response(subscriptions, &source_names),
        sources,
    }
}

fn bundle_subscription_response(
    subscriptions: &[FinanceSubscription],
    source_names: &[&str],
) -> FeedCatalogSubscriptionResponse {
    let source_names = source_names.iter().copied().collect::<HashSet<_>>();
    let user_subscription_ids = subscriptions
        .iter()
        .filter(|subscription| {
            subscription
                .source_names
                .iter()
                .any(|name| source_names.contains(name.as_str()))
        })
        .map(|subscription| subscription.id)
        .collect::<Vec<_>>();

    FeedCatalogSubscriptionResponse {
        user_subscribed: !user_subscription_ids.is_empty(),
        user_subscription_ids,
    }
}

fn resolve_finance_subscription_source_names(
    catalog_source_ids: &[String],
    source_names: &[String],
    match_all_sources: bool,
) -> Result<Vec<String>, ProblemDetails> {
    if match_all_sources && (!catalog_source_ids.is_empty() || !source_names.is_empty()) {
        return Err(ProblemDetails::bad_request(
            "match_all_sources cannot be combined with catalog_source_ids or source_names",
        ));
    }
    if match_all_sources {
        return Ok(Vec::new());
    }

    let mut resolved = Vec::new();
    for catalog_source_id in catalog_source_ids {
        let id = catalog_source_id.trim();
        if id.is_empty() {
            continue;
        }
        resolved.push(find_catalog_source(id)?.feed_name());
    }
    resolved.extend(
        source_names
            .iter()
            .map(|source_name| source_name.trim())
            .filter(|source_name| !source_name.is_empty())
            .map(str::to_owned),
    );
    resolved = dedupe_strings(resolved);
    if resolved.is_empty() {
        return Err(ProblemDetails::bad_request(
            "catalog_source_ids or source_names is required unless match_all_sources is true",
        ));
    }
    Ok(resolved)
}

fn resolve_finance_subscription_event_kinds(
    request: &CreateFinanceSubscriptionRequest,
) -> Result<Vec<FinanceEventKind>, ProblemDetails> {
    let mut event_kinds = request.event_kinds.clone();
    if event_kinds.is_empty() {
        for catalog_source_id in &request.catalog_source_ids {
            let id = catalog_source_id.trim();
            if id.is_empty() {
                continue;
            }
            if let Some(kind) = finance_event_kind_for_feed_type(find_catalog_source(id)?.feed_type)
            {
                event_kinds.push(kind);
            }
        }
    }
    event_kinds = dedupe_finance_event_kinds(event_kinds);
    if event_kinds.is_empty() {
        return Err(ProblemDetails::bad_request(
            "event_kinds is required when it cannot be inferred from catalog_source_ids",
        ));
    }
    Ok(event_kinds)
}

fn finance_event_kind_for_feed_type(feed_type: FeedType) -> Option<FinanceEventKind> {
    match feed_type {
        FeedType::Rss => Some(FinanceEventKind::RssArticle),
        FeedType::MarketCandle => Some(FinanceEventKind::MarketCandleClosed),
        FeedType::Polling | FeedType::Webhook | FeedType::WebSocket => None,
    }
}

fn finance_event_kind_type(kind: FinanceEventKind) -> &'static str {
    match kind {
        FinanceEventKind::RssArticle => "rss_article",
        FinanceEventKind::MarketCandleClosed => "market_candle_closed",
    }
}

fn parse_event_kind_filter(value: Option<&str>) -> Result<Vec<FinanceEventKind>, ProblemDetails> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };

    let mut event_kinds = Vec::new();
    for raw in value.split(',') {
        match raw.trim() {
            "" => {}
            "rss_article" => event_kinds.push(FinanceEventKind::RssArticle),
            "market_candle_closed" => event_kinds.push(FinanceEventKind::MarketCandleClosed),
            invalid => {
                return Err(ProblemDetails::bad_request(format!(
                    "invalid event_kinds value: {invalid}"
                )));
            }
        }
    }
    Ok(dedupe_finance_event_kinds(event_kinds))
}

fn dedupe_finance_event_kinds(values: Vec<FinanceEventKind>) -> Vec<FinanceEventKind> {
    let mut seen = HashSet::new();
    values
        .into_iter()
        .filter(|value| seen.insert(*value))
        .collect()
}

fn dedupe_strings(values: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    values
        .into_iter()
        .filter(|value| seen.insert(value.clone()))
        .collect()
}

fn same_finance_event_kinds(left: &[FinanceEventKind], right: &[FinanceEventKind]) -> bool {
    left.len() == right.len() && left.iter().all(|kind| right.contains(kind))
}

fn same_string_set(left: &[String], right: &[String]) -> bool {
    left.len() == right.len() && left.iter().all(|value| right.contains(value))
}

fn finance_subscription_response(
    subscription: FinanceSubscription,
    feeds: &[DataFeedConfig],
    catalog_by_source_name: &HashMap<String, DefaultFeedSource>,
) -> FinanceSubscriptionResponse {
    let matches_all_sources = subscription.source_names.is_empty();
    let sources = subscription
        .source_names
        .iter()
        .map(|source_name| {
            finance_subscription_source_response(
                source_name,
                feeds.iter().find(|feed| feed.name == *source_name),
                catalog_by_source_name.get(source_name),
            )
        })
        .collect();

    FinanceSubscriptionResponse {
        subscription_id: subscription.id,
        session_key: subscription.session_key.to_string(),
        event_kinds: subscription.event_kinds,
        source_names: subscription.source_names,
        matches_all_sources,
        sources,
        category_tags: subscription.category_tags,
        watch_terms: subscription.watch_terms,
        venues: subscription.venues,
        symbols: subscription.symbols,
        timeframes: subscription.timeframes,
        delivery: subscription.delivery,
        cooldown_secs: subscription.cooldown_secs,
        max_immediate_per_hour: subscription.max_immediate_per_hour,
    }
}

fn finance_subscription_source_response(
    source_name: &str,
    feed: Option<&DataFeedConfig>,
    catalog_source: Option<&DefaultFeedSource>,
) -> FinanceSubscriptionSourceResponse {
    FinanceSubscriptionSourceResponse {
        source_name:       source_name.to_owned(),
        catalog_source_id: catalog_source.map(|source| source.id.clone()),
        catalog_name:      catalog_source.map(|source| source.name.clone()),
        provider:          finance_subscription_source_provider(catalog_source, feed),
        feed_id:           feed.map(|feed| feed.id.clone()),
        feed_type:         feed.map(|feed| feed.feed_type.clone()),
        enabled:           feed.map(|feed| feed.enabled),
        status:            feed.map(|feed| feed.status),
    }
}

fn finance_subscription_source_provider(
    catalog_source: Option<&DefaultFeedSource>,
    feed: Option<&DataFeedConfig>,
) -> Option<String> {
    catalog_source
        .and_then(|source| source.provider.clone())
        .or_else(|| {
            feed.and_then(|feed| {
                feed.transport
                    .get("provider")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
            })
        })
}

fn catalog_by_source_name() -> HashMap<String, DefaultFeedSource> {
    default_finance_feed_sources()
        .into_iter()
        .map(|source| (source.feed_name(), source))
        .collect()
}

fn finance_subscription_not_found(id: Uuid) -> ProblemDetails {
    ProblemDetails::not_found(
        "Finance Subscription Not Found",
        format!("no finance subscription owned by current user with id: {id}"),
    )
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

fn parse_unsubscribe_catalog_feed_request(
    body: &[u8],
) -> Result<UnsubscribeCatalogFeedRequest, ProblemDetails> {
    if body.iter().all(u8::is_ascii_whitespace) {
        return Ok(UnsubscribeCatalogFeedRequest::default());
    }
    serde_json::from_slice(body)
        .map_err(|e| ProblemDetails::bad_request(format!("invalid catalog unsubscribe body: {e}")))
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
    use rara_trading::{
        finance::registry::{FinanceDelivery, FinanceEventKind},
        market_data::{
            InMemoryMarketDataRepository, MarketCandle, MarketDataRepository,
            MarketDataRepositoryRef, Timeframe,
        },
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

    fn test_finance_registry() -> Arc<FinanceSubscriptionRegistry> {
        Arc::new(FinanceSubscriptionRegistry::load(
            std::env::temp_dir().join(format!(
                "rara-test-finance-subscriptions-{}.json",
                Uuid::new_v4()
            )),
        ))
    }

    fn test_market_data_repo() -> Arc<InMemoryMarketDataRepository> {
        Arc::new(InMemoryMarketDataRepository::default())
    }

    fn market_data_candle(
        open_time: &str,
        close_time: &str,
        close: &str,
        provider_sequence: Option<&str>,
    ) -> MarketCandle {
        MarketCandle {
            source_name:       "finance-binance-market-candles".to_owned(),
            venue:             "binance".to_owned(),
            symbol:            "BTCUSDT".to_owned(),
            timeframe:         Timeframe::parse("1m").unwrap(),
            open_time:         open_time.parse().unwrap(),
            close_time:        close_time.parse().unwrap(),
            open:              rust_decimal::Decimal::new(100_000, 2),
            high:              rust_decimal::Decimal::new(101_000, 2),
            low:               rust_decimal::Decimal::new(99_000, 2),
            close:             close.parse().unwrap(),
            volume:            rust_decimal::Decimal::new(42_500, 3),
            ingested_at:       close_time.parse().unwrap(),
            provider_sequence: provider_sequence.map(str::to_owned),
        }
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
            market_data_repo: test_market_data_repo(),
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
            market_data_repo: test_market_data_repo(),
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
            market_data_repo: test_market_data_repo(),
        };
        let auth = auth_state_direct(user);
        data_feed_routes(state).layer(middleware::from_fn_with_state(auth, auth_layer))
    }

    async fn app_with_user_and_market_data_repo(
        user: KernelUser,
        market_data_repo: MarketDataRepositoryRef,
    ) -> Router {
        let pools = build_memory_diesel_pools().await;
        bootstrap_data_feed_schema(&pools).await;
        let svc = DataFeedSvc::new(pools);
        let (event_tx, _event_rx) = tokio::sync::mpsc::channel(16);
        let registry = Arc::new(DataFeedRegistry::new(event_tx));
        let state = DataFeedRouterState {
            svc,
            registry,
            finance_registry: test_finance_registry(),
            market_data_repo,
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
            "CREATE INDEX idx_data_feed_events_source_received_created_id ON \
             data_feed_events(source_name, received_at DESC, created_at DESC, id DESC)",
            "CREATE INDEX idx_data_feed_events_source_type_received_created_id ON \
             data_feed_events(source_name, event_type, received_at DESC, created_at DESC, id DESC)",
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
        assert_eq!(binance["provider"], "binance");
        assert_eq!(binance["venue"], "binance");
        assert_eq!(
            binance["configured_symbols"],
            serde_json::json!(["BTCUSDT", "ETHUSDT"])
        );
        assert_eq!(binance["configured_timeframes"], serde_json::json!(["1m"]));
        assert_eq!(binance["load"]["user_subscription_count"], 0);
        assert_eq!(binance["load"]["subscribed_market_stream_count"], 0);
        assert_eq!(binance["load"]["configured_market_stream_count"], 2);
        assert_eq!(binance["load"]["configured_market_poll_request_count"], 2);
        assert_eq!(
            binance["load"]["configured_market_request_budget_per_second"],
            10.0
        );
        assert_eq!(
            binance["load"]["configured_market_minimum_safe_interval_secs"],
            5
        );
        assert_eq!(
            binance["load"]["configured_market_fanout_safe_to_start"],
            true
        );
        assert!(binance["load"]["configured_market_fanout_diagnostic"].is_null());
        assert!(
            (binance["load"]["configured_market_requests_per_second"]
                .as_f64()
                .unwrap()
                - 2.0 / 60.0)
                .abs()
                < f64::EPSILON
        );

        let longbridge = entries
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["id"] == "longbridge-market-candles")
            .unwrap();
        assert_eq!(longbridge["provider"], "longbridge");
        assert_eq!(longbridge["requires_configuration"], true);
        assert_eq!(longbridge["feed_type"], "market_candle");
        assert!(
            longbridge["setup_hint"]
                .as_str()
                .is_some_and(|hint| hint.contains("Longbridge"))
        );
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
        assert_eq!(fed["load"]["user_subscription_count"], 1);
        assert_eq!(fed["load"]["subscribed_market_stream_count"], 0);
        assert!(fed["load"]["configured_market_stream_count"].is_null());
        assert_eq!(sec["subscriptions"]["user_subscribed"], false);
        assert_eq!(
            sec["subscriptions"]["user_subscription_ids"],
            serde_json::json!([])
        );
    }

    #[tokio::test]
    async fn finance_feed_bundles_list_expands_sources_and_subscription_status() {
        let finance_registry = test_finance_registry();
        let subscription_id = Uuid::new_v4();
        finance_registry
            .upsert(FinanceSubscription {
                id:                     subscription_id,
                owner:                  UserId("admin".to_owned()),
                session_key:            SessionKey::new(),
                event_kinds:            vec![FinanceEventKind::MarketCandleClosed],
                source_names:           vec!["finance-binance-major-crypto-15m".to_owned()],
                category_tags:          Vec::new(),
                watch_terms:            Vec::new(),
                venues:                 vec!["binance".to_owned()],
                symbols:                vec!["BTCUSDT".to_owned(), "ETHUSDT".to_owned()],
                timeframes:             vec!["15m".to_owned()],
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
                    .uri("/api/v1/data-feeds/finance/bundles")
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
        let result: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let bundles = result["bundles"].as_array().unwrap();
        assert_eq!(result["count"], bundles.len());

        let macro_news = bundles
            .iter()
            .find(|bundle| bundle["id"] == "macro-news")
            .unwrap();
        assert_eq!(
            macro_news["catalog_source_ids"],
            serde_json::json!([
                "fed-press-releases",
                "fed-h15-announcements",
                "fed-h10-announcements",
                "sec-press-releases"
            ])
        );
        assert_eq!(macro_news["feed_types"], serde_json::json!(["rss"]));
        assert_eq!(macro_news["source_count"], 4);
        assert_eq!(macro_news["ready_source_count"], 4);
        assert_eq!(macro_news["requires_configuration"], false);
        assert_eq!(macro_news["can_enable"], true);

        let binance = bundles
            .iter()
            .find(|bundle| bundle["id"] == "binance-major-crypto-15m")
            .unwrap();
        assert_eq!(binance["providers"], serde_json::json!(["binance"]));
        assert_eq!(binance["feed_types"], serde_json::json!(["market_candle"]));
        assert_eq!(binance["source_count"], 1);
        assert_eq!(
            binance["sources"][0]["source_name"],
            "finance-binance-major-crypto-15m"
        );
        assert_eq!(binance["subscriptions"]["user_subscribed"], true);
        assert_eq!(
            binance["subscriptions"]["user_subscription_ids"],
            serde_json::json!([subscription_id])
        );
        assert_eq!(binance["sources"][0]["load"]["user_subscription_count"], 1);
        assert_eq!(
            binance["sources"][0]["load"]["subscribed_market_stream_count"],
            2
        );
        assert_eq!(
            binance["sources"][0]["load"]["configured_market_stream_count"],
            5
        );
        assert_eq!(
            binance["sources"][0]["load"]["configured_market_fanout_safe_to_start"],
            true
        );

        let longbridge = bundles
            .iter()
            .find(|bundle| bundle["id"] == "longbridge-equities-daily")
            .unwrap();
        assert_eq!(longbridge["requires_configuration"], true);
        assert_eq!(longbridge["can_enable"], false);

        let yahoo = bundles
            .iter()
            .find(|bundle| bundle["id"] == "yahoo-us-equities-daily")
            .unwrap();
        assert_eq!(yahoo["providers"], serde_json::json!(["yahoo"]));
        assert_eq!(yahoo["requires_configuration"], false);
        assert_eq!(yahoo["can_enable"], true);
        assert_eq!(yahoo["sources"][0]["provider"], "yahoo");
        assert_eq!(
            yahoo["sources"][0]["configured_timeframes"],
            serde_json::json!(["1d"])
        );
        assert_eq!(
            yahoo["sources"][0]["load"]["configured_market_request_budget_per_second"],
            0.2
        );
        assert_eq!(
            yahoo["sources"][0]["load"]["configured_market_fanout_safe_to_start"],
            true
        );
        assert!(
            yahoo["sources"][0]["tags"]
                .as_array()
                .unwrap()
                .iter()
                .any(|tag| tag == "best-effort")
        );

        let fmp = bundles
            .iter()
            .find(|bundle| bundle["id"] == "fmp-us-equities-daily")
            .unwrap();
        assert_eq!(fmp["providers"], serde_json::json!(["fmp"]));
        assert_eq!(fmp["requires_configuration"], true);
        assert_eq!(fmp["can_enable"], false);
        assert_eq!(fmp["sources"][0]["provider"], "fmp");
        assert_eq!(
            fmp["sources"][0]["configured_timeframes"],
            serde_json::json!(["1d"])
        );
    }

    #[test]
    fn finance_catalog_reports_unsafe_persisted_market_candle_fanout() {
        let now = Timestamp::now();
        let symbols = (0..200)
            .map(|index| format!("ASSET{index}USDT"))
            .collect::<Vec<_>>();
        let feed = DataFeedConfig::builder()
            .id("wide-binance".to_owned())
            .name("finance-binance-market-candles".to_owned())
            .feed_type(FeedType::MarketCandle)
            .tags(vec!["finance".to_owned(), "market-data".to_owned()])
            .transport(serde_json::json!({
                "provider": "binance",
                "base_url": "https://api.binance.com",
                "interval_secs": 5,
                "headers": {},
                "venue": "binance",
                "symbols": symbols,
                "timeframes": ["1m"],
                "max_candles_per_poll": 1000
            }))
            .enabled(false)
            .status(FeedStatus::Idle)
            .created_at(now)
            .updated_at(now)
            .build();

        let entries = catalog_response(&[feed], &[]);
        let binance = entries
            .iter()
            .find(|entry| entry.id == "binance-market-candles")
            .unwrap();

        assert_eq!(binance.load.configured_market_stream_count, Some(200));
        assert_eq!(binance.load.configured_market_poll_request_count, Some(200));
        assert_eq!(
            binance.load.configured_market_minimum_safe_interval_secs,
            Some(20)
        );
        assert_eq!(
            binance.load.configured_market_fanout_safe_to_start,
            Some(false)
        );
        assert!(
            binance
                .load
                .configured_market_fanout_diagnostic
                .as_deref()
                .is_some_and(|diagnostic| {
                    diagnostic.contains("fans out to 200 requests per poll")
                        && diagnostic.contains("at least 20")
                })
        );
    }

    #[tokio::test]
    async fn finance_catalog_unsubscribe_removes_only_current_user_source_subscriptions() {
        let finance_registry = test_finance_registry();
        let alice_fed = Uuid::new_v4();
        let alice_sec = Uuid::new_v4();
        let admin_fed = Uuid::new_v4();

        for (id, owner, source_name) in [
            (
                alice_fed,
                UserId("alice".to_owned()),
                "finance-fed-press-releases",
            ),
            (
                alice_sec,
                UserId("alice".to_owned()),
                "finance-sec-press-releases",
            ),
            (
                admin_fed,
                UserId("admin".to_owned()),
                "finance-fed-press-releases",
            ),
        ] {
            finance_registry
                .upsert(FinanceSubscription {
                    id,
                    owner,
                    session_key: SessionKey::new(),
                    event_kinds: vec![FinanceEventKind::RssArticle],
                    source_names: vec![source_name.to_owned()],
                    category_tags: Vec::new(),
                    watch_terms: Vec::new(),
                    venues: Vec::new(),
                    symbols: Vec::new(),
                    timeframes: Vec::new(),
                    delivery: FinanceDelivery::Silent,
                    cooldown_secs: 900,
                    max_immediate_per_hour: 6,
                })
                .await
                .unwrap();
        }

        let app =
            app_with_user_and_finance_registry(user_of(Role::User), finance_registry.clone()).await;
        let res = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/data-feeds/catalog/fed-press-releases/unsubscribe")
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
        let result: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(result["source_name"], "finance-fed-press-releases");
        assert_eq!(
            result["removed_subscription_ids"],
            serde_json::json!([alice_fed])
        );
        assert_eq!(result["removed_count"], 1);
        assert_eq!(result["remaining_subscription_ids"], serde_json::json!([]));

        let alice_subscriptions = finance_registry
            .list_for_owner(&UserId("alice".to_owned()))
            .await;
        assert_eq!(alice_subscriptions.len(), 1);
        assert_eq!(alice_subscriptions[0].id, alice_sec);

        let admin_subscriptions = finance_registry
            .list_for_owner(&UserId("admin".to_owned()))
            .await;
        assert_eq!(admin_subscriptions.len(), 1);
        assert_eq!(admin_subscriptions[0].id, admin_fed);
    }

    #[tokio::test]
    async fn finance_subscriptions_list_returns_current_user_read_model() {
        let finance_registry = test_finance_registry();
        let subscription_id = Uuid::new_v4();
        finance_registry
            .upsert(FinanceSubscription {
                id:                     subscription_id,
                owner:                  UserId("alice".to_owned()),
                session_key:            SessionKey::new(),
                event_kinds:            vec![FinanceEventKind::MarketCandleClosed],
                source_names:           vec!["finance-binance-market-candles".to_owned()],
                category_tags:          Vec::new(),
                watch_terms:            Vec::new(),
                venues:                 vec!["binance".to_owned()],
                symbols:                vec!["BTCUSDT".to_owned()],
                timeframes:             vec!["1m".to_owned()],
                delivery:               FinanceDelivery::Silent,
                cooldown_secs:          900,
                max_immediate_per_hour: 6,
            })
            .await
            .unwrap();
        finance_registry
            .upsert(FinanceSubscription {
                id:                     Uuid::new_v4(),
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

        let app =
            app_with_user_and_finance_registry(user_of(Role::User), finance_registry.clone()).await;
        let res = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/data-feeds/finance/subscriptions")
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
        let result: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(result["count"], 1);
        let subscription = &result["subscriptions"][0];
        assert_eq!(subscription["subscription_id"], subscription_id.to_string());
        assert_eq!(
            subscription["event_kinds"],
            serde_json::json!(["market_candle_closed"])
        );
        assert_eq!(
            subscription["source_names"],
            serde_json::json!(["finance-binance-market-candles"])
        );
        assert_eq!(
            subscription["sources"][0]["catalog_source_id"],
            "binance-market-candles"
        );
        assert_eq!(subscription["sources"][0]["provider"], "binance");
        assert_eq!(subscription["venues"], serde_json::json!(["binance"]));
        assert_eq!(subscription["symbols"], serde_json::json!(["BTCUSDT"]));
        assert_eq!(subscription["timeframes"], serde_json::json!(["1m"]));
    }

    #[tokio::test]
    async fn finance_subscriptions_create_infers_catalog_event_kind_and_updates_existing() {
        let finance_registry = test_finance_registry();
        let app =
            app_with_user_and_finance_registry(user_of(Role::User), finance_registry.clone()).await;
        let session_key = SessionKey::new();
        let body = serde_json::json!({
            "session_key": session_key.to_string(),
            "catalog_source_ids": ["binance-market-candles"],
            "venues": ["binance"],
            "symbols": ["BTCUSDT"],
            "timeframes": ["1m"],
            "delivery": "immediate"
        });

        let create_res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/data-feeds/finance/subscriptions")
                    .header("Authorization", "Bearer s3cret")
                    .header("Content-Type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(create_res.status(), StatusCode::CREATED);
        let create_body = axum::body::to_bytes(create_res.into_body(), usize::MAX)
            .await
            .unwrap();
        let created: serde_json::Value = serde_json::from_slice(&create_body).unwrap();
        assert_eq!(created["created"], true);
        let subscription_id = created["subscription"]["subscription_id"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_eq!(
            created["subscription"]["event_kinds"],
            serde_json::json!(["market_candle_closed"])
        );
        assert_eq!(
            created["subscription"]["source_names"],
            serde_json::json!(["finance-binance-market-candles"])
        );
        assert_eq!(
            created["subscription"]["sources"][0]["catalog_source_id"],
            "binance-market-candles"
        );
        assert_eq!(created["subscription"]["sources"][0]["provider"], "binance");
        assert_eq!(created["subscription"]["delivery"], "immediate");

        let update_body = serde_json::json!({
            "session_key": session_key.to_string(),
            "catalog_source_ids": ["binance-market-candles"],
            "venues": ["binance"],
            "symbols": ["BTCUSDT"],
            "timeframes": ["1m"],
            "delivery": "silent"
        });
        let update_res = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/data-feeds/finance/subscriptions")
                    .header("Authorization", "Bearer s3cret")
                    .header("Content-Type", "application/json")
                    .body(Body::from(update_body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(update_res.status(), StatusCode::OK);
        let update_body = axum::body::to_bytes(update_res.into_body(), usize::MAX)
            .await
            .unwrap();
        let updated: serde_json::Value = serde_json::from_slice(&update_body).unwrap();
        assert_eq!(updated["created"], false);
        assert_eq!(
            updated["subscription"]["subscription_id"],
            serde_json::json!(subscription_id)
        );
        assert_eq!(updated["subscription"]["delivery"], "silent");

        let subscriptions = finance_registry
            .list_for_owner(&UserId("alice".to_owned()))
            .await;
        assert_eq!(subscriptions.len(), 1);
        assert_eq!(subscriptions[0].session_key, session_key);
    }

    #[tokio::test]
    async fn finance_subscriptions_create_requires_source_scope() {
        let app = app_with_user(user_of(Role::User)).await;
        let body = serde_json::json!({
            "session_key": SessionKey::new().to_string(),
            "event_kinds": ["rss_article"]
        });

        let res = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/data-feeds/finance/subscriptions")
                    .header("Authorization", "Bearer s3cret")
                    .header("Content-Type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn finance_subscriptions_delete_is_scoped_to_current_user() {
        let finance_registry = test_finance_registry();
        let alice_id = Uuid::new_v4();
        let admin_id = Uuid::new_v4();

        for (id, owner) in [
            (alice_id, UserId("alice".to_owned())),
            (admin_id, UserId("admin".to_owned())),
        ] {
            finance_registry
                .upsert(FinanceSubscription {
                    id,
                    owner,
                    session_key: SessionKey::new(),
                    event_kinds: vec![FinanceEventKind::RssArticle],
                    source_names: vec!["finance-fed-press-releases".to_owned()],
                    category_tags: Vec::new(),
                    watch_terms: Vec::new(),
                    venues: Vec::new(),
                    symbols: Vec::new(),
                    timeframes: Vec::new(),
                    delivery: FinanceDelivery::Silent,
                    cooldown_secs: 900,
                    max_immediate_per_hour: 6,
                })
                .await
                .unwrap();
        }

        let app =
            app_with_user_and_finance_registry(user_of(Role::User), finance_registry.clone()).await;
        let forbidden_res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!(
                        "/api/v1/data-feeds/finance/subscriptions/{admin_id}"
                    ))
                    .header("Authorization", "Bearer s3cret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(forbidden_res.status(), StatusCode::NOT_FOUND);

        let delete_res = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!(
                        "/api/v1/data-feeds/finance/subscriptions/{alice_id}"
                    ))
                    .header("Authorization", "Bearer s3cret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(delete_res.status(), StatusCode::OK);

        assert!(
            finance_registry
                .list_for_owner(&UserId("alice".to_owned()))
                .await
                .is_empty()
        );
        assert_eq!(
            finance_registry
                .list_for_owner(&UserId("admin".to_owned()))
                .await
                .len(),
            1
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
        assert_eq!(summary["last_event_type"], "poll_response");
        assert_eq!(summary["last_event_at"], event_at.to_string());
        let lag_seconds = summary["lag_seconds"].as_i64().unwrap();
        assert!((58..=62).contains(&lag_seconds), "lag={lag_seconds}");
    }

    #[tokio::test]
    async fn feed_events_endpoint_filters_by_event_kind() {
        let (app, pools) = app_with_user_and_pools(user_of(Role::Admin)).await;
        let create_body = serde_json::json!({
            "name": "mixed-finance-feed",
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

        let mut conn = pools.writer.get().await.expect("pool conn");
        for (event_id, event_type, title, received_at) in [
            (
                "mixed-finance-feed:rss",
                "rss_article",
                "Macro update",
                "2026-07-12T08:00:00Z",
            ),
            (
                "mixed-finance-feed:candle",
                "market_candle_closed",
                "BTCUSDT candle",
                "2026-07-12T08:01:00Z",
            ),
        ] {
            diesel::sql_query(
                "INSERT INTO data_feed_events (id, source_name, event_type, tags, payload, \
                 received_at) VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind::<diesel::sql_types::Text, _>(
                rara_kernel::data_feed::FeedEventId::deterministic(event_id).to_string(),
            )
            .bind::<diesel::sql_types::Text, _>("mixed-finance-feed")
            .bind::<diesel::sql_types::Text, _>(event_type)
            .bind::<diesel::sql_types::Text, _>("[\"finance\"]")
            .bind::<diesel::sql_types::Text, _>(serde_json::json!({ "title": title }).to_string())
            .bind::<diesel::sql_types::Text, _>(received_at)
            .execute(&mut *conn)
            .await
            .expect("insert event");
        }
        drop(conn);

        let res = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!(
                        "/api/v1/data-feeds/{}/events?event_kinds=market_candle_closed",
                        feed.id
                    ))
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
        let events: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(events["total"], 1);
        assert_eq!(events["has_more"], false);
        let returned = events["events"].as_array().unwrap();
        assert_eq!(returned.len(), 1);
        assert_eq!(returned[0]["event_type"], "market_candle_closed");
        assert_eq!(returned[0]["payload"]["title"], "BTCUSDT candle");
    }

    #[tokio::test]
    async fn feed_events_endpoint_rejects_unknown_event_kind() {
        let app = app_with_user(user_of(Role::Admin)).await;
        let create_body = serde_json::json!({
            "name": "invalid-kind-feed",
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

        let res = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!(
                        "/api/v1/data-feeds/{}/events?event_kinds=unknown_kind",
                        feed.id
                    ))
                    .header("Authorization", "Bearer s3cret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
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
    async fn market_data_candle_streams_endpoint_lists_stored_streams() {
        let market_data_repo = test_market_data_repo();
        market_data_repo
            .upsert_closed_candle(MarketCandle {
                source_name:       "finance-binance-market-candles".to_owned(),
                venue:             "binance".to_owned(),
                symbol:            "BTCUSDT".to_owned(),
                timeframe:         Timeframe::parse("1m").unwrap(),
                open_time:         "2026-07-10T08:00:00Z".parse().unwrap(),
                close_time:        "2026-07-10T08:01:00Z".parse().unwrap(),
                open:              rust_decimal::Decimal::new(100_000, 2),
                high:              rust_decimal::Decimal::new(101_000, 2),
                low:               rust_decimal::Decimal::new(99_000, 2),
                close:             rust_decimal::Decimal::new(100_500, 2),
                volume:            rust_decimal::Decimal::new(42, 0),
                ingested_at:       "2026-07-10T08:01:03Z".parse().unwrap(),
                provider_sequence: Some("seq-1".to_owned()),
            })
            .await
            .unwrap();

        let app = app_with_user_and_market_data_repo(user_of(Role::Admin), market_data_repo).await;
        let res = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(
                        "/api/v1/data-feeds/market-data/candle-streams?venue=BINANCE&\
                         symbol=btcusdt&timeframe=1M",
                    )
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
        let result: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(result["count"], 1);
        assert_eq!(result["query_limit"], 500);
        assert_eq!(result["has_more"], false);
        let stream = &result["streams"][0];
        assert_eq!(stream["source_name"], "finance-binance-market-candles");
        assert_eq!(stream["venue"], "binance");
        assert_eq!(stream["symbol"], "BTCUSDT");
        assert_eq!(stream["timeframe"], "1m");
        assert_eq!(stream["candle_count"], 1);
        assert_eq!(stream["first_open_time"], "2026-07-10T08:00:00Z");
        assert_eq!(stream["latest_open_time"], "2026-07-10T08:00:00Z");
        assert_eq!(stream["latest_close_time"], "2026-07-10T08:01:00Z");
        assert_eq!(stream["latest_ingested_at"], "2026-07-10T08:01:03Z");
    }

    #[tokio::test]
    async fn market_data_candle_streams_endpoint_reports_has_more() {
        let market_data_repo = test_market_data_repo();
        market_data_repo
            .upsert_closed_candle(market_data_candle(
                "2026-07-10T08:00:00Z",
                "2026-07-10T08:01:00Z",
                "1005.00",
                Some("seq-1"),
            ))
            .await
            .unwrap();
        market_data_repo
            .upsert_closed_candle(MarketCandle {
                symbol: "ETHUSDT".to_owned(),
                open_time: "2026-07-10T08:01:00Z".parse().unwrap(),
                close_time: "2026-07-10T08:02:00Z".parse().unwrap(),
                close: rust_decimal::Decimal::new(320_000, 2),
                ..market_data_candle(
                    "2026-07-10T08:01:00Z",
                    "2026-07-10T08:02:00Z",
                    "3200.00",
                    Some("seq-2"),
                )
            })
            .await
            .unwrap();

        let app = app_with_user_and_market_data_repo(user_of(Role::Admin), market_data_repo).await;
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/data-feeds/market-data/candle-streams?limit=1")
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
        let result: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(result["count"], 1);
        assert_eq!(result["query_limit"], 1);
        assert_eq!(result["query_offset"], 0);
        assert_eq!(result["has_more"], true);
        assert_eq!(result["streams"].as_array().unwrap().len(), 1);

        let res = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/data-feeds/market-data/candle-streams?limit=1&offset=1")
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
        let result: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(result["count"], 1);
        assert_eq!(result["query_limit"], 1);
        assert_eq!(result["query_offset"], 1);
        assert_eq!(result["has_more"], false);
        assert_eq!(result["streams"][0]["symbol"], "BTCUSDT");
    }

    #[tokio::test]
    async fn market_data_latest_candle_endpoint_returns_stored_candle() {
        let market_data_repo = test_market_data_repo();
        market_data_repo
            .upsert_closed_candle(market_data_candle(
                "2026-07-10T08:00:00Z",
                "2026-07-10T08:01:00Z",
                "1005.00",
                Some("seq-1"),
            ))
            .await
            .unwrap();
        market_data_repo
            .upsert_closed_candle(market_data_candle(
                "2026-07-10T08:01:00Z",
                "2026-07-10T08:02:00Z",
                "1006.25",
                Some("seq-2"),
            ))
            .await
            .unwrap();

        let app = app_with_user_and_market_data_repo(user_of(Role::Admin), market_data_repo).await;
        let res = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(
                        "/api/v1/data-feeds/market-data/candles/latest?venue=BINANCE&\
                         symbol=btcusdt&timeframe=1M",
                    )
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
        let result: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let candle = &result["candle"];

        assert_eq!(candle["source_name"], "finance-binance-market-candles");
        assert_eq!(candle["venue"], "binance");
        assert_eq!(candle["symbol"], "BTCUSDT");
        assert_eq!(candle["timeframe"], "1m");
        assert_eq!(candle["open_time"], "2026-07-10T08:01:00Z");
        assert_eq!(candle["close_time"], "2026-07-10T08:02:00Z");
        assert_eq!(candle["close"], "1006.25");
        assert_eq!(candle["volume"], "42.500");
        assert_eq!(candle["provider_sequence"], "seq-2");
    }

    #[tokio::test]
    async fn market_data_candles_endpoint_returns_bounded_ordered_range() {
        let market_data_repo = test_market_data_repo();
        for (open_time, close_time, close) in [
            ("2026-07-10T08:00:00Z", "2026-07-10T08:01:00Z", "1005.00"),
            ("2026-07-10T08:01:00Z", "2026-07-10T08:02:00Z", "1006.25"),
            ("2026-07-10T08:02:00Z", "2026-07-10T08:03:00Z", "1007.75"),
        ] {
            market_data_repo
                .upsert_closed_candle(market_data_candle(open_time, close_time, close, None))
                .await
                .unwrap();
        }

        let app = app_with_user_and_market_data_repo(user_of(Role::Admin), market_data_repo).await;
        let res = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(
                        "/api/v1/data-feeds/market-data/candles?venue=binance&symbol=BTCUSDT&\
                         timeframe=1m&start=2026-07-10T08:00:00Z&end=2026-07-10T08:03:00Z&limit=2",
                    )
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
        let result: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(result["count"], 2);
        assert_eq!(result["query_limit"], 2);
        assert_eq!(result["has_more"], true);
        assert_eq!(result["next_start"], "2026-07-10T08:02:00Z");
        assert_eq!(result["candles"][0]["open_time"], "2026-07-10T08:00:00Z");
        assert_eq!(result["candles"][0]["close"], "1005.00");
        assert_eq!(result["candles"][1]["open_time"], "2026-07-10T08:01:00Z");
        assert_eq!(result["candles"][1]["close"], "1006.25");
    }

    #[tokio::test]
    async fn market_data_recent_candles_endpoint_returns_latest_candles() {
        let market_data_repo = test_market_data_repo();
        for (open_time, close_time, close) in [
            ("2026-07-10T08:00:00Z", "2026-07-10T08:01:00Z", "1005.00"),
            ("2026-07-10T08:01:00Z", "2026-07-10T08:02:00Z", "1006.25"),
            ("2026-07-10T08:02:00Z", "2026-07-10T08:03:00Z", "1007.75"),
        ] {
            market_data_repo
                .upsert_closed_candle(market_data_candle(open_time, close_time, close, None))
                .await
                .unwrap();
        }

        let app = app_with_user_and_market_data_repo(user_of(Role::Admin), market_data_repo).await;
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(
                        "/api/v1/data-feeds/market-data/candles/recent?venue=binance&\
                         symbol=BTCUSDT&timeframe=1m&limit=2",
                    )
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
        let result: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(result["count"], 2);
        assert_eq!(result["query_limit"], 2);
        assert_eq!(result["has_more"], true);
        assert_eq!(result["next_end"], "2026-07-10T08:01:00Z");
        assert_eq!(result["candles"][0]["open_time"], "2026-07-10T08:01:00Z");
        assert_eq!(result["candles"][0]["close"], "1006.25");
        assert_eq!(result["candles"][1]["open_time"], "2026-07-10T08:02:00Z");
        assert_eq!(result["candles"][1]["close"], "1007.75");

        let res = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(
                        "/api/v1/data-feeds/market-data/candles/recent?venue=binance&\
                         symbol=BTCUSDT&timeframe=1m&limit=2&end=2026-07-10T08:01:00Z",
                    )
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
        let result: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(result["count"], 1);
        assert_eq!(result["query_limit"], 2);
        assert_eq!(result["has_more"], false);
        assert_eq!(result["next_end"], serde_json::Value::Null);
        assert_eq!(result["candles"][0]["open_time"], "2026-07-10T08:00:00Z");
    }

    #[tokio::test]
    async fn market_data_candles_endpoint_rejects_unprobeable_limit() {
        let app =
            app_with_user_and_market_data_repo(user_of(Role::Admin), test_market_data_repo()).await;
        let res = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(
                        "/api/v1/data-feeds/market-data/candles?venue=binance&symbol=BTCUSDT&\
                         timeframe=1m&start=2026-07-10T08:00:00Z&end=2026-07-10T08:03:00Z&\
                         limit=10000",
                    )
                    .header("Authorization", "Bearer s3cret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let result: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(
            result["detail"]
                .as_str()
                .unwrap_or_default()
                .contains("limit must be <= 9999"),
            "unexpected body: {result}"
        );
    }

    #[tokio::test]
    async fn market_data_candle_gaps_endpoint_reports_missing_open_times() {
        let market_data_repo = test_market_data_repo();
        for (open_time, close_time, close) in [
            ("2026-07-10T08:00:00Z", "2026-07-10T08:01:00Z", "1005.00"),
            ("2026-07-10T08:02:00Z", "2026-07-10T08:03:00Z", "1007.75"),
        ] {
            market_data_repo
                .upsert_closed_candle(market_data_candle(open_time, close_time, close, None))
                .await
                .unwrap();
        }

        let app = app_with_user_and_market_data_repo(user_of(Role::Admin), market_data_repo).await;
        let res = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(
                        "/api/v1/data-feeds/market-data/candles/gaps?venue=binance&symbol=BTCUSDT&\
                         timeframe=1m&start=2026-07-10T08:00:00Z&end=2026-07-10T08:03:00Z",
                    )
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
        let result: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(result["expected_count"], 3);
        assert_eq!(result["missing_count"], 1);
        assert_eq!(result["complete"], false);
        assert_eq!(result["missing_open_times"][0], "2026-07-10T08:01:00Z");
    }

    #[tokio::test]
    async fn market_data_candle_freshness_endpoint_reports_latest_status() {
        let market_data_repo = test_market_data_repo();
        market_data_repo
            .upsert_closed_candle(market_data_candle(
                "2026-07-10T08:01:00Z",
                "2026-07-10T08:02:00Z",
                "1006.25",
                None,
            ))
            .await
            .unwrap();

        let app = app_with_user_and_market_data_repo(user_of(Role::Admin), market_data_repo).await;
        let res = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(
                        "/api/v1/data-feeds/market-data/candles/freshness?venue=BINANCE&\
                         symbol=btcusdt&timeframe=1M&as_of=2026-07-10T08:03:00Z&\
                         stale_after_secs=120",
                    )
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
        let result: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(result["status"], "fresh");
        assert_eq!(result["is_stale"], false);
        assert_eq!(result["lag_secs"], 60);
        assert_eq!(result["stale_after_secs"], 120);
        assert_eq!(result["latest"]["open_time"], "2026-07-10T08:01:00Z");
        assert_eq!(result["latest"]["close"], "1006.25");
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
