// Copyright 2026 Rararulab
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

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use async_trait::async_trait;
use jiff::Timestamp;
use rara_backend_admin::data_feeds::{DataFeedSvc, start_feed_task};
use rara_kernel::{
    data_feed::{
        DataFeedConfig, DataFeedRegistry, FeedEvent, FeedStatus, FeedType, parse_duration_ago,
    },
    identity::UserId,
    tool::{ToolContext, ToolExecute},
};
use rara_tool_macro::ToolDef;
use rara_trading::{
    feed::{
        catalog::{DefaultFeedSource, default_finance_feed_sources},
        market_candle::MarketCandleSource,
    },
    finance::registry::{
        FinanceDelivery, FinanceEventKind, FinanceSubscription, FinanceSubscriptionRegistry,
    },
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const MAX_CATALOG_SOURCE_ID_LEN: usize = 128;
const MAX_FEED_ID_LEN: usize = 128;
const MAX_SOURCE_NAME_LEN: usize = 128;
const MAX_NEWS_SOURCE_REFS: usize = 32;
const MAX_EVENT_SOURCE_REFS: usize = 32;
const MAX_UNSUBSCRIBE_SOURCE_REFS: usize = 64;
const MAX_NEWS_CATEGORY_TAGS: usize = 64;
const MAX_NEWS_WATCH_TERMS: usize = 64;
const MAX_NEWS_SELECTOR_LEN: usize = 128;
const MAX_SYMBOLS: usize = 500;
const MAX_TIMEFRAMES: usize = 32;
const MAX_INSTRUMENT_SELECTOR_LEN: usize = 64;
const DEFAULT_COOLDOWN_SECS: u64 = 900;
const DEFAULT_MAX_IMMEDIATE_PER_HOUR: u16 = 6;
const DEFAULT_FEED_EVENT_LIMIT: i64 = 20;
const MAX_FEED_EVENT_LIMIT: i64 = 200;
const DEFAULT_MARKET_CANDLE_CATALOG_SOURCE_ID: &str = "binance-market-candles";

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct FinanceListFeedSourcesParams {}

#[derive(Debug, Clone, Serialize)]
pub(super) struct FinanceFeedSourceEntry {
    pub id:                     String,
    pub name:                   String,
    pub description:            String,
    pub feed_type:              String,
    pub subscribe_tool:         Option<String>,
    pub subscription_hint:      Option<FinanceFeedSourceSubscriptionHint>,
    pub provider:               Option<String>,
    pub tags:                   Vec<String>,
    pub source_name:            String,
    pub requires_configuration: bool,
    pub can_enable:             bool,
    pub setup_hint:             Option<String>,
    pub runtime:                FinanceFeedSourceRuntime,
    pub subscriptions:          FinanceFeedSourceSubscriptions,
    pub venue:                  Option<String>,
    pub configured_symbols:     Vec<String>,
    pub configured_timeframes:  Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct FinanceFeedSourceSubscriptionHint {
    pub tool:            String,
    pub default_params:  serde_json::Value,
    pub required_params: Vec<String>,
    pub optional_params: Vec<String>,
    pub diagnostic_tool: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct FinanceFeedSourceRuntime {
    pub persisted:       bool,
    pub feed_id:         Option<String>,
    pub enabled:         bool,
    pub running:         bool,
    pub status:          Option<String>,
    pub last_error:      Option<String>,
    pub event_count:     i64,
    pub last_event_type: Option<String>,
    pub last_event_at:   Option<String>,
    pub lag_seconds:     Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct FinanceFeedSourceSubscriptions {
    pub user_subscribed:          bool,
    pub session_subscribed:       bool,
    pub user_subscription_ids:    Vec<Uuid>,
    pub session_subscription_ids: Vec<Uuid>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct FinanceListFeedSourcesResult {
    pub sources: Vec<FinanceFeedSourceEntry>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub(super) struct FinanceListFeedEventsParams {
    /// Built-in source ids from finance_list_feed_sources.
    #[serde(default)]
    pub catalog_source_ids: Vec<String>,
    /// Concrete finance source names for custom feeds.
    #[serde(default)]
    pub source_names:       Vec<String>,
    /// Existing persisted finance DataFeedConfig ids.
    #[serde(default)]
    pub feed_ids:           Vec<String>,
    /// Optional event-kind filter, e.g. rss_article or market_candle_closed.
    #[serde(default)]
    pub event_kinds:        Vec<FinanceEventKind>,
    /// Duration string such as "1h", "24h", or "7d".
    #[serde(default)]
    pub since:              Option<String>,
    /// Maximum events per source. Defaults to 20, max 200.
    #[serde(default)]
    pub limit:              Option<i64>,
    /// Offset per source for pagination. Defaults to 0.
    #[serde(default)]
    pub offset:             Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct FinanceListFeedEventsResult {
    pub sources: Vec<FinanceFeedEventPage>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct FinanceFeedEventPage {
    pub source_name:       String,
    pub catalog_source_id: Option<String>,
    pub feed_id:           Option<String>,
    pub events:            Vec<FeedEvent>,
    pub total:             i64,
    pub has_more:          bool,
    pub query_limit:       i64,
    pub query_offset:      i64,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct FinanceListSubscriptionsParams {
    /// Only return subscriptions bound to the current conversation/session.
    #[serde(default)]
    pub current_session_only: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct FinanceListSubscriptionsResult {
    pub subscriptions: Vec<FinanceSubscriptionEntry>,
    pub count:         usize,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub(super) struct FinanceUnsubscribeParams {
    /// Backward-compatible single subscription id. Prefer using
    /// subscription_ids for multiple removals.
    #[serde(default)]
    pub subscription_id:      Option<Uuid>,
    /// Finance subscription ids from finance_list_subscriptions.
    #[serde(default)]
    pub subscription_ids:     Vec<Uuid>,
    /// Built-in source ids from finance_list_feed_sources, for example
    /// fed-press-releases or binance-market-candles.
    #[serde(default)]
    pub catalog_source_ids:   Vec<String>,
    /// Concrete finance source names for custom feeds.
    #[serde(default)]
    pub source_names:         Vec<String>,
    /// Event kinds to match, for example rss_article or market_candle_closed.
    #[serde(default)]
    pub event_kinds:          Vec<FinanceEventKind>,
    /// Instrument symbols to match.
    #[serde(default)]
    pub symbols:              Vec<String>,
    /// Candle intervals to match.
    #[serde(default)]
    pub timeframes:           Vec<String>,
    /// Restrict selector-based removals to the current conversation/session.
    /// Defaults to true unless explicit subscription ids are supplied.
    #[serde(default)]
    pub current_session_only: Option<bool>,
    /// Return matches without removing them.
    #[serde(default)]
    pub dry_run:              Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct FinanceUnsubscribeResult {
    pub dry_run:                  bool,
    pub matched_count:            usize,
    pub removed_count:            usize,
    pub removed_subscription_ids: Vec<Uuid>,
    pub matches:                  Vec<FinanceUnsubscribeMatch>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct FinanceUnsubscribeMatch {
    pub subscription_id: Uuid,
    pub current_session: bool,
    pub session_key:     String,
    pub event_kinds:     Vec<FinanceEventKind>,
    pub source_names:    Vec<String>,
    pub venues:          Vec<String>,
    pub symbols:         Vec<String>,
    pub timeframes:      Vec<String>,
    pub delivery:        FinanceDelivery,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct FinanceSubscriptionEntry {
    pub subscription_id:            Uuid,
    pub current_session:            bool,
    pub session_key:                String,
    pub event_kinds:                Vec<FinanceEventKind>,
    pub diagnostic_tool:            Option<String>,
    pub diagnostic_subscription_id: Option<Uuid>,
    pub unsubscribe_hint:           Option<FinanceSubscriptionUnsubscribeHint>,
    pub events_hint:                Option<FinanceSubscriptionEventsHint>,
    pub source_names:               Vec<String>,
    pub matches_all_sources:        bool,
    pub sources:                    Vec<FinanceSubscriptionSource>,
    pub category_tags:              Vec<String>,
    pub watch_terms:                Vec<String>,
    pub venues:                     Vec<String>,
    pub symbols:                    Vec<String>,
    pub timeframes:                 Vec<String>,
    pub delivery:                   FinanceDelivery,
    pub cooldown_secs:              u64,
    pub max_immediate_per_hour:     u16,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct FinanceSubscriptionUnsubscribeHint {
    pub tool:            String,
    pub default_params:  serde_json::Value,
    pub required_params: Vec<String>,
    pub optional_params: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct FinanceSubscriptionEventsHint {
    pub tool:            String,
    pub default_params:  serde_json::Value,
    pub required_params: Vec<String>,
    pub optional_params: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct FinanceSubscriptionSource {
    pub source_name:       String,
    pub catalog_source_id: Option<String>,
    pub catalog_name:      Option<String>,
    pub provider:          Option<String>,
    pub feed_id:           Option<String>,
    pub feed_type:         Option<String>,
    pub persisted:         bool,
    pub enabled:           Option<bool>,
    pub running:           bool,
    pub status:            Option<String>,
    pub last_error:        Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct FinanceEnableFeedSourceParams {
    /// Built-in source id from `finance_list_feed_sources`.
    pub catalog_source_id: String,
    /// Whether to start the runtime feed task immediately. Defaults to true.
    #[serde(default)]
    pub start_now:         Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct FinanceEnableFeedSourceResult {
    pub catalog_source_id: String,
    pub feed_id:           String,
    pub source_name:       String,
    pub feed_type:         String,
    pub tags:              Vec<String>,
    pub created:           bool,
    pub enabled:           bool,
    pub started:           bool,
    pub running:           bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct FinanceDisableFeedSourceParams {
    /// Built-in source id from `finance_list_feed_sources`.
    #[serde(default)]
    pub catalog_source_id: Option<String>,
    /// Existing persisted finance DataFeedConfig id.
    #[serde(default)]
    pub feed_id:           Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct FinanceDisableFeedSourceResult {
    pub feed_id:           String,
    pub source_name:       String,
    pub catalog_source_id: Option<String>,
    pub feed_type:         String,
    pub changed:           bool,
    pub enabled:           bool,
    pub running:           bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct FinanceRestartFeedSourceParams {
    /// Built-in source id from `finance_list_feed_sources`.
    #[serde(default)]
    pub catalog_source_id: Option<String>,
    /// Existing persisted finance DataFeedConfig id.
    #[serde(default)]
    pub feed_id:           Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct FinanceRestartFeedSourceResult {
    pub feed_id:           String,
    pub source_name:       String,
    pub catalog_source_id: Option<String>,
    pub feed_type:         String,
    pub was_running:       bool,
    pub started:           bool,
    pub running:           bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct FinanceSubscribeNewsParams {
    /// Built-in RSS source ids from `finance_list_feed_sources`, for example
    /// `fed-press-releases` or `sec-press-releases`.
    #[serde(default)]
    pub catalog_source_ids:     Vec<String>,
    /// Existing persisted finance RSS DataFeedConfig ids.
    #[serde(default)]
    pub feed_ids:               Vec<String>,
    /// Optional category tag selectors. Values are normalized by the finance
    /// registry to category tags.
    #[serde(default)]
    pub category_tags:          Vec<String>,
    /// Literal title/summary terms to match.
    #[serde(default)]
    pub watch_terms:            Vec<String>,
    /// Whether to start inactive feed tasks immediately. Defaults to true.
    #[serde(default)]
    pub start_now:              Option<bool>,
    /// Delivery policy for matched article events. Defaults to silent.
    #[serde(default)]
    pub delivery:               Option<FinanceDelivery>,
    #[serde(default)]
    pub cooldown_secs:          Option<u64>,
    #[serde(default)]
    pub max_immediate_per_hour: Option<u16>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct FinanceSubscribeNewsResult {
    pub subscription_id:        Uuid,
    pub subscription_created:   bool,
    pub sources:                Vec<SubscribedNewsSource>,
    pub source_names:           Vec<String>,
    pub category_tags:          Vec<String>,
    pub watch_terms:            Vec<String>,
    pub delivery:               FinanceDelivery,
    pub cooldown_secs:          u64,
    pub max_immediate_per_hour: u16,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct SubscribedNewsSource {
    pub feed_id:           String,
    pub source_name:       String,
    pub catalog_source_id: Option<String>,
    pub feed_change:       FeedChange,
    pub started:           bool,
    pub running:           bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct FinanceSubscribeInstrumentsParams {
    /// Built-in market-candle source id from `finance_list_feed_sources`.
    /// Defaults to binance-market-candles when feed_id is not provided.
    #[serde(default)]
    pub catalog_source_id:      Option<String>,
    /// Existing persisted DataFeedConfig id. Use this for custom market-candle
    /// feeds.
    #[serde(default)]
    pub feed_id:                Option<String>,
    /// Venue selector used for candle event matching, e.g. "binance".
    #[serde(default)]
    pub venue:                  Option<String>,
    /// Instrument symbols to fetch and subscribe to, for example `BTCUSDT`.
    pub symbols:                Vec<String>,
    /// Candle intervals to fetch and subscribe to, for example `1m` or `5m`.
    pub timeframes:             Vec<String>,
    /// Whether to start or restart the runtime feed task immediately. Defaults
    /// to true.
    #[serde(default)]
    pub start_now:              Option<bool>,
    /// Delivery policy for matched closed-candle events. Defaults to silent.
    #[serde(default)]
    pub delivery:               Option<FinanceDelivery>,
    #[serde(default)]
    pub cooldown_secs:          Option<u64>,
    #[serde(default)]
    pub max_immediate_per_hour: Option<u16>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct FinanceSubscribeInstrumentsResult {
    pub subscription_id: Uuid,
    pub subscription_created: bool,
    pub diagnostic_tool: Option<String>,
    pub diagnostic_subscription_id: Option<Uuid>,
    pub feed_id: String,
    pub source_name: String,
    pub catalog_source_id: Option<String>,
    pub venue: String,
    pub symbols: Vec<String>,
    pub timeframes: Vec<String>,
    pub feed_change: FeedChange,
    pub feed_restarted: bool,
    pub running: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum FeedChange {
    Created,
    Updated,
    Unchanged,
}

#[derive(ToolDef)]
#[tool(
    name = "finance_list_feed_sources",
    description = "List built-in finance data feed source catalog entries with current persisted \
                   config and runtime status. Use this before enabling or subscribing to default \
                   feeds such as Fed, SEC, Binance, or Longbridge. This is read-only and never \
                   places trades.",
    tier = "deferred",
    read_only,
    concurrency_safe
)]
pub(super) struct FinanceListFeedSourcesTool {
    svc:              DataFeedSvc,
    registry:         Arc<DataFeedRegistry>,
    finance_registry: Arc<FinanceSubscriptionRegistry>,
}

#[derive(ToolDef)]
#[tool(
    name = "finance_list_feed_events",
    description = "List recent persisted events for finance RSS or market-candle feed sources. \
                   Use this after subscribing or diagnosing a feed when the user asks what recent \
                   finance news or closed-candle notifications were received. Select sources by \
                   catalog_source_ids, source_names, or feed_ids from finance_list_feed_sources. \
                   This is read-only and never places trades.",
    tier = "deferred",
    read_only,
    concurrency_safe
)]
pub(super) struct FinanceListFeedEventsTool {
    svc: DataFeedSvc,
}

#[derive(ToolDef)]
#[tool(
    name = "finance_list_subscriptions",
    description = "List finance information subscriptions owned by the current user with \
                   conversation/session ownership, source catalog ids, persisted feed config, and \
                   runtime status. Use this before finance_unsubscribe when the user asks what \
                   they are watching or wants to cancel a finance subscription. This is read-only \
                   and never places trades.",
    tier = "deferred",
    read_only,
    concurrency_safe
)]
pub(super) struct FinanceListSubscriptionsTool {
    svc:              DataFeedSvc,
    registry:         Arc<DataFeedRegistry>,
    finance_registry: Arc<FinanceSubscriptionRegistry>,
}

#[derive(ToolDef)]
#[tool(
    name = "finance_unsubscribe",
    description = "Remove finance information subscriptions owned by the current user. Accepts \
                   explicit subscription ids from finance_list_subscriptions, or selector filters \
                   such as catalog_source_ids, source_names, event_kinds, symbols, and \
                   timeframes. Selector-based removals default to the current conversation only; \
                   broad wildcard subscriptions are only removed by explicit id. This never \
                   places trades.",
    tier = "deferred"
)]
pub(super) struct FinanceUnsubscribeTool {
    finance_registry: Arc<FinanceSubscriptionRegistry>,
}

#[derive(ToolDef)]
#[tool(
    name = "finance_enable_feed_source",
    description = "Enable one built-in finance feed source from finance_list_feed_sources. This \
                   persists a DataFeedConfig and optionally starts the runtime feed task. It only \
                   works for catalog entries with can_enable=true; provider presets that require \
                   credentials or custom endpoints are rejected. This never places trades.",
    tier = "deferred"
)]
pub(super) struct FinanceEnableFeedSourceTool {
    svc:      DataFeedSvc,
    registry: Arc<DataFeedRegistry>,
}

#[derive(ToolDef)]
#[tool(
    name = "finance_disable_feed_source",
    description = "Disable one persisted finance feed source by catalog_source_id or feed_id. \
                   This stops any running feed task, persists enabled=false and clears runtime \
                   error state. It only operates on finance-scoped RSS or market-candle feeds and \
                   never places trades.",
    tier = "deferred"
)]
pub(super) struct FinanceDisableFeedSourceTool {
    svc:      DataFeedSvc,
    registry: Arc<DataFeedRegistry>,
}

#[derive(ToolDef)]
#[tool(
    name = "finance_restart_feed_source",
    description = "Restart one enabled persisted finance feed source by catalog_source_id or \
                   feed_id. This cancels the current runtime task if present, refreshes the \
                   registry config, and starts a new RSS or market-candle feed task. It never \
                   places trades.",
    tier = "deferred"
)]
pub(super) struct FinanceRestartFeedSourceTool {
    svc:      DataFeedSvc,
    registry: Arc<DataFeedRegistry>,
}

#[derive(ToolDef)]
#[tool(
    name = "finance_subscribe_news",
    description = "Subscribe the current conversation to finance RSS/Atom article updates from \
                   built-in catalog sources or existing finance RSS feeds. This ensures the \
                   selected RSS feeds are enabled, optionally starts them, and creates an \
                   rss_article subscription using ToolContext identity/session. It never accepts \
                   arbitrary URLs and never places trades.",
    tier = "deferred"
)]
pub(super) struct FinanceSubscribeNewsTool {
    data_feed_svc:      DataFeedSvc,
    data_feed_registry: Arc<DataFeedRegistry>,
    finance_registry:   Arc<FinanceSubscriptionRegistry>,
}

#[derive(ToolDef)]
#[tool(
    name = "finance_subscribe_instruments",
    description = "Subscribe the current conversation to closed market-candle updates for \
                   specific instruments. This ensures the selected market-candle feed source is \
                   enabled, persists requested symbols/timeframes into the DataFeedConfig, and \
                   optionally starts or restarts the runtime feed task. Identity and session are \
                   always taken from tool context. This never places trades.",
    tier = "deferred"
)]
pub(super) struct FinanceSubscribeInstrumentsTool {
    data_feed_svc:      DataFeedSvc,
    data_feed_registry: Arc<DataFeedRegistry>,
    finance_registry:   Arc<FinanceSubscriptionRegistry>,
}

impl FinanceListFeedSourcesTool {
    pub(super) fn new(
        svc: DataFeedSvc,
        registry: Arc<DataFeedRegistry>,
        finance_registry: Arc<FinanceSubscriptionRegistry>,
    ) -> Self {
        Self {
            svc,
            registry,
            finance_registry,
        }
    }
}

impl FinanceListFeedEventsTool {
    pub(super) fn new(svc: DataFeedSvc) -> Self { Self { svc } }
}

impl FinanceListSubscriptionsTool {
    pub(super) fn new(
        svc: DataFeedSvc,
        registry: Arc<DataFeedRegistry>,
        finance_registry: Arc<FinanceSubscriptionRegistry>,
    ) -> Self {
        Self {
            svc,
            registry,
            finance_registry,
        }
    }
}

impl FinanceUnsubscribeTool {
    pub(super) fn new(finance_registry: Arc<FinanceSubscriptionRegistry>) -> Self {
        Self { finance_registry }
    }
}

impl FinanceEnableFeedSourceTool {
    pub(super) fn new(svc: DataFeedSvc, registry: Arc<DataFeedRegistry>) -> Self {
        Self { svc, registry }
    }
}

impl FinanceDisableFeedSourceTool {
    pub(super) fn new(svc: DataFeedSvc, registry: Arc<DataFeedRegistry>) -> Self {
        Self { svc, registry }
    }
}

impl FinanceRestartFeedSourceTool {
    pub(super) fn new(svc: DataFeedSvc, registry: Arc<DataFeedRegistry>) -> Self {
        Self { svc, registry }
    }
}

impl FinanceSubscribeNewsTool {
    pub(super) fn new(
        data_feed_svc: DataFeedSvc,
        data_feed_registry: Arc<DataFeedRegistry>,
        finance_registry: Arc<FinanceSubscriptionRegistry>,
    ) -> Self {
        Self {
            data_feed_svc,
            data_feed_registry,
            finance_registry,
        }
    }
}

impl FinanceSubscribeInstrumentsTool {
    pub(super) fn new(
        data_feed_svc: DataFeedSvc,
        data_feed_registry: Arc<DataFeedRegistry>,
        finance_registry: Arc<FinanceSubscriptionRegistry>,
    ) -> Self {
        Self {
            data_feed_svc,
            data_feed_registry,
            finance_registry,
        }
    }
}

#[async_trait]
impl ToolExecute for FinanceListFeedSourcesTool {
    type Output = FinanceListFeedSourcesResult;
    type Params = FinanceListFeedSourcesParams;

    async fn run(
        &self,
        _params: FinanceListFeedSourcesParams,
        context: &ToolContext,
    ) -> anyhow::Result<FinanceListFeedSourcesResult> {
        let feeds = self.svc.list_feeds().await?;
        let summaries = self
            .svc
            .event_summaries()
            .await?
            .into_iter()
            .map(|summary| (summary.source_name.clone(), summary))
            .collect::<HashMap<_, _>>();
        let owner = UserId(context.user_id.clone());
        let subscriptions = self.finance_registry.list_for_owner(&owner).await;
        let now = Timestamp::now();

        Ok(FinanceListFeedSourcesResult {
            sources: default_finance_feed_sources()
                .into_iter()
                .map(|source| {
                    let source_name = source.feed_name();
                    let persisted = feeds.iter().find(|feed| feed.name == source_name);
                    let summary = summaries.get(&source_name);
                    let last_event_at = summary.and_then(|summary| summary.last_event_at);
                    let lag_seconds = last_event_at
                        .map(|last_event_at| now.duration_since(last_event_at).as_secs().max(0));
                    feed_source_entry(
                        source,
                        persisted,
                        self.registry.is_running(&source_name),
                        summary.map_or(0, |summary| summary.event_count),
                        summary.and_then(|summary| summary.last_event_type.clone()),
                        last_event_at.map(|timestamp| timestamp.to_string()),
                        lag_seconds,
                        source_subscription_summary(
                            &subscriptions,
                            context.session_key,
                            &source_name,
                        ),
                    )
                })
                .collect(),
        })
    }
}

#[async_trait]
impl ToolExecute for FinanceListFeedEventsTool {
    type Output = FinanceListFeedEventsResult;
    type Params = FinanceListFeedEventsParams;

    async fn run(
        &self,
        params: FinanceListFeedEventsParams,
        _context: &ToolContext,
    ) -> anyhow::Result<FinanceListFeedEventsResult> {
        let source_refs = resolve_feed_event_sources(
            &self.svc,
            params.catalog_source_ids,
            params.source_names,
            params.feed_ids,
        )
        .await?;
        let since = params
            .since
            .as_deref()
            .map(parse_duration_ago)
            .transpose()
            .map_err(|err| anyhow::anyhow!("invalid since duration: {err}"))?;
        let limit = params
            .limit
            .unwrap_or(DEFAULT_FEED_EVENT_LIMIT)
            .clamp(1, MAX_FEED_EVENT_LIMIT);
        let offset = params.offset.unwrap_or(0).max(0);
        let event_types = dedupe_event_kinds(params.event_kinds)
            .into_iter()
            .map(finance_event_kind_type)
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();

        let mut sources = Vec::with_capacity(source_refs.len());
        for source_ref in source_refs {
            let page = self
                .svc
                .query_events(&source_ref.source_name, since, &event_types, limit, offset)
                .await?;
            sources.push(FinanceFeedEventPage {
                source_name:       source_ref.source_name,
                catalog_source_id: source_ref.catalog_source_id,
                feed_id:           source_ref.feed_id,
                events:            page.events,
                total:             page.total,
                has_more:          page.has_more,
                query_limit:       limit,
                query_offset:      offset,
            });
        }

        Ok(FinanceListFeedEventsResult { sources })
    }
}

#[async_trait]
impl ToolExecute for FinanceListSubscriptionsTool {
    type Output = FinanceListSubscriptionsResult;
    type Params = FinanceListSubscriptionsParams;

    async fn run(
        &self,
        params: FinanceListSubscriptionsParams,
        context: &ToolContext,
    ) -> anyhow::Result<FinanceListSubscriptionsResult> {
        let current_session_only = params.current_session_only.unwrap_or(false);
        let owner = UserId(context.user_id.clone());
        let feeds = self.svc.list_feeds().await?;
        let feeds_by_name = feeds
            .iter()
            .map(|feed| (feed.name.clone(), feed))
            .collect::<HashMap<_, _>>();
        let catalog_by_source_name = catalog_by_source_name();

        let subscriptions = self
            .finance_registry
            .list_for_owner(&owner)
            .await
            .into_iter()
            .filter(|subscription| {
                !current_session_only || subscription.session_key == context.session_key
            })
            .map(|subscription| {
                subscription_entry(
                    subscription,
                    context.session_key,
                    &feeds_by_name,
                    &catalog_by_source_name,
                    &self.registry,
                )
            })
            .collect::<Vec<_>>();

        Ok(FinanceListSubscriptionsResult {
            count: subscriptions.len(),
            subscriptions,
        })
    }
}

#[async_trait]
impl ToolExecute for FinanceUnsubscribeTool {
    type Output = FinanceUnsubscribeResult;
    type Params = FinanceUnsubscribeParams;

    async fn run(
        &self,
        params: FinanceUnsubscribeParams,
        context: &ToolContext,
    ) -> anyhow::Result<FinanceUnsubscribeResult> {
        let selector = normalize_unsubscribe_selector(params)?;
        let owner = UserId(context.user_id.clone());
        let dry_run = selector.dry_run;

        let matches = self
            .finance_registry
            .list_for_owner(&owner)
            .await
            .into_iter()
            .filter(|subscription| unsubscribe_selector_matches(subscription, &selector, context))
            .map(|subscription| unsubscribe_match(subscription, context.session_key))
            .collect::<Vec<_>>();

        let mut removed_subscription_ids = Vec::new();
        if !dry_run {
            for entry in &matches {
                if self
                    .finance_registry
                    .remove(&owner, entry.subscription_id)
                    .await?
                {
                    removed_subscription_ids.push(entry.subscription_id);
                }
            }
        }

        Ok(FinanceUnsubscribeResult {
            dry_run,
            matched_count: matches.len(),
            removed_count: removed_subscription_ids.len(),
            removed_subscription_ids,
            matches,
        })
    }
}

#[async_trait]
impl ToolExecute for FinanceEnableFeedSourceTool {
    type Output = FinanceEnableFeedSourceResult;
    type Params = FinanceEnableFeedSourceParams;

    async fn run(
        &self,
        params: FinanceEnableFeedSourceParams,
        _context: &ToolContext,
    ) -> anyhow::Result<FinanceEnableFeedSourceResult> {
        let catalog_source_id = normalize_catalog_source_id(params.catalog_source_id)?;
        let start_now = params.start_now.unwrap_or(true);
        let source = find_catalog_source(&catalog_source_id)?;
        anyhow::ensure!(
            source.can_enable(),
            "finance feed source {catalog_source_id} requires configuration: {}",
            source
                .setup_hint
                .as_deref()
                .unwrap_or("configure this provider before enabling it")
        );

        let source_name = source.feed_name();
        let existing = self
            .svc
            .list_feeds()
            .await?
            .into_iter()
            .find(|feed| feed.name == source_name);
        let already_running = self.registry.is_running(&source_name);
        let already_enabled = existing.as_ref().is_some_and(|feed| feed.enabled);

        let (config, created) = match existing {
            Some(feed) if already_enabled && already_running => (feed, false),
            Some(feed) => {
                let config = config_from_source(&source, Some(feed))?;
                anyhow::ensure!(
                    self.svc.update_feed(&config).await?,
                    "failed to update existing finance feed source: {source_name}"
                );
                replace_registry_config(&self.registry, config.clone())?;
                (config, false)
            }
            None => {
                let config = config_from_source(&source, None)?;
                self.svc.create_feed(&config).await?;
                self.registry.register(config.clone())?;
                (config, true)
            }
        };

        let started = if start_now && !already_running {
            start_feed_task(&config, &self.registry);
            true
        } else {
            false
        };

        Ok(FinanceEnableFeedSourceResult {
            catalog_source_id,
            feed_id: config.id,
            source_name: config.name,
            feed_type: config.feed_type.to_string(),
            tags: config.tags,
            created,
            enabled: config.enabled,
            started,
            running: self.registry.is_running(&source_name),
        })
    }
}

#[async_trait]
impl ToolExecute for FinanceDisableFeedSourceTool {
    type Output = FinanceDisableFeedSourceResult;
    type Params = FinanceDisableFeedSourceParams;

    async fn run(
        &self,
        params: FinanceDisableFeedSourceParams,
        _context: &ToolContext,
    ) -> anyhow::Result<FinanceDisableFeedSourceResult> {
        let source_ref = SourceRef::from_params(params.catalog_source_id, params.feed_id)?;
        let (mut config, catalog_source_id) =
            self.resolve_existing_finance_feed(source_ref).await?;
        let was_running = self.registry.is_running(&config.name);
        let changed = config.enabled
            || was_running
            || config.status != FeedStatus::Idle
            || config.last_error.is_some();

        if changed {
            config.enabled = false;
            config.status = FeedStatus::Idle;
            config.last_error = None;
            config.updated_at = Timestamp::now();
            anyhow::ensure!(
                self.svc.update_feed(&config).await?,
                "failed to disable finance feed source: {}",
                config.name
            );
            replace_registry_config(&self.registry, config.clone())?;
        }

        Ok(FinanceDisableFeedSourceResult {
            feed_id: config.id,
            source_name: config.name.clone(),
            catalog_source_id,
            feed_type: config.feed_type.to_string(),
            changed,
            enabled: config.enabled,
            running: self.registry.is_running(&config.name),
        })
    }
}

impl FinanceDisableFeedSourceTool {
    async fn resolve_existing_finance_feed(
        &self,
        source_ref: SourceRef,
    ) -> anyhow::Result<(DataFeedConfig, Option<String>)> {
        resolve_existing_finance_feed(&self.svc, source_ref).await
    }
}

#[async_trait]
impl ToolExecute for FinanceRestartFeedSourceTool {
    type Output = FinanceRestartFeedSourceResult;
    type Params = FinanceRestartFeedSourceParams;

    async fn run(
        &self,
        params: FinanceRestartFeedSourceParams,
        _context: &ToolContext,
    ) -> anyhow::Result<FinanceRestartFeedSourceResult> {
        let source_ref = SourceRef::from_params(params.catalog_source_id, params.feed_id)?;
        let (mut config, catalog_source_id) =
            self.resolve_existing_finance_feed(source_ref).await?;
        anyhow::ensure!(
            config.enabled,
            "finance feed source {} is disabled; enable it before restarting",
            config.name
        );
        let was_running = self.registry.is_running(&config.name);

        normalize_finance_feed_config(&mut config)?;
        config.status = FeedStatus::Idle;
        config.last_error = None;
        config.updated_at = Timestamp::now();
        anyhow::ensure!(
            self.svc.update_feed(&config).await?,
            "failed to refresh finance feed source before restart: {}",
            config.name
        );
        replace_registry_config(&self.registry, config.clone())?;
        start_feed_task(&config, &self.registry);
        let running = self.registry.is_running(&config.name);

        Ok(FinanceRestartFeedSourceResult {
            feed_id: config.id,
            source_name: config.name,
            catalog_source_id,
            feed_type: config.feed_type.to_string(),
            was_running,
            started: running,
            running,
        })
    }
}

impl FinanceRestartFeedSourceTool {
    async fn resolve_existing_finance_feed(
        &self,
        source_ref: SourceRef,
    ) -> anyhow::Result<(DataFeedConfig, Option<String>)> {
        resolve_existing_finance_feed(&self.svc, source_ref).await
    }
}

#[async_trait]
impl ToolExecute for FinanceSubscribeNewsTool {
    type Output = FinanceSubscribeNewsResult;
    type Params = FinanceSubscribeNewsParams;

    async fn run(
        &self,
        params: FinanceSubscribeNewsParams,
        context: &ToolContext,
    ) -> anyhow::Result<FinanceSubscribeNewsResult> {
        let start_now = params.start_now.unwrap_or(true);
        let catalog_source_ids = normalize_string_refs(
            "catalog_source_ids",
            params.catalog_source_ids,
            MAX_NEWS_SOURCE_REFS,
            MAX_CATALOG_SOURCE_ID_LEN,
        )?;
        let feed_ids = normalize_string_refs(
            "feed_ids",
            params.feed_ids,
            MAX_NEWS_SOURCE_REFS,
            MAX_FEED_ID_LEN,
        )?;
        anyhow::ensure!(
            !catalog_source_ids.is_empty() || !feed_ids.is_empty(),
            "catalog_source_ids or feed_ids is required"
        );
        anyhow::ensure!(
            catalog_source_ids.len() + feed_ids.len() <= MAX_NEWS_SOURCE_REFS,
            "too many news feed sources"
        );

        let category_tags = normalize_news_category_tags(params.category_tags)?;
        let watch_terms = normalize_news_watch_terms(params.watch_terms)?;
        let delivery = params.delivery.unwrap_or(FinanceDelivery::Silent);
        let cooldown_secs = params.cooldown_secs.unwrap_or(DEFAULT_COOLDOWN_SECS);
        anyhow::ensure!(cooldown_secs <= 86_400, "cooldown_secs must be <= 86400");
        let max_immediate_per_hour = params
            .max_immediate_per_hour
            .unwrap_or(DEFAULT_MAX_IMMEDIATE_PER_HOUR);
        anyhow::ensure!(
            max_immediate_per_hour <= 60,
            "max_immediate_per_hour must be <= 60"
        );

        let mut sources = Vec::new();
        let mut source_names = Vec::new();
        for catalog_source_id in catalog_source_ids {
            let source = self
                .ensure_catalog_rss_source(catalog_source_id, start_now)
                .await?;
            if !source_names.contains(&source.source_name) {
                source_names.push(source.source_name.clone());
                sources.push(source);
            }
        }
        for feed_id in feed_ids {
            let source = self.ensure_existing_rss_source(feed_id, start_now).await?;
            if !source_names.contains(&source.source_name) {
                source_names.push(source.source_name.clone());
                sources.push(source);
            }
        }

        let (subscription_id, subscription_created) = self
            .upsert_news_subscription(
                context,
                &source_names,
                &category_tags,
                &watch_terms,
                delivery,
                cooldown_secs,
                max_immediate_per_hour,
            )
            .await?;
        let persisted = self
            .finance_registry
            .list_for_owner(&UserId(context.user_id.clone()))
            .await
            .into_iter()
            .find(|subscription| subscription.id == subscription_id)
            .ok_or_else(|| anyhow::anyhow!("created news subscription was not persisted"))?;

        Ok(FinanceSubscribeNewsResult {
            subscription_id,
            subscription_created,
            sources,
            source_names: persisted.source_names,
            category_tags: persisted.category_tags,
            watch_terms: persisted.watch_terms,
            delivery: persisted.delivery,
            cooldown_secs: persisted.cooldown_secs,
            max_immediate_per_hour: persisted.max_immediate_per_hour,
        })
    }
}

impl FinanceSubscribeNewsTool {
    async fn ensure_catalog_rss_source(
        &self,
        catalog_source_id: String,
        start_now: bool,
    ) -> anyhow::Result<SubscribedNewsSource> {
        let source = find_catalog_source(&catalog_source_id)?;
        anyhow::ensure!(
            source.can_enable(),
            "finance feed source {catalog_source_id} requires configuration: {}",
            source
                .setup_hint
                .as_deref()
                .unwrap_or("configure this provider before enabling it")
        );
        anyhow::ensure!(
            source.feed_type == FeedType::Rss,
            "catalog source {catalog_source_id} is not an rss source"
        );
        let source_name = source.feed_name();
        let existing = self
            .data_feed_svc
            .list_feeds()
            .await?
            .into_iter()
            .find(|feed| feed.name == source_name);
        let created = existing.is_none();
        let config = config_from_source(&source, existing)?;
        self.ensure_rss_config(config, Some(catalog_source_id), created, start_now)
            .await
    }

    async fn ensure_existing_rss_source(
        &self,
        feed_id: String,
        start_now: bool,
    ) -> anyhow::Result<SubscribedNewsSource> {
        let config = self
            .data_feed_svc
            .get_feed(&feed_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("unknown data feed id: {feed_id}"))?;
        ensure_finance_feed_source(&config)?;
        self.ensure_rss_config(config, None, false, start_now).await
    }

    async fn ensure_rss_config(
        &self,
        mut config: DataFeedConfig,
        catalog_source_id: Option<String>,
        created: bool,
        start_now: bool,
    ) -> anyhow::Result<SubscribedNewsSource> {
        anyhow::ensure!(
            config.feed_type == FeedType::Rss,
            "finance_subscribe_news requires rss feed sources"
        );
        let was_enabled = config.enabled;
        let had_runtime_error = config.status != FeedStatus::Idle || config.last_error.is_some();
        let was_running = self.data_feed_registry.is_running(&config.name);
        config.enabled = true;
        config.status = FeedStatus::Idle;
        config.last_error = None;
        config.updated_at = Timestamp::now();

        let feed_updated = if created {
            self.data_feed_svc.create_feed(&config).await?;
            self.data_feed_registry.register(config.clone())?;
            true
        } else if !was_enabled || had_runtime_error {
            anyhow::ensure!(
                self.data_feed_svc.update_feed(&config).await?,
                "failed to update RSS feed source: {}",
                config.name
            );
            if was_running {
                replace_registry_config_preserving_runtime(
                    &self.data_feed_registry,
                    config.clone(),
                )?;
            } else {
                replace_registry_config(&self.data_feed_registry, config.clone())?;
            }
            true
        } else if self.data_feed_registry.get(&config.name).is_none() {
            self.data_feed_registry.register(config.clone())?;
            false
        } else {
            false
        };

        let started = start_now && !was_running;
        if started {
            start_feed_task(&config, &self.data_feed_registry);
        }
        let running = self.data_feed_registry.is_running(&config.name);

        Ok(SubscribedNewsSource {
            feed_id: config.id,
            source_name: config.name,
            catalog_source_id,
            feed_change: if created {
                FeedChange::Created
            } else if feed_updated {
                FeedChange::Updated
            } else {
                FeedChange::Unchanged
            },
            started,
            running,
        })
    }

    async fn upsert_news_subscription(
        &self,
        context: &ToolContext,
        source_names: &[String],
        category_tags: &[String],
        watch_terms: &[String],
        delivery: FinanceDelivery,
        cooldown_secs: u64,
        max_immediate_per_hour: u16,
    ) -> anyhow::Result<(Uuid, bool)> {
        let owner = UserId(context.user_id.clone());
        let existing = self
            .finance_registry
            .list_for_owner(&owner)
            .await
            .into_iter()
            .find(|subscription| {
                subscription.session_key == context.session_key
                    && set_eq(&subscription.source_names, source_names)
                    && set_eq(&subscription.category_tags, category_tags)
                    && set_eq(&subscription.watch_terms, watch_terms)
                    && set_eq(&subscription.event_kinds, &[FinanceEventKind::RssArticle])
            });
        let id = existing.as_ref().map_or_else(Uuid::new_v4, |sub| sub.id);
        let subscription = FinanceSubscription {
            id,
            owner,
            session_key: context.session_key,
            event_kinds: vec![FinanceEventKind::RssArticle],
            source_names: source_names.to_vec(),
            category_tags: category_tags.to_vec(),
            watch_terms: watch_terms.to_vec(),
            venues: Vec::new(),
            symbols: Vec::new(),
            timeframes: Vec::new(),
            delivery,
            cooldown_secs,
            max_immediate_per_hour,
        };
        let id = self.finance_registry.upsert(subscription).await?;
        Ok((id, existing.is_none()))
    }
}

#[async_trait]
impl ToolExecute for FinanceSubscribeInstrumentsTool {
    type Output = FinanceSubscribeInstrumentsResult;
    type Params = FinanceSubscribeInstrumentsParams;

    async fn run(
        &self,
        params: FinanceSubscribeInstrumentsParams,
        context: &ToolContext,
    ) -> anyhow::Result<FinanceSubscribeInstrumentsResult> {
        let start_now = params.start_now.unwrap_or(true);
        let symbols = normalize_instrument_values("symbols", params.symbols, true, MAX_SYMBOLS)?;
        let timeframes =
            normalize_instrument_values("timeframes", params.timeframes, false, MAX_TIMEFRAMES)?;
        let delivery = params.delivery.unwrap_or(FinanceDelivery::Silent);
        let cooldown_secs = params.cooldown_secs.unwrap_or(DEFAULT_COOLDOWN_SECS);
        anyhow::ensure!(cooldown_secs <= 86_400, "cooldown_secs must be <= 86400");
        let max_immediate_per_hour = params
            .max_immediate_per_hour
            .unwrap_or(DEFAULT_MAX_IMMEDIATE_PER_HOUR);
        anyhow::ensure!(
            max_immediate_per_hour <= 60,
            "max_immediate_per_hour must be <= 60"
        );

        let source_ref =
            SourceRef::from_params_or_default_market(params.catalog_source_id, params.feed_id)?;
        let FeedResolution {
            mut config,
            catalog_source_id,
            created,
            replace_existing_instruments,
        } = self.resolve_feed(source_ref).await?;
        anyhow::ensure!(
            config.feed_type == FeedType::MarketCandle,
            "finance_subscribe_instruments requires a market_candle feed source"
        );

        let original_transport = config.transport.clone();
        normalize_finance_feed_config(&mut config)?;
        let normalized_transport_changed = config.transport != original_transport;

        let requested_venue = params.venue.map(normalize_venue).transpose()?;
        let feed_venue = config
            .transport
            .get("venue")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let venue = match requested_venue {
            Some(requested_venue) => {
                anyhow::ensure!(
                    requested_venue == feed_venue,
                    "requested venue {requested_venue} does not match market candle feed venue \
                     {feed_venue}"
                );
                requested_venue
            }
            None => feed_venue,
        };
        anyhow::ensure!(!venue.is_empty(), "market candle venue is required");

        let was_enabled = config.enabled;
        let was_running = self.data_feed_registry.is_running(&config.name);
        let transport_changed = apply_market_candle_selection(
            &mut config.transport,
            &symbols,
            &timeframes,
            replace_existing_instruments,
        )?;
        config.enabled = true;
        config.status = FeedStatus::Idle;
        config.last_error = None;
        config.updated_at = Timestamp::now();

        let feed_updated = if created {
            self.data_feed_svc.create_feed(&config).await?;
            self.data_feed_registry.register(config.clone())?;
            true
        } else if normalized_transport_changed || transport_changed || !was_enabled {
            anyhow::ensure!(
                self.data_feed_svc.update_feed(&config).await?,
                "failed to update market candle feed source: {}",
                config.name
            );
            if start_now && was_running {
                replace_registry_config(&self.data_feed_registry, config.clone())?;
            } else {
                replace_registry_config_preserving_runtime(
                    &self.data_feed_registry,
                    config.clone(),
                )?;
            }
            true
        } else {
            false
        };

        let feed_restarted = start_now && (created || transport_changed || !was_running);
        if feed_restarted {
            if self.data_feed_registry.get(&config.name).is_none() {
                self.data_feed_registry.register(config.clone())?;
            }
            start_feed_task(&config, &self.data_feed_registry);
        }

        let (subscription_id, subscription_created) = self
            .upsert_finance_subscription(
                context,
                &config.name,
                &venue,
                &symbols,
                &timeframes,
                delivery,
                cooldown_secs,
                max_immediate_per_hour,
            )
            .await?;

        Ok(FinanceSubscribeInstrumentsResult {
            subscription_id,
            subscription_created,
            diagnostic_tool: Some("finance_diagnose_candle_subscriptions".to_owned()),
            diagnostic_subscription_id: Some(subscription_id),
            feed_id: config.id,
            source_name: config.name.clone(),
            catalog_source_id,
            venue,
            symbols,
            timeframes,
            feed_change: if created {
                FeedChange::Created
            } else if feed_updated {
                FeedChange::Updated
            } else {
                FeedChange::Unchanged
            },
            feed_restarted,
            running: self.data_feed_registry.is_running(&config.name),
        })
    }
}

impl FinanceSubscribeInstrumentsTool {
    async fn resolve_feed(&self, source_ref: SourceRef) -> anyhow::Result<FeedResolution> {
        match source_ref {
            SourceRef::Catalog(catalog_source_id) => {
                let source = find_catalog_source(&catalog_source_id)?;
                anyhow::ensure!(
                    source.can_enable(),
                    "finance feed source {catalog_source_id} requires configuration: {}",
                    source
                        .setup_hint
                        .as_deref()
                        .unwrap_or("configure this provider before enabling it")
                );
                anyhow::ensure!(
                    source.feed_type == FeedType::MarketCandle,
                    "catalog source {catalog_source_id} is not a market_candle source"
                );
                let source_name = source.feed_name();
                let existing = self
                    .data_feed_svc
                    .list_feeds()
                    .await?
                    .into_iter()
                    .find(|feed| feed.name == source_name);
                let created = existing.is_none();
                let config = config_from_source(&source, existing)?;
                Ok(FeedResolution {
                    config,
                    catalog_source_id: Some(catalog_source_id),
                    created,
                    replace_existing_instruments: created,
                })
            }
            SourceRef::FeedId(feed_id) => {
                let config = self
                    .data_feed_svc
                    .get_feed(&feed_id)
                    .await?
                    .ok_or_else(|| anyhow::anyhow!("unknown data feed id: {feed_id}"))?;
                ensure_finance_feed_source(&config)?;
                Ok(FeedResolution {
                    config,
                    catalog_source_id: None,
                    created: false,
                    replace_existing_instruments: false,
                })
            }
        }
    }

    async fn upsert_finance_subscription(
        &self,
        context: &ToolContext,
        source_name: &str,
        venue: &str,
        symbols: &[String],
        timeframes: &[String],
        delivery: FinanceDelivery,
        cooldown_secs: u64,
        max_immediate_per_hour: u16,
    ) -> anyhow::Result<(Uuid, bool)> {
        let owner = UserId(context.user_id.clone());
        let existing = self
            .finance_registry
            .list_for_owner(&owner)
            .await
            .into_iter()
            .find(|subscription| {
                subscription.session_key == context.session_key
                    && set_eq(&subscription.source_names, &[source_name.to_owned()])
                    && set_eq(&subscription.venues, &[venue.to_owned()])
                    && set_eq(&subscription.symbols, symbols)
                    && set_eq(&subscription.timeframes, timeframes)
                    && set_eq(
                        &subscription.event_kinds,
                        &[FinanceEventKind::MarketCandleClosed],
                    )
            });
        let id = existing.as_ref().map_or_else(Uuid::new_v4, |sub| sub.id);
        let subscription = FinanceSubscription {
            id,
            owner,
            session_key: context.session_key,
            event_kinds: vec![FinanceEventKind::MarketCandleClosed],
            source_names: vec![source_name.to_owned()],
            category_tags: Vec::new(),
            watch_terms: Vec::new(),
            venues: vec![venue.to_owned()],
            symbols: symbols.to_vec(),
            timeframes: timeframes.to_vec(),
            delivery,
            cooldown_secs,
            max_immediate_per_hour,
        };
        let id = self.finance_registry.upsert(subscription).await?;
        Ok((id, existing.is_none()))
    }
}

enum SourceRef {
    Catalog(String),
    FeedId(String),
}

impl SourceRef {
    fn from_params(
        catalog_source_id: Option<String>,
        feed_id: Option<String>,
    ) -> anyhow::Result<Self> {
        match (catalog_source_id, feed_id) {
            (Some(catalog_source_id), None) => Ok(Self::Catalog(normalize_catalog_source_id(
                catalog_source_id,
            )?)),
            (None, Some(feed_id)) => Ok(Self::FeedId(normalize_feed_id(feed_id)?)),
            (Some(_), Some(_)) => {
                anyhow::bail!("provide either catalog_source_id or feed_id, not both")
            }
            (None, None) => anyhow::bail!("catalog_source_id or feed_id is required"),
        }
    }

    fn from_params_or_default_market(
        catalog_source_id: Option<String>,
        feed_id: Option<String>,
    ) -> anyhow::Result<Self> {
        match (catalog_source_id, feed_id) {
            (None, None) => Ok(Self::Catalog(
                DEFAULT_MARKET_CANDLE_CATALOG_SOURCE_ID.to_owned(),
            )),
            (catalog_source_id, feed_id) => Self::from_params(catalog_source_id, feed_id),
        }
    }
}

struct FeedResolution {
    config: DataFeedConfig,
    catalog_source_id: Option<String>,
    created: bool,
    replace_existing_instruments: bool,
}

struct FeedEventSourceRef {
    source_name:       String,
    catalog_source_id: Option<String>,
    feed_id:           Option<String>,
}

async fn resolve_feed_event_sources(
    svc: &DataFeedSvc,
    catalog_source_ids: Vec<String>,
    source_names: Vec<String>,
    feed_ids: Vec<String>,
) -> anyhow::Result<Vec<FeedEventSourceRef>> {
    let catalog_source_ids = normalize_string_refs(
        "catalog_source_ids",
        catalog_source_ids,
        MAX_EVENT_SOURCE_REFS,
        MAX_CATALOG_SOURCE_ID_LEN,
    )?;
    let source_names = normalize_string_refs(
        "source_names",
        source_names,
        MAX_EVENT_SOURCE_REFS,
        MAX_SOURCE_NAME_LEN,
    )?;
    let feed_ids =
        normalize_string_refs("feed_ids", feed_ids, MAX_EVENT_SOURCE_REFS, MAX_FEED_ID_LEN)?;
    anyhow::ensure!(
        !catalog_source_ids.is_empty() || !source_names.is_empty() || !feed_ids.is_empty(),
        "catalog_source_ids, source_names, or feed_ids is required"
    );
    anyhow::ensure!(
        catalog_source_ids.len() + source_names.len() + feed_ids.len() <= MAX_EVENT_SOURCE_REFS,
        "too many feed event sources"
    );

    let feeds = svc.list_feeds().await?;
    let catalog_by_source_name = catalog_by_source_name();
    let mut refs = Vec::new();

    for catalog_source_id in catalog_source_ids {
        let catalog_source = find_catalog_source(&catalog_source_id)?;
        let source_name = catalog_source.feed_name();
        let feed_id = feeds
            .iter()
            .find(|feed| feed.name == source_name)
            .map(|feed| feed.id.clone());
        push_unique_feed_event_source(
            &mut refs,
            FeedEventSourceRef {
                source_name,
                catalog_source_id: Some(catalog_source_id),
                feed_id,
            },
        );
    }

    for source_name in source_names {
        let feed = feeds.iter().find(|feed| feed.name == source_name);
        if let Some(feed) = feed {
            ensure_finance_feed_source(feed)?;
        } else {
            anyhow::ensure!(
                catalog_by_source_name.contains_key(&source_name),
                "source_name is not a known finance feed source: {source_name}"
            );
        }
        push_unique_feed_event_source(
            &mut refs,
            FeedEventSourceRef {
                catalog_source_id: catalog_by_source_name
                    .get(&source_name)
                    .map(|source| source.id.clone()),
                feed_id: feed.map(|feed| feed.id.clone()),
                source_name,
            },
        );
    }

    for feed_id in feed_ids {
        let config = svc
            .get_feed(&feed_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("unknown data feed id: {feed_id}"))?;
        ensure_finance_feed_source(&config)?;
        let catalog_source_id = catalog_by_source_name
            .get(&config.name)
            .map(|source| source.id.clone());
        push_unique_feed_event_source(
            &mut refs,
            FeedEventSourceRef {
                source_name: config.name,
                catalog_source_id,
                feed_id: Some(feed_id),
            },
        );
    }

    Ok(refs)
}

fn push_unique_feed_event_source(
    refs: &mut Vec<FeedEventSourceRef>,
    candidate: FeedEventSourceRef,
) {
    if refs
        .iter()
        .any(|existing| existing.source_name == candidate.source_name)
    {
        return;
    }
    refs.push(candidate);
}

fn normalize_catalog_source_id(value: String) -> anyhow::Result<String> {
    let value = value.trim();
    anyhow::ensure!(!value.is_empty(), "catalog_source_id must not be empty");
    anyhow::ensure!(
        value.chars().count() <= MAX_CATALOG_SOURCE_ID_LEN,
        "catalog_source_id is too long"
    );
    Ok(value.to_owned())
}

fn normalize_feed_id(value: String) -> anyhow::Result<String> {
    let value = value.trim();
    anyhow::ensure!(!value.is_empty(), "feed_id must not be empty");
    anyhow::ensure!(
        value.chars().count() <= MAX_FEED_ID_LEN,
        "feed_id is too long"
    );
    Ok(value.to_owned())
}

fn normalize_string_refs(
    name: &str,
    values: Vec<String>,
    max_values: usize,
    max_len: usize,
) -> anyhow::Result<Vec<String>> {
    anyhow::ensure!(values.len() <= max_values, "{name} has too many values");
    let mut out = Vec::with_capacity(values.len());
    for value in values {
        let value = value.trim();
        anyhow::ensure!(!value.is_empty(), "{name} contains an empty value");
        anyhow::ensure!(value.chars().count() <= max_len, "{name} value is too long");
        let value = value.to_owned();
        if !out.contains(&value) {
            out.push(value);
        }
    }
    Ok(out)
}

fn normalize_news_values(
    name: &str,
    values: Vec<String>,
    max_values: usize,
) -> anyhow::Result<Vec<String>> {
    anyhow::ensure!(values.len() <= max_values, "{name} has too many values");
    let mut out = Vec::with_capacity(values.len());
    for value in values {
        let value = value.split_whitespace().collect::<Vec<_>>().join(" ");
        anyhow::ensure!(!value.is_empty(), "{name} contains an empty value");
        anyhow::ensure!(
            value.chars().count() <= MAX_NEWS_SELECTOR_LEN,
            "{name} value is too long"
        );
        if !out.contains(&value) {
            out.push(value);
        }
    }
    Ok(out)
}

fn normalize_news_category_tags(values: Vec<String>) -> anyhow::Result<Vec<String>> {
    normalize_news_values("category_tags", values, MAX_NEWS_CATEGORY_TAGS).map(|values| {
        dedupe_strings(
            values
                .into_iter()
                .filter_map(|tag| {
                    let normalized = if tag
                        .get(.."category:".len())
                        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("category:"))
                    {
                        normalize_category_tag(&tag["category:".len()..])
                    } else {
                        normalize_category_tag(&tag)
                    };
                    (!normalized.is_empty()).then(|| format!("category:{normalized}"))
                })
                .collect(),
        )
    })
}

fn normalize_news_watch_terms(values: Vec<String>) -> anyhow::Result<Vec<String>> {
    normalize_news_values("watch_terms", values, MAX_NEWS_WATCH_TERMS).map(|values| {
        dedupe_strings(
            values
                .into_iter()
                .map(|term| {
                    term.chars()
                        .flat_map(char::to_lowercase)
                        .collect::<String>()
                })
                .collect(),
        )
    })
}

fn normalize_category_tag(value: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in value.chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            last_dash = false;
        } else if !last_dash && !out.is_empty() {
            out.push('-');
            last_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

fn dedupe_strings(mut values: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    values.retain(|value| seen.insert(value.clone()));
    values
}

fn normalize_venue(value: String) -> anyhow::Result<String> {
    let value = value.trim().to_ascii_lowercase();
    anyhow::ensure!(!value.is_empty(), "venue must not be empty");
    anyhow::ensure!(
        value.chars().count() <= MAX_INSTRUMENT_SELECTOR_LEN,
        "venue is too long"
    );
    Ok(value)
}

fn normalize_instrument_values(
    name: &str,
    values: Vec<String>,
    uppercase: bool,
    max_values: usize,
) -> anyhow::Result<Vec<String>> {
    anyhow::ensure!(!values.is_empty(), "{name} must not be empty");
    anyhow::ensure!(values.len() <= max_values, "{name} has too many values");
    let mut out = Vec::with_capacity(values.len());
    for value in values {
        let normalized = normalize_instrument_value(name, &value, uppercase)?;
        if !out.contains(&normalized) {
            out.push(normalized);
        }
    }
    Ok(out)
}

fn normalize_instrument_value(name: &str, value: &str, uppercase: bool) -> anyhow::Result<String> {
    let value = value.trim();
    anyhow::ensure!(!value.is_empty(), "{name} contains an empty value");
    anyhow::ensure!(
        value.chars().count() <= MAX_INSTRUMENT_SELECTOR_LEN,
        "{name} value is too long"
    );
    Ok(if uppercase {
        value.to_ascii_uppercase()
    } else {
        value.to_ascii_lowercase()
    })
}

fn apply_market_candle_selection(
    transport: &mut serde_json::Value,
    symbols: &[String],
    timeframes: &[String],
    replace_existing: bool,
) -> anyhow::Result<bool> {
    let before = transport.clone();
    let object = transport
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("market candle transport must be a JSON object"))?;
    object.insert(
        "symbols".to_owned(),
        serde_json::Value::Array(
            merge_json_string_array(
                "symbols",
                object.get("symbols"),
                symbols,
                replace_existing,
                true,
                MAX_SYMBOLS,
            )?
            .into_iter()
            .map(serde_json::Value::String)
            .collect(),
        ),
    );
    object.insert(
        "timeframes".to_owned(),
        serde_json::Value::Array(
            merge_json_string_array(
                "timeframes",
                object.get("timeframes"),
                timeframes,
                replace_existing,
                false,
                MAX_TIMEFRAMES,
            )?
            .into_iter()
            .map(serde_json::Value::String)
            .collect(),
        ),
    );
    Ok(*transport != before)
}

fn merge_json_string_array(
    name: &str,
    current: Option<&serde_json::Value>,
    requested: &[String],
    replace_existing: bool,
    uppercase: bool,
    max_values: usize,
) -> anyhow::Result<Vec<String>> {
    if replace_existing {
        return Ok(requested.to_vec());
    }
    let existing = match current {
        Some(serde_json::Value::Array(values)) => values
            .iter()
            .map(|value| {
                value.as_str().map(str::to_owned).ok_or_else(|| {
                    anyhow::anyhow!("market candle transport arrays must be strings")
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()?,
        Some(_) => anyhow::bail!("market candle transport arrays must be arrays"),
        None => Vec::new(),
    };
    let mut out = Vec::new();
    for value in existing {
        let normalized = normalize_instrument_value(name, &value, uppercase)?;
        if !out.contains(&normalized) {
            out.push(normalized);
        }
    }
    anyhow::ensure!(out.len() <= max_values, "{name} has too many values");
    for value in requested {
        if !out.contains(value) {
            out.push(value.clone());
        }
    }
    anyhow::ensure!(out.len() <= max_values, "{name} has too many values");
    Ok(out)
}

fn set_eq<T>(left: &[T], right: &[T]) -> bool
where
    T: Eq + std::hash::Hash,
{
    left.iter().collect::<HashSet<_>>() == right.iter().collect::<HashSet<_>>()
}

struct UnsubscribeSelector {
    subscription_ids:     Vec<Uuid>,
    source_names:         Vec<String>,
    event_kinds:          Vec<FinanceEventKind>,
    symbols:              Vec<String>,
    timeframes:           Vec<String>,
    current_session_only: bool,
    dry_run:              bool,
}

fn normalize_unsubscribe_selector(
    params: FinanceUnsubscribeParams,
) -> anyhow::Result<UnsubscribeSelector> {
    let mut subscription_ids = params.subscription_ids;
    if let Some(subscription_id) = params.subscription_id {
        subscription_ids.push(subscription_id);
    }
    subscription_ids.sort_unstable();
    subscription_ids.dedup();

    let catalog_source_ids = params
        .catalog_source_ids
        .into_iter()
        .map(normalize_catalog_source_id)
        .collect::<anyhow::Result<Vec<_>>>()?;
    let mut source_names = normalize_string_refs(
        "source_names",
        params.source_names,
        MAX_UNSUBSCRIBE_SOURCE_REFS,
        MAX_NEWS_SELECTOR_LEN,
    )?;
    for catalog_source_id in catalog_source_ids {
        let source_name = find_catalog_source(&catalog_source_id)?.feed_name();
        if !source_names.contains(&source_name) {
            source_names.push(source_name);
        }
    }
    anyhow::ensure!(
        source_names.len() <= MAX_UNSUBSCRIBE_SOURCE_REFS,
        "source_names has too many values"
    );

    let event_kinds = dedupe_event_kinds(params.event_kinds);
    let symbols =
        normalize_optional_instrument_values("symbols", params.symbols, true, MAX_SYMBOLS)?;
    let timeframes = normalize_optional_instrument_values(
        "timeframes",
        params.timeframes,
        false,
        MAX_TIMEFRAMES,
    )?;
    let has_ids = !subscription_ids.is_empty();
    anyhow::ensure!(
        has_ids
            || !source_names.is_empty()
            || !event_kinds.is_empty()
            || !symbols.is_empty()
            || !timeframes.is_empty(),
        "finance_unsubscribe requires subscription_ids or at least one selector filter"
    );

    Ok(UnsubscribeSelector {
        subscription_ids,
        source_names,
        event_kinds,
        symbols,
        timeframes,
        current_session_only: params.current_session_only.unwrap_or(!has_ids),
        dry_run: params.dry_run.unwrap_or(false),
    })
}

fn normalize_optional_instrument_values(
    name: &str,
    values: Vec<String>,
    uppercase: bool,
    max_values: usize,
) -> anyhow::Result<Vec<String>> {
    if values.is_empty() {
        Ok(Vec::new())
    } else {
        normalize_instrument_values(name, values, uppercase, max_values)
    }
}

fn dedupe_event_kinds(values: Vec<FinanceEventKind>) -> Vec<FinanceEventKind> {
    let mut out = Vec::with_capacity(values.len());
    for value in values {
        if !out.contains(&value) {
            out.push(value);
        }
    }
    out
}

fn finance_event_kind_type(kind: FinanceEventKind) -> &'static str {
    match kind {
        FinanceEventKind::RssArticle => "rss_article",
        FinanceEventKind::MarketCandleClosed => "market_candle_closed",
    }
}

fn unsubscribe_selector_matches(
    subscription: &FinanceSubscription,
    selector: &UnsubscribeSelector,
    context: &ToolContext,
) -> bool {
    if selector.current_session_only && subscription.session_key != context.session_key {
        return false;
    }
    if !selector.subscription_ids.is_empty()
        && !selector.subscription_ids.contains(&subscription.id)
    {
        return false;
    }
    selector_group_matches(&subscription.source_names, &selector.source_names)
        && selector_group_matches(&subscription.event_kinds, &selector.event_kinds)
        && selector_group_matches(&subscription.symbols, &selector.symbols)
        && selector_group_matches(&subscription.timeframes, &selector.timeframes)
}

fn selector_group_matches<T>(subscription_values: &[T], selector_values: &[T]) -> bool
where
    T: Eq,
{
    selector_values.is_empty()
        || (!subscription_values.is_empty()
            && subscription_values
                .iter()
                .any(|value| selector_values.contains(value)))
}

fn unsubscribe_match(
    subscription: FinanceSubscription,
    current_session: rara_kernel::session::SessionKey,
) -> FinanceUnsubscribeMatch {
    FinanceUnsubscribeMatch {
        subscription_id: subscription.id,
        current_session: subscription.session_key == current_session,
        session_key:     subscription.session_key.to_string(),
        event_kinds:     subscription.event_kinds,
        source_names:    subscription.source_names,
        venues:          subscription.venues,
        symbols:         subscription.symbols,
        timeframes:      subscription.timeframes,
        delivery:        subscription.delivery,
    }
}

fn find_catalog_source(catalog_source_id: &str) -> anyhow::Result<DefaultFeedSource> {
    default_finance_feed_sources()
        .into_iter()
        .find(|source| source.id == catalog_source_id)
        .ok_or_else(|| {
            anyhow::anyhow!("unknown finance feed catalog source id: {catalog_source_id}")
        })
}

fn catalog_by_source_name() -> HashMap<String, DefaultFeedSource> {
    default_finance_feed_sources()
        .into_iter()
        .map(|source| (source.feed_name(), source))
        .collect()
}

async fn resolve_existing_finance_feed(
    svc: &DataFeedSvc,
    source_ref: SourceRef,
) -> anyhow::Result<(DataFeedConfig, Option<String>)> {
    match source_ref {
        SourceRef::Catalog(catalog_source_id) => {
            let source = find_catalog_source(&catalog_source_id)?;
            let source_name = source.feed_name();
            let config = svc
                .list_feeds()
                .await?
                .into_iter()
                .find(|feed| feed.name == source_name)
                .ok_or_else(|| {
                    anyhow::anyhow!("finance feed source {catalog_source_id} is not enabled yet")
                })?;
            ensure_finance_feed_source(&config)?;
            Ok((config, Some(catalog_source_id)))
        }
        SourceRef::FeedId(feed_id) => {
            let config = svc
                .get_feed(&feed_id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("unknown data feed id: {feed_id}"))?;
            ensure_finance_feed_source(&config)?;
            Ok((config, None))
        }
    }
}

fn ensure_finance_feed_source(config: &DataFeedConfig) -> anyhow::Result<()> {
    anyhow::ensure!(
        matches!(config.feed_type, FeedType::Rss | FeedType::MarketCandle),
        "data feed {} is not a finance RSS or market-candle source",
        config.name
    );
    anyhow::ensure!(
        is_finance_feed_source(config),
        "data feed {} is not finance-scoped",
        config.name
    );
    Ok(())
}

fn is_finance_feed_source(config: &DataFeedConfig) -> bool {
    config.tags.iter().any(|tag| tag == "finance")
        || default_finance_feed_sources()
            .into_iter()
            .any(|source| source.feed_name() == config.name)
}

fn subscribe_tool_for_feed_type(feed_type: FeedType) -> Option<String> {
    match feed_type {
        FeedType::Rss => Some("finance_subscribe_news".to_owned()),
        FeedType::MarketCandle => Some("finance_subscribe_instruments".to_owned()),
        FeedType::Polling | FeedType::Webhook | FeedType::WebSocket => None,
    }
}

fn subscription_hint_for_source(
    catalog_source_id: &str,
    feed_type: FeedType,
    transport: Option<&serde_json::Value>,
) -> Option<FinanceFeedSourceSubscriptionHint> {
    match feed_type {
        FeedType::Rss => Some(FinanceFeedSourceSubscriptionHint {
            tool:            "finance_subscribe_news".to_owned(),
            default_params:  serde_json::json!({
                "catalog_source_ids": [catalog_source_id]
            }),
            required_params: Vec::new(),
            optional_params: [
                "category_tags",
                "watch_terms",
                "delivery",
                "start_now",
                "cooldown_secs",
                "max_immediate_per_hour",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            diagnostic_tool: None,
        }),
        FeedType::MarketCandle => {
            let venue = transport
                .and_then(|value| value.get("venue"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned);
            let symbols = transport.map_or_else(Vec::new, |value| {
                extract_normalized_string_array(value, "symbols", true)
            });
            let timeframes = transport.map_or_else(Vec::new, |value| {
                extract_normalized_string_array(value, "timeframes", false)
            });
            let mut default_params = serde_json::Map::from_iter([(
                "catalog_source_id".to_owned(),
                serde_json::Value::String(catalog_source_id.to_owned()),
            )]);
            if let Some(venue) = venue {
                default_params.insert("venue".to_owned(), serde_json::Value::String(venue));
            }
            if !symbols.is_empty() {
                default_params.insert("symbols".to_owned(), serde_json::json!(symbols));
            }
            if !timeframes.is_empty() {
                default_params.insert("timeframes".to_owned(), serde_json::json!(timeframes));
            }

            Some(FinanceFeedSourceSubscriptionHint {
                tool:            "finance_subscribe_instruments".to_owned(),
                default_params:  serde_json::Value::Object(default_params),
                required_params: ["symbols", "timeframes"]
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
                optional_params: [
                    "venue",
                    "delivery",
                    "start_now",
                    "cooldown_secs",
                    "max_immediate_per_hour",
                ]
                .into_iter()
                .map(str::to_owned)
                .collect(),
                diagnostic_tool: Some("finance_diagnose_candle_subscriptions".to_owned()),
            })
        }
        FeedType::Polling | FeedType::Webhook | FeedType::WebSocket => None,
    }
}

fn feed_source_entry(
    source: DefaultFeedSource,
    persisted: Option<&DataFeedConfig>,
    running: bool,
    event_count: i64,
    last_event_type: Option<String>,
    last_event_at: Option<String>,
    lag_seconds: Option<i64>,
    subscriptions: FinanceFeedSourceSubscriptions,
) -> FinanceFeedSourceEntry {
    let source_name = source.feed_name();
    let transport = persisted
        .map(|feed| &feed.transport)
        .or(source.transport.as_ref());
    let subscription_hint = subscription_hint_for_source(&source.id, source.feed_type, transport);
    let can_enable = source.can_enable();
    let provider = source.provider.clone().or_else(|| {
        transport
            .and_then(|value| value.get("provider"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    });
    FinanceFeedSourceEntry {
        id: source.id,
        name: source.name,
        description: source.description,
        feed_type: source.feed_type.to_string(),
        subscribe_tool: subscribe_tool_for_feed_type(source.feed_type),
        subscription_hint,
        provider,
        tags: source.tags,
        source_name,
        requires_configuration: source.requires_configuration,
        can_enable,
        setup_hint: source.setup_hint,
        runtime: FinanceFeedSourceRuntime {
            persisted: persisted.is_some(),
            feed_id: persisted.map(|feed| feed.id.clone()),
            enabled: persisted.is_some_and(|feed| feed.enabled),
            running,
            status: persisted.map(|feed| feed.status.to_string()),
            last_error: persisted.and_then(|feed| feed.last_error.clone()),
            event_count,
            last_event_type,
            last_event_at,
            lag_seconds,
        },
        subscriptions,
        venue: transport
            .and_then(|value| value.get("venue"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        configured_symbols: transport.map_or_else(Vec::new, |value| {
            extract_normalized_string_array(value, "symbols", true)
        }),
        configured_timeframes: transport.map_or_else(Vec::new, |value| {
            extract_normalized_string_array(value, "timeframes", false)
        }),
    }
}

fn subscription_entry(
    subscription: FinanceSubscription,
    current_session: rara_kernel::session::SessionKey,
    feeds_by_name: &HashMap<String, &DataFeedConfig>,
    catalog_by_source_name: &HashMap<String, DefaultFeedSource>,
    registry: &DataFeedRegistry,
) -> FinanceSubscriptionEntry {
    let matches_all_sources = subscription.source_names.is_empty();
    let sources = subscription
        .source_names
        .iter()
        .map(|source_name| {
            subscription_source_entry(
                source_name,
                feeds_by_name.get(source_name).copied(),
                catalog_by_source_name.get(source_name),
                registry,
            )
        })
        .collect();

    let diagnostic_tool = subscription_is_market_candle(&subscription)
        .then(|| "finance_diagnose_candle_subscriptions".to_owned());
    let diagnostic_subscription_id = diagnostic_tool.as_ref().map(|_| subscription.id);
    let unsubscribe_hint = Some(unsubscribe_hint_for_subscription(subscription.id));
    let events_hint = events_hint_for_subscription(&subscription);

    FinanceSubscriptionEntry {
        subscription_id: subscription.id,
        current_session: subscription.session_key == current_session,
        session_key: subscription.session_key.to_string(),
        event_kinds: subscription.event_kinds,
        diagnostic_tool,
        diagnostic_subscription_id,
        unsubscribe_hint,
        events_hint,
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

fn events_hint_for_subscription(
    subscription: &FinanceSubscription,
) -> Option<FinanceSubscriptionEventsHint> {
    if subscription.source_names.is_empty() {
        return None;
    }
    let event_kinds = subscription
        .event_kinds
        .iter()
        .copied()
        .map(finance_event_kind_type)
        .collect::<Vec<_>>();

    Some(FinanceSubscriptionEventsHint {
        tool:            "finance_list_feed_events".to_owned(),
        default_params:  serde_json::json!({
            "source_names": subscription.source_names.clone(),
            "event_kinds": event_kinds,
            "since": "24h",
            "limit": DEFAULT_FEED_EVENT_LIMIT,
        }),
        required_params: Vec::new(),
        optional_params: ["since", "limit", "offset"]
            .into_iter()
            .map(str::to_owned)
            .collect(),
    })
}

fn unsubscribe_hint_for_subscription(subscription_id: Uuid) -> FinanceSubscriptionUnsubscribeHint {
    FinanceSubscriptionUnsubscribeHint {
        tool:            "finance_unsubscribe".to_owned(),
        default_params:  serde_json::json!({
            "subscription_ids": [subscription_id],
        }),
        required_params: Vec::new(),
        optional_params: vec!["dry_run".to_owned()],
    }
}

fn subscription_is_market_candle(subscription: &FinanceSubscription) -> bool {
    subscription
        .event_kinds
        .contains(&FinanceEventKind::MarketCandleClosed)
}

fn subscription_source_entry(
    source_name: &str,
    feed: Option<&DataFeedConfig>,
    catalog_source: Option<&DefaultFeedSource>,
    registry: &DataFeedRegistry,
) -> FinanceSubscriptionSource {
    FinanceSubscriptionSource {
        source_name:       source_name.to_owned(),
        catalog_source_id: catalog_source.map(|source| source.id.clone()),
        catalog_name:      catalog_source.map(|source| source.name.clone()),
        provider:          subscription_source_provider(catalog_source, feed),
        feed_id:           feed.map(|feed| feed.id.clone()),
        feed_type:         feed.map(|feed| feed.feed_type.to_string()),
        persisted:         feed.is_some(),
        enabled:           feed.map(|feed| feed.enabled),
        running:           registry.is_running(source_name),
        status:            feed.map(|feed| feed.status.to_string()),
        last_error:        feed.and_then(|feed| feed.last_error.clone()),
    }
}

fn subscription_source_provider(
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

fn source_subscription_summary(
    subscriptions: &[FinanceSubscription],
    session_key: rara_kernel::session::SessionKey,
    source_name: &str,
) -> FinanceFeedSourceSubscriptions {
    let mut user_subscription_ids = Vec::new();
    let mut session_subscription_ids = Vec::new();

    for subscription in subscriptions {
        if !subscription
            .source_names
            .iter()
            .any(|value| value == source_name)
        {
            continue;
        }
        user_subscription_ids.push(subscription.id);
        if subscription.session_key == session_key {
            session_subscription_ids.push(subscription.id);
        }
    }

    FinanceFeedSourceSubscriptions {
        user_subscribed: !user_subscription_ids.is_empty(),
        session_subscribed: !session_subscription_ids.is_empty(),
        user_subscription_ids,
        session_subscription_ids,
    }
}

fn extract_normalized_string_array(
    value: &serde_json::Value,
    key: &str,
    uppercase: bool,
) -> Vec<String> {
    value
        .get(key)
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            let mut out = Vec::new();
            for item in items {
                let Some(value) = item.as_str() else {
                    continue;
                };
                let Ok(normalized) = normalize_instrument_value(key, value, uppercase) else {
                    continue;
                };
                if !out.contains(&normalized) {
                    out.push(normalized);
                }
            }
            out
        })
        .unwrap_or_default()
}

fn config_from_source(
    source: &DefaultFeedSource,
    existing: Option<DataFeedConfig>,
) -> anyhow::Result<DataFeedConfig> {
    let now = Timestamp::now();
    let id = existing
        .as_ref()
        .map(|feed| feed.id.clone())
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let created_at = existing.as_ref().map(|feed| feed.created_at).unwrap_or(now);
    let transport = existing
        .as_ref()
        .map(|feed| feed.transport.clone())
        .or_else(|| source.transport.clone())
        .ok_or_else(|| anyhow::anyhow!("finance feed source {} has no transport", source.id))?;
    let auth = existing
        .as_ref()
        .and_then(|feed| feed.auth.clone())
        .or_else(|| source.auth.clone());
    let tags = existing.as_ref().map_or_else(
        || source.tags.clone(),
        |feed| merge_tags(source.tags.clone(), &feed.tags),
    );

    let mut config = DataFeedConfig::builder()
        .id(id)
        .name(source.feed_name())
        .feed_type(source.feed_type)
        .tags(tags)
        .transport(transport)
        .maybe_auth(auth)
        .enabled(true)
        .status(FeedStatus::Idle)
        .maybe_last_error(None)
        .created_at(created_at)
        .updated_at(now)
        .build();
    normalize_finance_feed_config(&mut config)?;
    Ok(config)
}

fn normalize_finance_feed_config(config: &mut DataFeedConfig) -> anyhow::Result<()> {
    if config.feed_type == FeedType::MarketCandle {
        MarketCandleSource::normalize_config(config)?;
    }
    Ok(())
}

fn merge_tags(mut tags: Vec<String>, existing: &[String]) -> Vec<String> {
    for tag in existing {
        if !tags.contains(tag) {
            tags.push(tag.clone());
        }
    }
    tags
}

fn replace_registry_config(
    registry: &Arc<DataFeedRegistry>,
    config: DataFeedConfig,
) -> anyhow::Result<()> {
    if registry.get(&config.name).is_some() {
        registry.remove(&config.name)?;
    }
    registry.register(config)?;
    Ok(())
}

fn replace_registry_config_preserving_runtime(
    registry: &Arc<DataFeedRegistry>,
    config: DataFeedConfig,
) -> anyhow::Result<()> {
    if registry.get(&config.name).is_some() {
        registry.replace_config(config)?;
    } else {
        registry.register(config)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use diesel_async::RunQueryDsl;
    use rara_kernel::{
        data_feed::{DataFeedConfig, FeedEvent, FeedEventId, FeedStatus, FeedStore, FeedType},
        identity::UserId,
        io::MessageId,
        queue::{ShardedEventQueue, ShardedEventQueueConfig},
        session::SessionKey,
        tool::{AgentTool, ToolContext, ToolExecute},
    };
    use rara_trading::finance::registry::{
        FinanceDelivery, FinanceEventKind, FinanceSubscription, FinanceSubscriptionRegistry,
    };
    use uuid::Uuid;

    use super::{
        FeedChange, FinanceDisableFeedSourceParams, FinanceDisableFeedSourceTool,
        FinanceEnableFeedSourceParams, FinanceEnableFeedSourceTool, FinanceListFeedEventsParams,
        FinanceListFeedEventsTool, FinanceListFeedSourcesParams, FinanceListFeedSourcesTool,
        FinanceListSubscriptionsParams, FinanceListSubscriptionsTool,
        FinanceRestartFeedSourceParams, FinanceRestartFeedSourceTool,
        FinanceSubscribeInstrumentsParams, FinanceSubscribeInstrumentsTool,
        FinanceSubscribeNewsParams, FinanceSubscribeNewsTool, FinanceUnsubscribeParams,
        FinanceUnsubscribeTool,
    };

    fn context() -> ToolContext {
        ToolContext {
            user_id:               "alice".to_owned(),
            session_key:           SessionKey::new(),
            origin_endpoint:       None,
            origin_user_id:        None,
            event_queue:           Arc::new(ShardedEventQueue::new(ShardedEventQueueConfig {
                num_shards:      0,
                shard_capacity:  1,
                global_capacity: 16,
            })),
            rara_turn_id:          MessageId::new(),
            context_window_tokens: 0,
            tool_registry:         None,
            stream_handle:         None,
            tool_call_id:          None,
        }
    }

    async fn tool() -> (
        FinanceListFeedSourcesTool,
        FinanceEnableFeedSourceTool,
        FinanceDisableFeedSourceTool,
        FinanceRestartFeedSourceTool,
        FinanceSubscribeInstrumentsTool,
        rara_backend_admin::data_feeds::DataFeedSvc,
        Arc<rara_kernel::data_feed::DataFeedRegistry>,
        Arc<FinanceSubscriptionRegistry>,
    ) {
        let pools = rara_kernel::testing::build_memory_diesel_pools().await;
        bootstrap_data_feed_schema(&pools).await;
        let svc = rara_backend_admin::data_feeds::DataFeedSvc::new(pools);
        let (event_tx, _event_rx) = tokio::sync::mpsc::channel(16);
        let registry = Arc::new(rara_kernel::data_feed::DataFeedRegistry::new(event_tx));
        let finance_path = std::env::temp_dir().join(format!(
            "rara-finance-subscriptions-{}.json",
            uuid::Uuid::new_v4()
        ));
        let finance_registry = Arc::new(FinanceSubscriptionRegistry::load(finance_path));
        (
            FinanceListFeedSourcesTool::new(
                svc.clone(),
                registry.clone(),
                finance_registry.clone(),
            ),
            FinanceEnableFeedSourceTool::new(svc.clone(), registry.clone()),
            FinanceDisableFeedSourceTool::new(svc.clone(), registry.clone()),
            FinanceRestartFeedSourceTool::new(svc.clone(), registry.clone()),
            FinanceSubscribeInstrumentsTool::new(
                svc.clone(),
                registry.clone(),
                finance_registry.clone(),
            ),
            svc,
            registry,
            finance_registry,
        )
    }

    fn finance_subscription(
        ctx: &ToolContext,
        session_key: SessionKey,
        event_kinds: Vec<FinanceEventKind>,
        source_names: Vec<String>,
        symbols: Vec<String>,
        timeframes: Vec<String>,
    ) -> FinanceSubscription {
        FinanceSubscription {
            id: Uuid::new_v4(),
            owner: UserId(ctx.user_id.clone()),
            session_key,
            event_kinds,
            source_names,
            category_tags: Vec::new(),
            watch_terms: Vec::new(),
            venues: Vec::new(),
            symbols,
            timeframes,
            delivery: FinanceDelivery::Silent,
            cooldown_secs: 900,
            max_immediate_per_hour: 6,
        }
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
    async fn list_feed_sources_reports_catalog_and_absent_runtime_state() {
        let (tool, _enable, _disable, _restart, _subscribe, _svc, _registry, _finance_registry) =
            tool().await;

        let result = tool
            .run(FinanceListFeedSourcesParams {}, &context())
            .await
            .unwrap();

        let fed = result
            .sources
            .iter()
            .find(|source| source.id == "fed-press-releases")
            .expect("fed source should be listed");
        assert_eq!(fed.source_name, "finance-fed-press-releases");
        assert_eq!(
            fed.subscribe_tool.as_deref(),
            Some("finance_subscribe_news")
        );
        let fed_hint = fed
            .subscription_hint
            .as_ref()
            .expect("fed should include subscription hint");
        assert_eq!(fed_hint.tool, "finance_subscribe_news");
        assert_eq!(
            fed_hint.default_params,
            serde_json::json!({
                "catalog_source_ids": ["fed-press-releases"]
            })
        );
        assert!(fed_hint.required_params.is_empty());
        assert!(fed_hint.optional_params.contains(&"watch_terms".to_owned()));
        assert_eq!(fed_hint.diagnostic_tool, None);
        assert!(fed.can_enable);
        assert!(!fed.runtime.persisted);
        assert_eq!(fed.runtime.feed_id, None);
        assert!(!fed.runtime.enabled);
        assert!(!fed.runtime.running);
        assert_eq!(fed.runtime.status, None);
        assert_eq!(fed.runtime.event_count, 0);
        assert_eq!(fed.runtime.last_event_type, None);
        assert_eq!(fed.runtime.last_event_at, None);
        assert_eq!(fed.runtime.lag_seconds, None);
        assert!(!fed.subscriptions.user_subscribed);
        assert!(!fed.subscriptions.session_subscribed);
        assert!(fed.subscriptions.user_subscription_ids.is_empty());
        assert!(fed.subscriptions.session_subscription_ids.is_empty());

        let binance = result
            .sources
            .iter()
            .find(|source| source.id == "binance-market-candles")
            .expect("binance source should be listed");
        assert_eq!(binance.provider.as_deref(), Some("binance"));
        assert_eq!(
            binance.subscribe_tool.as_deref(),
            Some("finance_subscribe_instruments")
        );
        let binance_hint = binance
            .subscription_hint
            .as_ref()
            .expect("binance should include subscription hint");
        assert_eq!(binance_hint.tool, "finance_subscribe_instruments");
        assert_eq!(
            binance_hint.default_params,
            serde_json::json!({
                "catalog_source_id": "binance-market-candles",
                "venue": "binance",
                "symbols": ["BTCUSDT", "ETHUSDT"],
                "timeframes": ["1m"]
            })
        );
        assert_eq!(binance_hint.required_params, ["symbols", "timeframes"]);
        assert_eq!(
            binance_hint.diagnostic_tool.as_deref(),
            Some("finance_diagnose_candle_subscriptions")
        );
        assert_eq!(binance.venue.as_deref(), Some("binance"));
        assert_eq!(binance.configured_symbols, ["BTCUSDT", "ETHUSDT"]);
        assert_eq!(binance.configured_timeframes, ["1m"]);

        let longbridge = result
            .sources
            .iter()
            .find(|source| source.id == "longbridge-market-candles")
            .expect("longbridge source should be listed");
        assert_eq!(longbridge.provider.as_deref(), Some("longbridge"));
        assert_eq!(
            longbridge.subscribe_tool.as_deref(),
            Some("finance_subscribe_instruments")
        );
        let longbridge_hint = longbridge
            .subscription_hint
            .as_ref()
            .expect("longbridge should include subscription hint");
        assert_eq!(longbridge_hint.tool, "finance_subscribe_instruments");
        assert_eq!(
            longbridge_hint.default_params,
            serde_json::json!({
                "catalog_source_id": "longbridge-market-candles",
                "venue": "longbridge",
                "symbols": ["AAPL.US", "NVDA.US", "700.HK"],
                "timeframes": ["1d"]
            })
        );
        assert!(longbridge.requires_configuration);
        assert!(!longbridge.can_enable);
        assert!(
            longbridge
                .setup_hint
                .as_deref()
                .is_some_and(|hint| hint.contains("Longbridge"))
        );
    }

    #[tokio::test]
    async fn list_feed_sources_reports_persisted_enabled_source_state() {
        let (list, enable, _disable, _restart, _subscribe, _svc, _registry, _finance_registry) =
            tool().await;
        enable
            .run(
                FinanceEnableFeedSourceParams {
                    catalog_source_id: "fed-press-releases".to_owned(),
                    start_now:         Some(false),
                },
                &context(),
            )
            .await
            .unwrap();

        let result = list
            .run(FinanceListFeedSourcesParams {}, &context())
            .await
            .unwrap();
        let fed = result
            .sources
            .iter()
            .find(|source| source.id == "fed-press-releases")
            .expect("fed source should be listed");

        assert!(fed.runtime.persisted);
        assert!(
            fed.runtime
                .feed_id
                .as_deref()
                .is_some_and(|id| !id.is_empty())
        );
        assert!(fed.runtime.enabled);
        assert!(!fed.runtime.running);
        assert_eq!(fed.runtime.status.as_deref(), Some("idle"));
        assert_eq!(fed.runtime.last_error, None);
        assert_eq!(fed.runtime.event_count, 0);
    }

    #[tokio::test]
    async fn list_subscriptions_enriches_source_runtime_state() {
        let (
            _feed_sources,
            _enable,
            _disable,
            _restart,
            _subscribe,
            svc,
            registry,
            finance_registry,
        ) = tool().await;
        let subscribe_news =
            FinanceSubscribeNewsTool::new(svc.clone(), registry.clone(), finance_registry.clone());
        let list = FinanceListSubscriptionsTool::new(svc, registry, finance_registry);
        let ctx = context();
        let subscribed = subscribe_news
            .run(
                FinanceSubscribeNewsParams {
                    catalog_source_ids:     vec!["fed-press-releases".to_owned()],
                    feed_ids:               Vec::new(),
                    category_tags:          vec!["monetary policy".to_owned()],
                    watch_terms:            vec!["rate decision".to_owned()],
                    start_now:              Some(false),
                    delivery:               Some(FinanceDelivery::Immediate),
                    cooldown_secs:          Some(120),
                    max_immediate_per_hour: Some(2),
                },
                &ctx,
            )
            .await
            .unwrap();

        let result = list
            .run(
                FinanceListSubscriptionsParams {
                    current_session_only: None,
                },
                &ctx,
            )
            .await
            .unwrap();

        assert_eq!(result.count, 1);
        let subscription = &result.subscriptions[0];
        assert_eq!(subscription.subscription_id, subscribed.subscription_id);
        assert!(subscription.current_session);
        assert_eq!(subscription.session_key, ctx.session_key.to_string());
        assert_eq!(subscription.event_kinds, [FinanceEventKind::RssArticle]);
        assert_eq!(subscription.diagnostic_tool, None);
        assert_eq!(subscription.diagnostic_subscription_id, None);
        let unsubscribe_hint = subscription
            .unsubscribe_hint
            .as_ref()
            .expect("subscription should include unsubscribe hint");
        assert_eq!(unsubscribe_hint.tool, "finance_unsubscribe");
        assert_eq!(
            unsubscribe_hint.default_params,
            serde_json::json!({
                "subscription_ids": [subscribed.subscription_id],
            })
        );
        assert!(unsubscribe_hint.required_params.is_empty());
        assert_eq!(unsubscribe_hint.optional_params, ["dry_run"]);
        let events_hint = subscription
            .events_hint
            .as_ref()
            .expect("subscription should include events hint");
        assert_eq!(events_hint.tool, "finance_list_feed_events");
        assert_eq!(
            events_hint.default_params,
            serde_json::json!({
                "source_names": ["finance-fed-press-releases"],
                "event_kinds": ["rss_article"],
                "since": "24h",
                "limit": 20,
            })
        );
        assert!(events_hint.required_params.is_empty());
        assert_eq!(events_hint.optional_params, ["since", "limit", "offset"]);
        assert_eq!(subscription.source_names, ["finance-fed-press-releases"]);
        assert!(!subscription.matches_all_sources);
        assert_eq!(subscription.category_tags, ["category:monetary-policy"]);
        assert_eq!(subscription.watch_terms, ["rate decision"]);
        assert_eq!(subscription.delivery, FinanceDelivery::Immediate);
        assert_eq!(subscription.cooldown_secs, 120);
        assert_eq!(subscription.max_immediate_per_hour, 2);

        let source = &subscription.sources[0];
        assert_eq!(source.source_name, "finance-fed-press-releases");
        assert_eq!(
            source.catalog_source_id.as_deref(),
            Some("fed-press-releases")
        );
        assert_eq!(
            source.catalog_name.as_deref(),
            Some("Federal Reserve Press Releases")
        );
        assert!(source.feed_id.as_deref().is_some_and(|id| !id.is_empty()));
        assert_eq!(source.feed_type.as_deref(), Some("rss"));
        assert!(source.persisted);
        assert_eq!(source.enabled, Some(true));
        assert!(!source.running);
        assert_eq!(source.status.as_deref(), Some("idle"));
        assert_eq!(source.provider, None);
        assert_eq!(source.last_error, None);
    }

    #[tokio::test]
    async fn list_subscriptions_can_filter_to_current_session() {
        let (
            _feed_sources,
            _enable,
            _disable,
            _restart,
            _subscribe,
            svc,
            registry,
            finance_registry,
        ) = tool().await;
        let list = FinanceListSubscriptionsTool::new(svc, registry, finance_registry.clone());
        let ctx = context();
        let other_session = SessionKey::new();
        let owner = UserId(ctx.user_id.clone());

        for session_key in [ctx.session_key, other_session] {
            finance_registry
                .upsert(FinanceSubscription {
                    id: Uuid::new_v4(),
                    owner: owner.clone(),
                    session_key,
                    event_kinds: vec![FinanceEventKind::RssArticle],
                    source_names: vec!["custom-news".to_owned()],
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

        let all = list
            .run(
                FinanceListSubscriptionsParams {
                    current_session_only: Some(false),
                },
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(all.count, 2);

        let current = list
            .run(
                FinanceListSubscriptionsParams {
                    current_session_only: Some(true),
                },
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(current.count, 1);
        assert!(current.subscriptions[0].current_session);
        assert_eq!(
            current.subscriptions[0].session_key,
            ctx.session_key.to_string()
        );
        assert_eq!(
            current.subscriptions[0].sources[0].source_name,
            "custom-news"
        );
        assert!(!current.subscriptions[0].sources[0].persisted);
        assert_eq!(current.subscriptions[0].sources[0].catalog_source_id, None);
        assert_eq!(current.subscriptions[0].sources[0].enabled, None);
        assert!(!current.subscriptions[0].sources[0].running);
    }

    #[tokio::test]
    async fn unsubscribe_accepts_legacy_subscription_id() {
        let (
            _feed_sources,
            _enable,
            _disable,
            _restart,
            _subscribe,
            _svc,
            _registry,
            finance_registry,
        ) = tool().await;
        let ctx = context();
        let subscription = finance_subscription(
            &ctx,
            ctx.session_key,
            vec![FinanceEventKind::RssArticle],
            vec!["finance-fed-press-releases".to_owned()],
            Vec::new(),
            Vec::new(),
        );
        let subscription_id = subscription.id;
        finance_registry.upsert(subscription).await.unwrap();

        let unsubscribe = FinanceUnsubscribeTool::new(finance_registry.clone());
        let result = unsubscribe
            .run(
                FinanceUnsubscribeParams {
                    subscription_id: Some(subscription_id),
                    ..Default::default()
                },
                &ctx,
            )
            .await
            .unwrap();

        assert_eq!(result.matched_count, 1);
        assert_eq!(result.removed_count, 1);
        assert_eq!(result.removed_subscription_ids, [subscription_id]);
        assert!(
            finance_registry
                .list_for_owner(&UserId(ctx.user_id.clone()))
                .await
                .is_empty()
        );
    }

    #[tokio::test]
    async fn unsubscribe_matches_catalog_source_in_current_session_by_default() {
        let (
            _feed_sources,
            _enable,
            _disable,
            _restart,
            _subscribe,
            _svc,
            _registry,
            finance_registry,
        ) = tool().await;
        let ctx = context();
        let other_session = SessionKey::new();
        let current = finance_subscription(
            &ctx,
            ctx.session_key,
            vec![FinanceEventKind::RssArticle],
            vec!["finance-fed-press-releases".to_owned()],
            Vec::new(),
            Vec::new(),
        );
        let other = finance_subscription(
            &ctx,
            other_session,
            vec![FinanceEventKind::RssArticle],
            vec!["finance-fed-press-releases".to_owned()],
            Vec::new(),
            Vec::new(),
        );
        let current_id = current.id;
        let other_id = other.id;
        finance_registry.upsert(current).await.unwrap();
        finance_registry.upsert(other).await.unwrap();

        let unsubscribe = FinanceUnsubscribeTool::new(finance_registry.clone());
        let result = unsubscribe
            .run(
                FinanceUnsubscribeParams {
                    catalog_source_ids: vec!["fed-press-releases".to_owned()],
                    ..Default::default()
                },
                &ctx,
            )
            .await
            .unwrap();

        assert_eq!(result.removed_subscription_ids, [current_id]);
        let remaining = finance_registry
            .list_for_owner(&UserId(ctx.user_id.clone()))
            .await;
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, other_id);
    }

    #[tokio::test]
    async fn unsubscribe_dry_run_does_not_remove_matches() {
        let (
            _feed_sources,
            _enable,
            _disable,
            _restart,
            _subscribe,
            _svc,
            _registry,
            finance_registry,
        ) = tool().await;
        let ctx = context();
        let subscription = finance_subscription(
            &ctx,
            ctx.session_key,
            vec![FinanceEventKind::MarketCandleClosed],
            vec!["finance-binance-market-candles".to_owned()],
            vec!["BTCUSDT".to_owned()],
            vec!["1m".to_owned()],
        );
        let subscription_id = subscription.id;
        finance_registry.upsert(subscription).await.unwrap();

        let unsubscribe = FinanceUnsubscribeTool::new(finance_registry.clone());
        let result = unsubscribe
            .run(
                FinanceUnsubscribeParams {
                    source_names: vec!["finance-binance-market-candles".to_owned()],
                    event_kinds: vec![FinanceEventKind::MarketCandleClosed],
                    symbols: vec!["btcusdt".to_owned()],
                    timeframes: vec!["1M".to_owned()],
                    dry_run: Some(true),
                    ..Default::default()
                },
                &ctx,
            )
            .await
            .unwrap();

        assert!(result.dry_run);
        assert_eq!(result.matched_count, 1);
        assert_eq!(result.removed_count, 0);
        assert!(result.removed_subscription_ids.is_empty());
        let remaining = finance_registry
            .list_for_owner(&UserId(ctx.user_id.clone()))
            .await;
        assert_eq!(remaining[0].id, subscription_id);
    }

    #[tokio::test]
    async fn unsubscribe_selector_does_not_remove_wildcard_subscription() {
        let (
            _feed_sources,
            _enable,
            _disable,
            _restart,
            _subscribe,
            _svc,
            _registry,
            finance_registry,
        ) = tool().await;
        let ctx = context();
        let subscription = finance_subscription(
            &ctx,
            ctx.session_key,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );
        let subscription_id = subscription.id;
        finance_registry.upsert(subscription).await.unwrap();

        let unsubscribe = FinanceUnsubscribeTool::new(finance_registry.clone());
        let result = unsubscribe
            .run(
                FinanceUnsubscribeParams {
                    catalog_source_ids: vec!["fed-press-releases".to_owned()],
                    ..Default::default()
                },
                &ctx,
            )
            .await
            .unwrap();

        assert_eq!(result.matched_count, 0);
        assert_eq!(result.removed_count, 0);
        let remaining = finance_registry
            .list_for_owner(&UserId(ctx.user_id.clone()))
            .await;
        assert_eq!(remaining[0].id, subscription_id);
    }

    #[tokio::test]
    async fn list_feed_sources_reports_persisted_event_summary() {
        let pools = rara_kernel::testing::build_memory_diesel_pools().await;
        bootstrap_data_feed_schema(&pools).await;
        let svc = rara_backend_admin::data_feeds::DataFeedSvc::new(pools.clone());
        let store = crate::feed_store::SqliteFeedStore::new(pools);
        let (event_tx, _event_rx) = tokio::sync::mpsc::channel(16);
        let registry = Arc::new(rara_kernel::data_feed::DataFeedRegistry::new(event_tx));
        let finance_path = std::env::temp_dir().join(format!(
            "rara-finance-subscriptions-{}.json",
            uuid::Uuid::new_v4()
        ));
        let finance_registry = Arc::new(FinanceSubscriptionRegistry::load(finance_path));
        let list = FinanceListFeedSourcesTool::new(svc.clone(), registry.clone(), finance_registry);
        let enable = FinanceEnableFeedSourceTool::new(svc.clone(), registry);
        enable
            .run(
                FinanceEnableFeedSourceParams {
                    catalog_source_id: "fed-press-releases".to_owned(),
                    start_now:         Some(false),
                },
                &context(),
            )
            .await
            .unwrap();
        let received_at = jiff::Timestamp::now();
        let event = FeedEvent::builder()
            .id(FeedEventId::deterministic("fed-event"))
            .source_name("finance-fed-press-releases".to_owned())
            .event_type("rss_article".to_owned())
            .tags(vec!["finance".to_owned(), "fed".to_owned()])
            .payload(serde_json::json!({
                "title": "Fed update",
                "url": "https://www.federalreserve.gov/example"
            }))
            .received_at(received_at)
            .build();
        store.append(&event).await.unwrap();

        let result = list
            .run(FinanceListFeedSourcesParams {}, &context())
            .await
            .unwrap();
        let fed = result
            .sources
            .iter()
            .find(|source| source.id == "fed-press-releases")
            .expect("fed source should be listed");

        assert_eq!(fed.runtime.event_count, 1);
        assert_eq!(fed.runtime.last_event_type.as_deref(), Some("rss_article"));
        assert_eq!(fed.runtime.last_event_at, Some(received_at.to_string()));
        assert!(fed.runtime.lag_seconds.is_some_and(|lag| lag >= 0));
    }

    #[tokio::test]
    async fn list_feed_events_returns_recent_events_for_finance_source() {
        let pools = rara_kernel::testing::build_memory_diesel_pools().await;
        bootstrap_data_feed_schema(&pools).await;
        let svc = rara_backend_admin::data_feeds::DataFeedSvc::new(pools.clone());
        let store = crate::feed_store::SqliteFeedStore::new(pools);
        let (event_tx, _event_rx) = tokio::sync::mpsc::channel(16);
        let registry = Arc::new(rara_kernel::data_feed::DataFeedRegistry::new(event_tx));
        let enable = FinanceEnableFeedSourceTool::new(svc.clone(), registry);
        enable
            .run(
                FinanceEnableFeedSourceParams {
                    catalog_source_id: "fed-press-releases".to_owned(),
                    start_now:         Some(false),
                },
                &context(),
            )
            .await
            .unwrap();

        for (title, received_at) in [
            ("Older Fed update", "2026-07-12T08:00:00Z"),
            ("Latest Fed update", "2026-07-12T08:01:00Z"),
        ] {
            let event = FeedEvent::builder()
                .id(FeedEventId::deterministic(title))
                .source_name("finance-fed-press-releases".to_owned())
                .event_type("rss_article".to_owned())
                .tags(vec!["finance".to_owned(), "fed".to_owned()])
                .payload(serde_json::json!({
                    "title": title,
                    "url": "https://www.federalreserve.gov/example"
                }))
                .received_at(received_at.parse().unwrap())
                .build();
            store.append(&event).await.unwrap();
        }

        let tool = FinanceListFeedEventsTool::new(svc);
        let result = tool
            .run(
                FinanceListFeedEventsParams {
                    catalog_source_ids: vec!["fed-press-releases".to_owned()],
                    source_names:       Vec::new(),
                    feed_ids:           Vec::new(),
                    event_kinds:        Vec::new(),
                    since:              None,
                    limit:              Some(1),
                    offset:             None,
                },
                &context(),
            )
            .await
            .unwrap();

        assert_eq!(result.sources.len(), 1);
        let page = &result.sources[0];
        assert_eq!(
            page.catalog_source_id.as_deref(),
            Some("fed-press-releases")
        );
        assert_eq!(page.source_name, "finance-fed-press-releases");
        assert_eq!(page.total, 2);
        assert!(page.has_more);
        assert_eq!(page.events.len(), 1);
        assert_eq!(page.events[0].event_type, "rss_article");
        assert_eq!(page.events[0].payload["title"], "Latest Fed update");
    }

    #[tokio::test]
    async fn list_feed_events_filters_by_event_kind() {
        let pools = rara_kernel::testing::build_memory_diesel_pools().await;
        bootstrap_data_feed_schema(&pools).await;
        let svc = rara_backend_admin::data_feeds::DataFeedSvc::new(pools.clone());
        let store = crate::feed_store::SqliteFeedStore::new(pools);
        let (event_tx, _event_rx) = tokio::sync::mpsc::channel(16);
        let registry = Arc::new(rara_kernel::data_feed::DataFeedRegistry::new(event_tx));
        let enable = FinanceEnableFeedSourceTool::new(svc.clone(), registry);
        enable
            .run(
                FinanceEnableFeedSourceParams {
                    catalog_source_id: "fed-press-releases".to_owned(),
                    start_now:         Some(false),
                },
                &context(),
            )
            .await
            .unwrap();

        for (id, event_type, title, received_at) in [
            (
                "rss-fed-event",
                "rss_article",
                "Fed update",
                "2026-07-12T08:00:00Z",
            ),
            (
                "candle-fed-event",
                "market_candle_closed",
                "BTCUSDT candle",
                "2026-07-12T08:01:00Z",
            ),
        ] {
            let event = FeedEvent::builder()
                .id(FeedEventId::deterministic(id))
                .source_name("finance-fed-press-releases".to_owned())
                .event_type(event_type.to_owned())
                .tags(vec!["finance".to_owned()])
                .payload(serde_json::json!({ "title": title }))
                .received_at(received_at.parse().unwrap())
                .build();
            store.append(&event).await.unwrap();
        }

        let tool = FinanceListFeedEventsTool::new(svc);
        let result = tool
            .run(
                FinanceListFeedEventsParams {
                    catalog_source_ids: vec!["fed-press-releases".to_owned()],
                    source_names:       Vec::new(),
                    feed_ids:           Vec::new(),
                    event_kinds:        vec![FinanceEventKind::MarketCandleClosed],
                    since:              None,
                    limit:              Some(20),
                    offset:             None,
                },
                &context(),
            )
            .await
            .unwrap();

        assert_eq!(result.sources.len(), 1);
        let page = &result.sources[0];
        assert_eq!(page.total, 1);
        assert!(!page.has_more);
        assert_eq!(page.events.len(), 1);
        assert_eq!(page.events[0].event_type, "market_candle_closed");
        assert_eq!(page.events[0].payload["title"], "BTCUSDT candle");
    }

    #[tokio::test]
    async fn list_feed_sources_reports_subscribed_market_instruments() {
        let (list, _enable, _disable, _restart, subscribe, _svc, _registry, _finance_registry) =
            tool().await;
        let ctx = context();
        subscribe
            .run(
                FinanceSubscribeInstrumentsParams {
                    catalog_source_id:      Some("binance-market-candles".to_owned()),
                    feed_id:                None,
                    venue:                  None,
                    symbols:                vec!["SOLUSDT".to_owned(), "btcusdt".to_owned()],
                    timeframes:             vec!["15m".to_owned(), "1m".to_owned()],
                    start_now:              Some(false),
                    delivery:               None,
                    cooldown_secs:          None,
                    max_immediate_per_hour: None,
                },
                &ctx,
            )
            .await
            .unwrap();

        let result = list
            .run(FinanceListFeedSourcesParams {}, &ctx)
            .await
            .unwrap();
        let binance = result
            .sources
            .iter()
            .find(|source| source.id == "binance-market-candles")
            .expect("binance source should be listed");

        assert!(binance.runtime.persisted);
        assert!(binance.runtime.enabled);
        assert_eq!(binance.configured_symbols, ["SOLUSDT", "BTCUSDT"]);
        assert_eq!(binance.configured_timeframes, ["15m", "1m"]);
        assert!(binance.subscriptions.user_subscribed);
        assert!(binance.subscriptions.session_subscribed);
        assert_eq!(binance.subscriptions.user_subscription_ids.len(), 1);
        assert_eq!(
            binance.subscriptions.session_subscription_ids,
            binance.subscriptions.user_subscription_ids
        );
    }

    #[tokio::test]
    async fn list_feed_sources_normalizes_persisted_market_selection() {
        let (list, _enable, _disable, _restart, _subscribe, svc, _registry, _finance_registry) =
            tool().await;
        let now = jiff::Timestamp::now();
        let config = DataFeedConfig::builder()
            .id("existing-binance".to_owned())
            .name("finance-binance-market-candles".to_owned())
            .feed_type(FeedType::MarketCandle)
            .tags(vec!["finance".to_owned(), "market-data".to_owned()])
            .transport(serde_json::json!({
                "provider": "binance",
                "base_url": "https://api.binance.com",
                "interval_secs": 60,
                "headers": {},
                "venue": "binance",
                "symbols": [" btcusdt ", "BTCUSDT", "ethusdt"],
                "timeframes": ["1M", " 5m ", "1m"],
                "max_candles_per_poll": 1000
            }))
            .enabled(true)
            .status(FeedStatus::Idle)
            .created_at(now)
            .updated_at(now)
            .build();
        svc.create_feed(&config).await.unwrap();

        let result = list
            .run(FinanceListFeedSourcesParams {}, &context())
            .await
            .unwrap();
        let binance = result
            .sources
            .iter()
            .find(|source| source.id == "binance-market-candles")
            .expect("binance source should be listed");

        assert_eq!(binance.configured_symbols, ["BTCUSDT", "ETHUSDT"]);
        assert_eq!(binance.configured_timeframes, ["1m", "5m"]);
    }

    #[tokio::test]
    async fn enable_feed_source_persists_and_registers_config_without_starting() {
        let (_list, tool, _disable, _restart, _subscribe, svc, registry, _finance_registry) =
            tool().await;
        let result = tool
            .run(
                FinanceEnableFeedSourceParams {
                    catalog_source_id: "fed-press-releases".to_owned(),
                    start_now:         Some(false),
                },
                &context(),
            )
            .await
            .unwrap();

        assert!(result.created);
        assert!(result.enabled);
        assert!(!result.started);
        assert!(!result.running);
        assert_eq!(result.source_name, "finance-fed-press-releases");
        assert_eq!(result.feed_type, "rss");
        assert!(registry.get("finance-fed-press-releases").is_some());

        let feeds = svc.list_feeds().await.unwrap();
        assert_eq!(feeds.len(), 1);
        assert_eq!(feeds[0].name, "finance-fed-press-releases");
        assert!(feeds[0].enabled);
    }

    #[tokio::test]
    async fn enable_feed_source_is_idempotent_for_existing_enabled_source() {
        let (_list, tool, _disable, _restart, _subscribe, svc, _registry, _finance_registry) =
            tool().await;
        for expected_created in [true, false] {
            let result = tool
                .run(
                    FinanceEnableFeedSourceParams {
                        catalog_source_id: "fed-press-releases".to_owned(),
                        start_now:         Some(false),
                    },
                    &context(),
                )
                .await
                .unwrap();
            assert_eq!(result.created, expected_created);
        }

        assert_eq!(svc.list_feeds().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn enable_feed_source_normalizes_existing_persisted_transport() {
        let (_list, tool, _disable, _restart, _subscribe, svc, _registry, _finance_registry) =
            tool().await;
        let now = jiff::Timestamp::now();
        let existing = DataFeedConfig::builder()
            .id("existing-binance".to_owned())
            .name("finance-binance-market-candles".to_owned())
            .feed_type(FeedType::MarketCandle)
            .tags(vec!["finance".to_owned(), "custom-watchlist".to_owned()])
            .transport(serde_json::json!({
                "provider": " BINANCE ",
                "base_url": "https://api.binance.com",
                "interval_secs": 60,
                "headers": {},
                "venue": " BINANCE ",
                "symbols": [" solusdt ", "SOLUSDT", "btcusdt"],
                "timeframes": [" 5M ", "5m"],
                "max_candles_per_poll": 1000
            }))
            .enabled(false)
            .status(FeedStatus::Error)
            .maybe_last_error(Some("previous failure".to_owned()))
            .created_at(now)
            .updated_at(now)
            .build();
        svc.create_feed(&existing).await.unwrap();

        let result = tool
            .run(
                FinanceEnableFeedSourceParams {
                    catalog_source_id: "binance-market-candles".to_owned(),
                    start_now:         Some(false),
                },
                &context(),
            )
            .await
            .unwrap();

        assert!(!result.created);
        let feed = svc.get_feed("existing-binance").await.unwrap().unwrap();
        assert!(feed.enabled);
        assert_eq!(feed.status, FeedStatus::Idle);
        assert_eq!(feed.last_error, None);
        assert_eq!(feed.transport["provider"], "binance");
        assert_eq!(feed.transport["venue"], "binance");
        assert_eq!(
            feed.transport["symbols"],
            serde_json::json!(["BTCUSDT", "SOLUSDT"])
        );
        assert_eq!(feed.transport["timeframes"], serde_json::json!(["5m"]));
        assert!(feed.tags.iter().any(|tag| tag == "custom-watchlist"));
    }

    #[tokio::test]
    async fn enable_feed_source_rejects_sources_that_require_configuration() {
        let (_list, tool, _disable, _restart, _subscribe, _svc, _registry, _finance_registry) =
            tool().await;
        let err = tool
            .run(
                FinanceEnableFeedSourceParams {
                    catalog_source_id: "longbridge-market-candles".to_owned(),
                    start_now:         Some(false),
                },
                &context(),
            )
            .await
            .unwrap_err();

        assert!(err.to_string().contains("requires configuration"));
    }

    #[tokio::test]
    async fn enable_feed_source_is_mutating() {
        let (_list, tool, _disable, _restart, _subscribe, _svc, _registry, _finance_registry) =
            tool().await;

        assert!(!tool.is_read_only(&serde_json::json!({})));
    }

    #[tokio::test]
    async fn disable_feed_source_turns_off_enabled_catalog_source() {
        let (list, enable, disable, _restart, _subscribe, svc, registry, _finance_registry) =
            tool().await;
        enable
            .run(
                FinanceEnableFeedSourceParams {
                    catalog_source_id: "fed-press-releases".to_owned(),
                    start_now:         Some(false),
                },
                &context(),
            )
            .await
            .unwrap();

        let result = disable
            .run(
                FinanceDisableFeedSourceParams {
                    catalog_source_id: Some("fed-press-releases".to_owned()),
                    feed_id:           None,
                },
                &context(),
            )
            .await
            .unwrap();

        assert!(result.changed);
        assert!(!result.enabled);
        assert!(!result.running);
        assert_eq!(result.source_name, "finance-fed-press-releases");
        assert_eq!(
            result.catalog_source_id.as_deref(),
            Some("fed-press-releases")
        );

        let feeds = svc.list_feeds().await.unwrap();
        assert_eq!(feeds.len(), 1);
        assert!(!feeds[0].enabled);
        assert_eq!(feeds[0].status, FeedStatus::Idle);
        assert!(feeds[0].last_error.is_none());
        assert!(registry.get("finance-fed-press-releases").is_some());
        assert!(!registry.is_running("finance-fed-press-releases"));

        let listed = list
            .run(FinanceListFeedSourcesParams {}, &context())
            .await
            .unwrap();
        let fed = listed
            .sources
            .iter()
            .find(|source| source.id == "fed-press-releases")
            .unwrap();
        assert!(fed.runtime.persisted);
        assert!(!fed.runtime.enabled);
    }

    #[tokio::test]
    async fn restart_feed_source_starts_enabled_finance_feed_by_feed_id() {
        let (_list, _enable, disable, restart, _subscribe, svc, registry, _finance_registry) =
            tool().await;
        let now = jiff::Timestamp::now();
        let config = DataFeedConfig::builder()
            .id("rss-feed".to_owned())
            .name("custom-finance-rss".to_owned())
            .feed_type(FeedType::Rss)
            .tags(vec!["finance".to_owned(), "news".to_owned()])
            .transport(serde_json::json!({
                "url": "https://example.invalid/feed.xml",
                "interval_secs": 3600,
                "headers": {},
                "max_entries_per_poll": 5
            }))
            .enabled(true)
            .status(FeedStatus::Error)
            .maybe_last_error(Some("previous failure".to_owned()))
            .created_at(now)
            .updated_at(now)
            .build();
        svc.create_feed(&config).await.unwrap();

        let result = restart
            .run(
                FinanceRestartFeedSourceParams {
                    catalog_source_id: None,
                    feed_id:           Some("rss-feed".to_owned()),
                },
                &context(),
            )
            .await
            .unwrap();

        assert_eq!(result.feed_id, "rss-feed");
        assert!(!result.was_running);
        assert!(result.started);
        assert!(result.running);
        assert!(registry.is_running("custom-finance-rss"));
        let feeds = svc.list_feeds().await.unwrap();
        assert_eq!(feeds[0].status, FeedStatus::Idle);
        assert_eq!(feeds[0].last_error, None);

        disable
            .run(
                FinanceDisableFeedSourceParams {
                    catalog_source_id: None,
                    feed_id:           Some("rss-feed".to_owned()),
                },
                &context(),
            )
            .await
            .unwrap();
        assert!(!registry.is_running("custom-finance-rss"));
    }

    #[tokio::test]
    async fn restart_feed_source_normalizes_existing_market_candle_transport() {
        let (_list, _enable, disable, restart, _subscribe, svc, registry, _finance_registry) =
            tool().await;
        let now = jiff::Timestamp::now();
        let config = DataFeedConfig::builder()
            .id("existing-binance".to_owned())
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
                "timeframes": [" 1M ", "15m", "1m"],
                "max_candles_per_poll": 1000
            }))
            .enabled(true)
            .status(FeedStatus::Error)
            .maybe_last_error(Some("previous failure".to_owned()))
            .created_at(now)
            .updated_at(now)
            .build();
        svc.create_feed(&config).await.unwrap();

        let result = restart
            .run(
                FinanceRestartFeedSourceParams {
                    catalog_source_id: Some("binance-market-candles".to_owned()),
                    feed_id:           None,
                },
                &context(),
            )
            .await
            .unwrap();

        assert_eq!(result.feed_id, "existing-binance");
        assert!(result.started);
        assert!(result.running);

        let feed = svc.get_feed("existing-binance").await.unwrap().unwrap();
        assert_eq!(feed.status, FeedStatus::Idle);
        assert_eq!(feed.last_error, None);
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

        let registered = registry
            .get("finance-binance-market-candles")
            .expect("restart should register normalized config");
        assert_eq!(registered.transport, feed.transport);

        disable
            .run(
                FinanceDisableFeedSourceParams {
                    catalog_source_id: Some("binance-market-candles".to_owned()),
                    feed_id:           None,
                },
                &context(),
            )
            .await
            .unwrap();
        assert!(!registry.is_running("finance-binance-market-candles"));
    }

    #[tokio::test]
    async fn feed_source_controls_reject_non_finance_feed_id() {
        let (_list, _enable, disable, restart, _subscribe, svc, _registry, _finance_registry) =
            tool().await;
        let now = jiff::Timestamp::now();
        let config = DataFeedConfig::builder()
            .id("plain-rss".to_owned())
            .name("plain-rss".to_owned())
            .feed_type(FeedType::Rss)
            .tags(vec!["news".to_owned()])
            .transport(serde_json::json!({
                "url": "https://example.invalid/feed.xml",
                "interval_secs": 3600,
                "headers": {},
                "max_entries_per_poll": 5
            }))
            .enabled(true)
            .status(FeedStatus::Idle)
            .created_at(now)
            .updated_at(now)
            .build();
        svc.create_feed(&config).await.unwrap();

        let disable_err = disable
            .run(
                FinanceDisableFeedSourceParams {
                    catalog_source_id: None,
                    feed_id:           Some("plain-rss".to_owned()),
                },
                &context(),
            )
            .await
            .unwrap_err();
        assert!(disable_err.to_string().contains("not finance-scoped"));

        let restart_err = restart
            .run(
                FinanceRestartFeedSourceParams {
                    catalog_source_id: None,
                    feed_id:           Some("plain-rss".to_owned()),
                },
                &context(),
            )
            .await
            .unwrap_err();
        assert!(restart_err.to_string().contains("not finance-scoped"));
    }

    #[tokio::test]
    async fn restart_feed_source_rejects_disabled_feed() {
        let (_list, _enable, _disable, restart, _subscribe, svc, _registry, _finance_registry) =
            tool().await;
        let now = jiff::Timestamp::now();
        let config = DataFeedConfig::builder()
            .id("disabled-rss".to_owned())
            .name("disabled-finance-rss".to_owned())
            .feed_type(FeedType::Rss)
            .tags(vec!["finance".to_owned()])
            .transport(serde_json::json!({
                "url": "https://example.invalid/feed.xml",
                "interval_secs": 3600,
                "headers": {},
                "max_entries_per_poll": 5
            }))
            .enabled(false)
            .status(FeedStatus::Idle)
            .created_at(now)
            .updated_at(now)
            .build();
        svc.create_feed(&config).await.unwrap();

        let err = restart
            .run(
                FinanceRestartFeedSourceParams {
                    catalog_source_id: None,
                    feed_id:           Some("disabled-rss".to_owned()),
                },
                &context(),
            )
            .await
            .unwrap_err();

        assert!(err.to_string().contains("is disabled"));
    }

    #[tokio::test]
    async fn subscribe_news_enables_rss_catalog_and_creates_subscription() {
        let (_list, _enable, _disable, _restart, _subscribe, svc, registry, finance_registry) =
            tool().await;
        let tool =
            FinanceSubscribeNewsTool::new(svc.clone(), registry.clone(), finance_registry.clone());
        let ctx = context();

        let result = tool
            .run(
                FinanceSubscribeNewsParams {
                    catalog_source_ids:     vec![
                        "fed-press-releases".to_owned(),
                        "sec-press-releases".to_owned(),
                    ],
                    feed_ids:               Vec::new(),
                    category_tags:          vec!["Monetary Policy".to_owned()],
                    watch_terms:            vec![" BTC ".to_owned(), "NVDA".to_owned()],
                    start_now:              Some(false),
                    delivery:               Some(FinanceDelivery::Immediate),
                    cooldown_secs:          Some(120),
                    max_immediate_per_hour: Some(3),
                },
                &ctx,
            )
            .await
            .unwrap();

        assert!(result.subscription_created);
        assert_eq!(
            result.source_names,
            ["finance-fed-press-releases", "finance-sec-press-releases"]
        );
        assert_eq!(result.category_tags, ["category:monetary-policy"]);
        assert_eq!(result.watch_terms, ["btc", "nvda"]);
        assert_eq!(result.delivery, FinanceDelivery::Immediate);
        assert_eq!(result.cooldown_secs, 120);
        assert_eq!(result.max_immediate_per_hour, 3);
        assert_eq!(result.sources.len(), 2);
        assert!(result.sources.iter().all(|source| {
            source.feed_change == FeedChange::Created && !source.started && !source.running
        }));

        let feeds = svc.list_feeds().await.unwrap();
        assert_eq!(feeds.len(), 2);
        assert!(feeds.iter().all(|feed| feed.enabled));
        assert!(registry.get("finance-fed-press-releases").is_some());
        assert!(registry.get("finance-sec-press-releases").is_some());

        let subs = finance_registry
            .list_for_owner(&rara_kernel::identity::UserId("alice".to_owned()))
            .await;
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0].event_kinds, [FinanceEventKind::RssArticle]);
        assert_eq!(subs[0].source_names, result.source_names);
        assert!(subs[0].venues.is_empty());
        assert!(subs[0].symbols.is_empty());
        assert!(subs[0].timeframes.is_empty());
    }

    #[tokio::test]
    async fn subscribe_news_is_idempotent_for_same_session_and_selectors() {
        let (_list, _enable, _disable, _restart, _subscribe, svc, registry, finance_registry) =
            tool().await;
        let tool = FinanceSubscribeNewsTool::new(svc.clone(), registry, finance_registry.clone());
        let ctx = context();
        let params = |watch_term: &str, category_tag: &str| FinanceSubscribeNewsParams {
            catalog_source_ids:     vec!["fed-press-releases".to_owned()],
            feed_ids:               Vec::new(),
            category_tags:          vec![category_tag.to_owned()],
            watch_terms:            vec![watch_term.to_owned()],
            start_now:              Some(false),
            delivery:               None,
            cooldown_secs:          None,
            max_immediate_per_hour: None,
        };

        let first = tool
            .run(params("Rate Cut", "Monetary Policy"), &ctx)
            .await
            .unwrap();
        let second = tool
            .run(params(" rate   cut ", "category:monetary policy"), &ctx)
            .await
            .unwrap();

        assert!(first.subscription_created);
        assert!(!second.subscription_created);
        assert_eq!(first.subscription_id, second.subscription_id);
        assert_eq!(svc.list_feeds().await.unwrap().len(), 1);
        assert_eq!(
            finance_registry
                .list_for_owner(&rara_kernel::identity::UserId("alice".to_owned()))
                .await
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn subscribe_news_accepts_existing_finance_rss_feed_id() {
        let (_list, _enable, _disable, _restart, _subscribe, svc, registry, finance_registry) =
            tool().await;
        let now = jiff::Timestamp::now();
        let config = DataFeedConfig::builder()
            .id("custom-rss".to_owned())
            .name("custom-finance-rss".to_owned())
            .feed_type(FeedType::Rss)
            .tags(vec!["finance".to_owned(), "news".to_owned()])
            .transport(serde_json::json!({
                "url": "https://example.invalid/feed.xml",
                "interval_secs": 3600,
                "headers": {},
                "max_entries_per_poll": 10
            }))
            .enabled(false)
            .status(FeedStatus::Error)
            .maybe_last_error(Some("previous failure".to_owned()))
            .created_at(now)
            .updated_at(now)
            .build();
        svc.create_feed(&config).await.unwrap();
        let tool = FinanceSubscribeNewsTool::new(svc.clone(), registry.clone(), finance_registry);

        let result = tool
            .run(
                FinanceSubscribeNewsParams {
                    catalog_source_ids:     Vec::new(),
                    feed_ids:               vec!["custom-rss".to_owned()],
                    category_tags:          Vec::new(),
                    watch_terms:            Vec::new(),
                    start_now:              Some(false),
                    delivery:               None,
                    cooldown_secs:          None,
                    max_immediate_per_hour: None,
                },
                &context(),
            )
            .await
            .unwrap();

        assert_eq!(result.source_names, ["custom-finance-rss"]);
        assert_eq!(result.sources[0].feed_change, FeedChange::Updated);
        let feed = svc.get_feed("custom-rss").await.unwrap().unwrap();
        assert!(feed.enabled);
        assert_eq!(feed.status, FeedStatus::Idle);
        assert_eq!(feed.last_error, None);
        assert!(registry.get("custom-finance-rss").is_some());
    }

    #[tokio::test]
    async fn subscribe_news_rejects_market_catalog_source() {
        let (_list, _enable, _disable, _restart, _subscribe, svc, registry, finance_registry) =
            tool().await;
        let tool = FinanceSubscribeNewsTool::new(svc, registry, finance_registry);

        let err = tool
            .run(
                FinanceSubscribeNewsParams {
                    catalog_source_ids:     vec!["binance-market-candles".to_owned()],
                    feed_ids:               Vec::new(),
                    category_tags:          Vec::new(),
                    watch_terms:            Vec::new(),
                    start_now:              Some(false),
                    delivery:               None,
                    cooldown_secs:          None,
                    max_immediate_per_hour: None,
                },
                &context(),
            )
            .await
            .unwrap_err();

        assert!(err.to_string().contains("not an rss source"));
    }

    #[tokio::test]
    async fn subscribe_instruments_creates_market_feed_and_finance_subscription() {
        let (_list, _enable, _disable, _restart, tool, svc, registry, finance_registry) =
            tool().await;
        let list = FinanceListSubscriptionsTool::new(
            svc.clone(),
            registry.clone(),
            finance_registry.clone(),
        );
        let ctx = context();

        let result = tool
            .run(
                FinanceSubscribeInstrumentsParams {
                    catalog_source_id:      Some("binance-market-candles".to_owned()),
                    feed_id:                None,
                    venue:                  None,
                    symbols:                vec!["btcusdt".to_owned(), "SOLUSDT".to_owned()],
                    timeframes:             vec!["1M".to_owned(), "5m".to_owned()],
                    start_now:              Some(false),
                    delivery:               Some(FinanceDelivery::Silent),
                    cooldown_secs:          None,
                    max_immediate_per_hour: None,
                },
                &ctx,
            )
            .await
            .unwrap();

        assert_eq!(result.feed_change, FeedChange::Created);
        assert!(!result.feed_restarted);
        assert!(!result.running);
        assert_eq!(
            result.diagnostic_tool.as_deref(),
            Some("finance_diagnose_candle_subscriptions")
        );
        assert_eq!(
            result.diagnostic_subscription_id,
            Some(result.subscription_id)
        );
        assert_eq!(result.source_name, "finance-binance-market-candles");
        assert_eq!(result.venue, "binance");
        assert_eq!(result.symbols, ["BTCUSDT", "SOLUSDT"]);
        assert_eq!(result.timeframes, ["1m", "5m"]);
        assert!(registry.get("finance-binance-market-candles").is_some());

        let feeds = svc.list_feeds().await.unwrap();
        assert_eq!(feeds.len(), 1);
        assert_eq!(feeds[0].transport["symbols"][0], "BTCUSDT");
        assert_eq!(feeds[0].transport["symbols"][1], "SOLUSDT");
        assert_eq!(feeds[0].transport["timeframes"][0], "1m");
        assert_eq!(feeds[0].transport["timeframes"][1], "5m");

        let subs = finance_registry
            .list_for_owner(&rara_kernel::identity::UserId("alice".to_owned()))
            .await;
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0].id, result.subscription_id);
        assert_eq!(subs[0].event_kinds, [FinanceEventKind::MarketCandleClosed]);
        assert_eq!(subs[0].source_names, ["finance-binance-market-candles"]);
        assert_eq!(subs[0].venues, ["binance"]);
        assert_eq!(subs[0].symbols, ["BTCUSDT", "SOLUSDT"]);
        assert_eq!(subs[0].timeframes, ["1m", "5m"]);

        let listed = list
            .run(
                FinanceListSubscriptionsParams {
                    current_session_only: Some(true),
                },
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(listed.count, 1);
        assert_eq!(
            listed.subscriptions[0].diagnostic_tool.as_deref(),
            Some("finance_diagnose_candle_subscriptions")
        );
        assert_eq!(
            listed.subscriptions[0].diagnostic_subscription_id,
            Some(result.subscription_id)
        );
        let unsubscribe_hint = listed.subscriptions[0]
            .unsubscribe_hint
            .as_ref()
            .expect("market subscription should include unsubscribe hint");
        assert_eq!(unsubscribe_hint.tool, "finance_unsubscribe");
        assert_eq!(
            unsubscribe_hint.default_params,
            serde_json::json!({
                "subscription_ids": [result.subscription_id],
            })
        );
        let events_hint = listed.subscriptions[0]
            .events_hint
            .as_ref()
            .expect("market subscription should include events hint");
        assert_eq!(events_hint.tool, "finance_list_feed_events");
        assert_eq!(
            events_hint.default_params,
            serde_json::json!({
                "source_names": ["finance-binance-market-candles"],
                "event_kinds": ["market_candle_closed"],
                "since": "24h",
                "limit": 20,
            })
        );
        assert_eq!(
            listed.subscriptions[0].sources[0].provider.as_deref(),
            Some("binance")
        );
    }

    #[tokio::test]
    async fn subscribe_instruments_defaults_to_binance_market_feed() {
        let (_list, _enable, _disable, _restart, tool, svc, registry, finance_registry) =
            tool().await;
        let ctx = context();

        let result = tool
            .run(
                FinanceSubscribeInstrumentsParams {
                    catalog_source_id:      None,
                    feed_id:                None,
                    venue:                  None,
                    symbols:                vec!["btcusdt".to_owned()],
                    timeframes:             vec!["1m".to_owned()],
                    start_now:              Some(false),
                    delivery:               None,
                    cooldown_secs:          None,
                    max_immediate_per_hour: None,
                },
                &ctx,
            )
            .await
            .unwrap();

        assert_eq!(result.feed_change, FeedChange::Created);
        assert_eq!(
            result.catalog_source_id.as_deref(),
            Some("binance-market-candles")
        );
        assert_eq!(result.source_name, "finance-binance-market-candles");
        assert_eq!(result.venue, "binance");
        assert_eq!(result.symbols, ["BTCUSDT"]);
        assert_eq!(result.timeframes, ["1m"]);
        assert!(svc.list_feeds().await.unwrap().iter().any(|feed| {
            feed.name == "finance-binance-market-candles"
                && feed.transport["symbols"][0] == "BTCUSDT"
                && feed.transport["timeframes"][0] == "1m"
        }));
        assert!(registry.get("finance-binance-market-candles").is_some());

        let subs = finance_registry
            .list_for_owner(&rara_kernel::identity::UserId("alice".to_owned()))
            .await;
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0].source_names, ["finance-binance-market-candles"]);
    }

    #[tokio::test]
    async fn subscribe_instruments_extends_existing_feed_transport_idempotently() {
        let (_list, _enable, _disable, _restart, tool, svc, _registry, finance_registry) =
            tool().await;
        let ctx = context();

        let first = tool
            .run(
                FinanceSubscribeInstrumentsParams {
                    catalog_source_id:      Some("binance-market-candles".to_owned()),
                    feed_id:                None,
                    venue:                  None,
                    symbols:                vec!["BTCUSDT".to_owned()],
                    timeframes:             vec!["1m".to_owned()],
                    start_now:              Some(false),
                    delivery:               None,
                    cooldown_secs:          None,
                    max_immediate_per_hour: None,
                },
                &ctx,
            )
            .await
            .unwrap();
        let second = tool
            .run(
                FinanceSubscribeInstrumentsParams {
                    catalog_source_id:      Some("binance-market-candles".to_owned()),
                    feed_id:                None,
                    venue:                  None,
                    symbols:                vec!["BTCUSDT".to_owned(), "ETHUSDT".to_owned()],
                    timeframes:             vec!["1m".to_owned(), "15m".to_owned()],
                    start_now:              Some(false),
                    delivery:               None,
                    cooldown_secs:          None,
                    max_immediate_per_hour: None,
                },
                &ctx,
            )
            .await
            .unwrap();

        assert!(first.subscription_created);
        assert!(second.subscription_created);

        let feeds = svc.list_feeds().await.unwrap();
        assert_eq!(feeds.len(), 1);
        assert_eq!(feeds[0].transport["symbols"][0], "BTCUSDT");
        assert_eq!(feeds[0].transport["symbols"][1], "ETHUSDT");
        assert_eq!(feeds[0].transport["timeframes"][0], "1m");
        assert_eq!(feeds[0].transport["timeframes"][1], "15m");

        let repeat = tool
            .run(
                FinanceSubscribeInstrumentsParams {
                    catalog_source_id:      Some("binance-market-candles".to_owned()),
                    feed_id:                None,
                    venue:                  None,
                    symbols:                vec!["ETHUSDT".to_owned(), "BTCUSDT".to_owned()],
                    timeframes:             vec!["15m".to_owned(), "1m".to_owned()],
                    start_now:              Some(false),
                    delivery:               None,
                    cooldown_secs:          None,
                    max_immediate_per_hour: None,
                },
                &ctx,
            )
            .await
            .unwrap();

        assert_eq!(repeat.subscription_id, second.subscription_id);
        assert!(!repeat.subscription_created);
        assert_eq!(
            finance_registry
                .list_for_owner(&rara_kernel::identity::UserId("alice".to_owned()))
                .await
                .len(),
            2
        );
    }

    #[tokio::test]
    async fn subscribe_instruments_preserves_existing_catalog_feed_selection() {
        let (_list, _enable, _disable, _restart, tool, svc, _registry, _finance_registry) =
            tool().await;
        let ctx = context();

        tool.run(
            FinanceSubscribeInstrumentsParams {
                catalog_source_id:      Some("binance-market-candles".to_owned()),
                feed_id:                None,
                venue:                  None,
                symbols:                vec!["SOLUSDT".to_owned()],
                timeframes:             vec!["5m".to_owned()],
                start_now:              Some(false),
                delivery:               None,
                cooldown_secs:          None,
                max_immediate_per_hour: None,
            },
            &ctx,
        )
        .await
        .unwrap();
        tool.run(
            FinanceSubscribeInstrumentsParams {
                catalog_source_id:      Some("binance-market-candles".to_owned()),
                feed_id:                None,
                venue:                  None,
                symbols:                vec!["XRPUSDT".to_owned()],
                timeframes:             vec!["15m".to_owned()],
                start_now:              Some(false),
                delivery:               None,
                cooldown_secs:          None,
                max_immediate_per_hour: None,
            },
            &ctx,
        )
        .await
        .unwrap();

        let feeds = svc.list_feeds().await.unwrap();
        assert_eq!(feeds.len(), 1);
        assert_eq!(
            feeds[0].transport["symbols"],
            serde_json::json!(["SOLUSDT", "XRPUSDT"])
        );
        assert_eq!(
            feeds[0].transport["timeframes"],
            serde_json::json!(["5m", "15m"])
        );
    }

    #[tokio::test]
    async fn subscribe_instruments_start_now_false_preserves_running_feed_task() {
        let (_list, _enable, _disable, _restart, tool, svc, registry, _finance_registry) =
            tool().await;
        let ctx = context();
        let first = tool
            .run(
                FinanceSubscribeInstrumentsParams {
                    catalog_source_id:      Some("binance-market-candles".to_owned()),
                    feed_id:                None,
                    venue:                  None,
                    symbols:                vec!["BTCUSDT".to_owned()],
                    timeframes:             vec!["1m".to_owned()],
                    start_now:              Some(false),
                    delivery:               None,
                    cooldown_secs:          None,
                    max_immediate_per_hour: None,
                },
                &ctx,
            )
            .await
            .unwrap();
        let token = tokio_util::sync::CancellationToken::new();
        let token_clone = token.clone();
        registry.set_running(first.source_name.clone(), token);

        let second = tool
            .run(
                FinanceSubscribeInstrumentsParams {
                    catalog_source_id:      Some("binance-market-candles".to_owned()),
                    feed_id:                None,
                    venue:                  None,
                    symbols:                vec!["ETHUSDT".to_owned()],
                    timeframes:             vec!["15m".to_owned()],
                    start_now:              Some(false),
                    delivery:               None,
                    cooldown_secs:          None,
                    max_immediate_per_hour: None,
                },
                &ctx,
            )
            .await
            .unwrap();

        assert_eq!(second.feed_change, FeedChange::Updated);
        assert!(!second.feed_restarted);
        assert!(!token_clone.is_cancelled());
        assert!(second.running);
        assert!(registry.is_running(&first.source_name));

        let feed = svc.get_feed(&first.feed_id).await.unwrap().unwrap();
        assert_eq!(
            feed.transport["symbols"],
            serde_json::json!(["BTCUSDT", "ETHUSDT"])
        );
        assert_eq!(
            feed.transport["timeframes"],
            serde_json::json!(["1m", "15m"])
        );
        assert_eq!(
            registry.get(&first.source_name).unwrap().transport["symbols"],
            serde_json::json!(["BTCUSDT", "ETHUSDT"])
        );
        assert_eq!(
            registry.get(&first.source_name).unwrap().transport["timeframes"],
            serde_json::json!(["1m", "15m"])
        );
    }

    #[tokio::test]
    async fn subscribe_instruments_rejects_non_market_catalog_source() {
        let (_list, _enable, _disable, _restart, tool, _svc, _registry, _finance_registry) =
            tool().await;
        let err = tool
            .run(
                FinanceSubscribeInstrumentsParams {
                    catalog_source_id:      Some("fed-press-releases".to_owned()),
                    feed_id:                None,
                    venue:                  None,
                    symbols:                vec!["BTCUSDT".to_owned()],
                    timeframes:             vec!["1m".to_owned()],
                    start_now:              Some(false),
                    delivery:               None,
                    cooldown_secs:          None,
                    max_immediate_per_hour: None,
                },
                &context(),
            )
            .await
            .unwrap_err();

        assert!(err.to_string().contains("not a market_candle source"));
    }

    #[tokio::test]
    async fn subscribe_instruments_rejects_ambiguous_source_reference() {
        let (_list, _enable, _disable, _restart, tool, _svc, _registry, _finance_registry) =
            tool().await;
        let err = tool
            .run(
                FinanceSubscribeInstrumentsParams {
                    catalog_source_id:      Some("binance-market-candles".to_owned()),
                    feed_id:                Some("feed-1".to_owned()),
                    venue:                  None,
                    symbols:                vec!["BTCUSDT".to_owned()],
                    timeframes:             vec!["1m".to_owned()],
                    start_now:              Some(false),
                    delivery:               None,
                    cooldown_secs:          None,
                    max_immediate_per_hour: None,
                },
                &context(),
            )
            .await
            .unwrap_err();

        assert!(
            err.to_string()
                .contains("provide either catalog_source_id or feed_id")
        );
    }

    #[tokio::test]
    async fn subscribe_instruments_accepts_existing_feed_id() {
        let (_list, _enable, _disable, _restart, tool, svc, _registry, finance_registry) =
            tool().await;
        let now = jiff::Timestamp::now();
        let config = DataFeedConfig::builder()
            .id("custom-feed".to_owned())
            .name("custom-market-candles".to_owned())
            .feed_type(FeedType::MarketCandle)
            .tags(vec!["finance".to_owned(), "market-data".to_owned()])
            .transport(serde_json::json!({
                "provider": "binance",
                "base_url": "https://api.binance.com",
                "interval_secs": 60,
                "headers": {},
                "venue": "binance",
                "symbols": ["BTCUSDT"],
                "timeframes": ["1m"],
                "max_candles_per_poll": 1000
            }))
            .enabled(false)
            .status(FeedStatus::Idle)
            .created_at(now)
            .updated_at(now)
            .build();
        svc.create_feed(&config).await.unwrap();

        let result = tool
            .run(
                FinanceSubscribeInstrumentsParams {
                    catalog_source_id:      None,
                    feed_id:                Some("custom-feed".to_owned()),
                    venue:                  None,
                    symbols:                vec!["ETHUSDT".to_owned()],
                    timeframes:             vec!["5m".to_owned()],
                    start_now:              Some(false),
                    delivery:               None,
                    cooldown_secs:          None,
                    max_immediate_per_hour: None,
                },
                &context(),
            )
            .await
            .unwrap();

        assert_eq!(result.feed_id, "custom-feed");
        assert_eq!(result.feed_change, FeedChange::Updated);
        let feeds = svc.list_feeds().await.unwrap();
        assert!(feeds[0].enabled);
        assert_eq!(feeds[0].transport["symbols"][0], "BTCUSDT");
        assert_eq!(feeds[0].transport["symbols"][1], "ETHUSDT");

        assert_eq!(
            finance_registry
                .list_for_owner(&rara_kernel::identity::UserId("alice".to_owned()))
                .await
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn subscribe_instruments_rejects_requested_venue_mismatch() {
        let (_list, _enable, _disable, _restart, tool, svc, _registry, finance_registry) =
            tool().await;
        let now = jiff::Timestamp::now();
        let config = DataFeedConfig::builder()
            .id("custom-feed".to_owned())
            .name("custom-market-candles".to_owned())
            .feed_type(FeedType::MarketCandle)
            .tags(vec!["finance".to_owned(), "market-data".to_owned()])
            .transport(serde_json::json!({
                "provider": "binance",
                "base_url": "https://api.binance.com",
                "interval_secs": 60,
                "headers": {},
                "venue": "binance",
                "symbols": ["BTCUSDT"],
                "timeframes": ["1m"],
                "max_candles_per_poll": 1000
            }))
            .enabled(true)
            .status(FeedStatus::Idle)
            .created_at(now)
            .updated_at(now)
            .build();
        svc.create_feed(&config).await.unwrap();

        let err = tool
            .run(
                FinanceSubscribeInstrumentsParams {
                    catalog_source_id:      None,
                    feed_id:                Some("custom-feed".to_owned()),
                    venue:                  Some("okx".to_owned()),
                    symbols:                vec!["BTCUSDT".to_owned()],
                    timeframes:             vec!["1m".to_owned()],
                    start_now:              Some(false),
                    delivery:               None,
                    cooldown_secs:          None,
                    max_immediate_per_hour: None,
                },
                &context(),
            )
            .await
            .unwrap_err();

        assert!(
            err.to_string()
                .contains("requested venue okx does not match market candle feed venue binance")
        );
        assert_eq!(
            finance_registry
                .list_for_owner(&rara_kernel::identity::UserId("alice".to_owned()))
                .await
                .len(),
            0
        );
    }

    #[tokio::test]
    async fn subscribe_instruments_normalizes_existing_feed_id_transport() {
        let (_list, _enable, _disable, _restart, tool, svc, registry, finance_registry) =
            tool().await;
        let now = jiff::Timestamp::now();
        let config = DataFeedConfig::builder()
            .id("custom-feed".to_owned())
            .name("custom-market-candles".to_owned())
            .feed_type(FeedType::MarketCandle)
            .tags(vec!["finance".to_owned(), "market-data".to_owned()])
            .transport(serde_json::json!({
                "provider": " BINANCE ",
                "base_url": "https://api.binance.com",
                "interval_secs": 60,
                "headers": {},
                "venue": " BINANCE ",
                "symbols": [" btcusdt ", "BTCUSDT"],
                "timeframes": [" 1M ", "1m"],
                "max_candles_per_poll": 1000
            }))
            .enabled(true)
            .status(FeedStatus::Idle)
            .created_at(now)
            .updated_at(now)
            .build();
        svc.create_feed(&config).await.unwrap();

        let result = tool
            .run(
                FinanceSubscribeInstrumentsParams {
                    catalog_source_id:      None,
                    feed_id:                Some("custom-feed".to_owned()),
                    venue:                  None,
                    symbols:                vec!["BTCUSDT".to_owned()],
                    timeframes:             vec!["1m".to_owned()],
                    start_now:              Some(false),
                    delivery:               None,
                    cooldown_secs:          None,
                    max_immediate_per_hour: None,
                },
                &context(),
            )
            .await
            .unwrap();

        assert_eq!(result.feed_id, "custom-feed");
        assert_eq!(result.feed_change, FeedChange::Updated);
        assert_eq!(result.venue, "binance");
        assert_eq!(result.symbols, ["BTCUSDT"]);
        assert_eq!(result.timeframes, ["1m"]);

        let feed = svc.get_feed("custom-feed").await.unwrap().unwrap();
        assert_eq!(feed.transport["provider"], "binance");
        assert_eq!(feed.transport["venue"], "binance");
        assert_eq!(feed.transport["symbols"], serde_json::json!(["BTCUSDT"]));
        assert_eq!(feed.transport["timeframes"], serde_json::json!(["1m"]));

        let registered = registry
            .get("custom-market-candles")
            .expect("subscribe should register normalized config");
        assert_eq!(registered.transport, feed.transport);

        let subs = finance_registry
            .list_for_owner(&rara_kernel::identity::UserId("alice".to_owned()))
            .await;
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0].venues, ["binance"]);
    }

    #[tokio::test]
    async fn subscribe_instruments_normalizes_existing_feed_selection() {
        let (list, _enable, _disable, _restart, tool, svc, _registry, _finance_registry) =
            tool().await;
        let now = jiff::Timestamp::now();
        let config = DataFeedConfig::builder()
            .id("existing-binance".to_owned())
            .name("finance-binance-market-candles".to_owned())
            .feed_type(FeedType::MarketCandle)
            .tags(vec!["finance".to_owned(), "market-data".to_owned()])
            .transport(serde_json::json!({
                "provider": "binance",
                "base_url": "https://api.binance.com",
                "interval_secs": 60,
                "headers": {},
                "venue": "binance",
                "symbols": [" btcusdt ", "BTCUSDT"],
                "timeframes": ["1M", " 5m "],
                "max_candles_per_poll": 1000
            }))
            .enabled(false)
            .status(FeedStatus::Idle)
            .created_at(now)
            .updated_at(now)
            .build();
        svc.create_feed(&config).await.unwrap();

        let result = tool
            .run(
                FinanceSubscribeInstrumentsParams {
                    catalog_source_id:      Some("binance-market-candles".to_owned()),
                    feed_id:                None,
                    venue:                  None,
                    symbols:                vec!["ethusdt".to_owned(), " BTCUSDT ".to_owned()],
                    timeframes:             vec!["15M".to_owned(), "1m".to_owned()],
                    start_now:              Some(false),
                    delivery:               None,
                    cooldown_secs:          None,
                    max_immediate_per_hour: None,
                },
                &context(),
            )
            .await
            .unwrap();

        assert_eq!(result.symbols, ["ETHUSDT", "BTCUSDT"]);
        assert_eq!(result.timeframes, ["15m", "1m"]);

        let feed = svc.get_feed("existing-binance").await.unwrap().unwrap();
        assert_eq!(
            feed.transport["symbols"],
            serde_json::json!(["BTCUSDT", "ETHUSDT"])
        );
        assert_eq!(
            feed.transport["timeframes"],
            serde_json::json!(["1m", "5m", "15m"])
        );

        let listed = list
            .run(FinanceListFeedSourcesParams {}, &context())
            .await
            .unwrap();
        let binance = listed
            .sources
            .iter()
            .find(|source| source.id == "binance-market-candles")
            .expect("binance source should be listed");
        assert_eq!(binance.configured_symbols, ["BTCUSDT", "ETHUSDT"]);
        assert_eq!(binance.configured_timeframes, ["1m", "5m", "15m"]);
    }

    #[tokio::test]
    async fn subscribe_instruments_rejects_non_finance_feed_id() {
        let (_list, _enable, _disable, _restart, tool, svc, _registry, _finance_registry) =
            tool().await;
        let now = jiff::Timestamp::now();
        let config = DataFeedConfig::builder()
            .id("plain-market-candles".to_owned())
            .name("plain-market-candles".to_owned())
            .feed_type(FeedType::MarketCandle)
            .tags(vec!["market-data".to_owned()])
            .transport(serde_json::json!({
                "provider": "binance",
                "base_url": "https://api.binance.com",
                "interval_secs": 60,
                "headers": {},
                "venue": "binance",
                "symbols": ["BTCUSDT"],
                "timeframes": ["1m"],
                "max_candles_per_poll": 1000
            }))
            .enabled(true)
            .status(FeedStatus::Idle)
            .created_at(now)
            .updated_at(now)
            .build();
        svc.create_feed(&config).await.unwrap();

        let err = tool
            .run(
                FinanceSubscribeInstrumentsParams {
                    catalog_source_id:      None,
                    feed_id:                Some("plain-market-candles".to_owned()),
                    venue:                  None,
                    symbols:                vec!["ETHUSDT".to_owned()],
                    timeframes:             vec!["5m".to_owned()],
                    start_now:              Some(false),
                    delivery:               None,
                    cooldown_secs:          None,
                    max_immediate_per_hour: None,
                },
                &context(),
            )
            .await
            .unwrap_err();

        assert!(err.to_string().contains("not finance-scoped"));
    }
}
