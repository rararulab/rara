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
    data_feed::{DataFeedConfig, DataFeedRegistry, FeedStatus, FeedType},
    identity::UserId,
    tool::{ToolContext, ToolExecute},
};
use rara_tool_macro::ToolDef;
use rara_trading::{
    feed::catalog::{DefaultFeedSource, default_finance_feed_sources},
    finance::registry::{
        FinanceDelivery, FinanceEventKind, FinanceSubscription, FinanceSubscriptionRegistry,
    },
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const MAX_CATALOG_SOURCE_ID_LEN: usize = 128;
const MAX_FEED_ID_LEN: usize = 128;
const MAX_SYMBOLS: usize = 500;
const MAX_TIMEFRAMES: usize = 32;
const MAX_INSTRUMENT_SELECTOR_LEN: usize = 64;
const DEFAULT_COOLDOWN_SECS: u64 = 900;
const DEFAULT_MAX_IMMEDIATE_PER_HOUR: u16 = 6;

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct FinanceListFeedSourcesParams {}

#[derive(Debug, Clone, Serialize)]
pub(super) struct FinanceFeedSourceEntry {
    pub id:                     String,
    pub name:                   String,
    pub description:            String,
    pub feed_type:              String,
    pub tags:                   Vec<String>,
    pub source_name:            String,
    pub requires_configuration: bool,
    pub can_enable:             bool,
    pub setup_hint:             Option<String>,
    pub runtime:                FinanceFeedSourceRuntime,
    pub venue:                  Option<String>,
    pub configured_symbols:     Vec<String>,
    pub configured_timeframes:  Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct FinanceFeedSourceRuntime {
    pub persisted:     bool,
    pub feed_id:       Option<String>,
    pub enabled:       bool,
    pub running:       bool,
    pub status:        Option<String>,
    pub last_error:    Option<String>,
    pub event_count:   i64,
    pub last_event_at: Option<String>,
    pub lag_seconds:   Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct FinanceListFeedSourcesResult {
    pub sources: Vec<FinanceFeedSourceEntry>,
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
pub(super) struct FinanceSubscribeInstrumentsParams {
    /// Built-in market-candle source id from `finance_list_feed_sources`.
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
    pub subscription_id:      Uuid,
    pub subscription_created: bool,
    pub feed_id:              String,
    pub source_name:          String,
    pub catalog_source_id:    Option<String>,
    pub venue:                String,
    pub symbols:              Vec<String>,
    pub timeframes:           Vec<String>,
    pub feed_change:          FeedChange,
    pub feed_restarted:       bool,
    pub running:              bool,
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
    svc:      DataFeedSvc,
    registry: Arc<DataFeedRegistry>,
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
    pub(super) fn new(svc: DataFeedSvc, registry: Arc<DataFeedRegistry>) -> Self {
        Self { svc, registry }
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
        _context: &ToolContext,
    ) -> anyhow::Result<FinanceListFeedSourcesResult> {
        let feeds = self.svc.list_feeds().await?;
        let summaries = self
            .svc
            .event_summaries()
            .await?
            .into_iter()
            .map(|summary| (summary.source_name.clone(), summary))
            .collect::<HashMap<_, _>>();
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
                        last_event_at.map(|timestamp| timestamp.to_string()),
                        lag_seconds,
                    )
                })
                .collect(),
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

        let source_ref = SourceRef::from_params(params.catalog_source_id, params.feed_id)?;
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

        let requested_venue = params.venue.map(normalize_venue).transpose()?;
        let venue = requested_venue.unwrap_or_else(|| {
            config
                .transport
                .get("venue")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned()
        });
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
        } else if transport_changed || !was_enabled {
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
}

struct FeedResolution {
    config: DataFeedConfig,
    catalog_source_id: Option<String>,
    created: bool,
    replace_existing_instruments: bool,
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
        let value = value.trim();
        anyhow::ensure!(!value.is_empty(), "{name} contains an empty value");
        anyhow::ensure!(
            value.chars().count() <= MAX_INSTRUMENT_SELECTOR_LEN,
            "{name} value is too long"
        );
        let normalized = if uppercase {
            value.to_ascii_uppercase()
        } else {
            value.to_ascii_lowercase()
        };
        if !out.contains(&normalized) {
            out.push(normalized);
        }
    }
    Ok(out)
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
            merge_json_string_array(object.get("symbols"), symbols, replace_existing)?
                .into_iter()
                .map(serde_json::Value::String)
                .collect(),
        ),
    );
    object.insert(
        "timeframes".to_owned(),
        serde_json::Value::Array(
            merge_json_string_array(object.get("timeframes"), timeframes, replace_existing)?
                .into_iter()
                .map(serde_json::Value::String)
                .collect(),
        ),
    );
    Ok(*transport != before)
}

fn merge_json_string_array(
    current: Option<&serde_json::Value>,
    requested: &[String],
    replace_existing: bool,
) -> anyhow::Result<Vec<String>> {
    if replace_existing {
        return Ok(requested.to_vec());
    }
    let mut out = match current {
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
    for value in requested {
        if !out.contains(value) {
            out.push(value.clone());
        }
    }
    Ok(out)
}

fn set_eq<T>(left: &[T], right: &[T]) -> bool
where
    T: Eq + std::hash::Hash,
{
    left.iter().collect::<HashSet<_>>() == right.iter().collect::<HashSet<_>>()
}

fn find_catalog_source(catalog_source_id: &str) -> anyhow::Result<DefaultFeedSource> {
    default_finance_feed_sources()
        .into_iter()
        .find(|source| source.id == catalog_source_id)
        .ok_or_else(|| {
            anyhow::anyhow!("unknown finance feed catalog source id: {catalog_source_id}")
        })
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

fn feed_source_entry(
    source: DefaultFeedSource,
    persisted: Option<&DataFeedConfig>,
    running: bool,
    event_count: i64,
    last_event_at: Option<String>,
    lag_seconds: Option<i64>,
) -> FinanceFeedSourceEntry {
    let source_name = source.feed_name();
    let transport = persisted
        .map(|feed| &feed.transport)
        .or(source.transport.as_ref());
    let can_enable = source.can_enable();
    FinanceFeedSourceEntry {
        id: source.id,
        name: source.name,
        description: source.description,
        feed_type: source.feed_type.to_string(),
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
            last_event_at,
            lag_seconds,
        },
        venue: transport
            .and_then(|value| value.get("venue"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        configured_symbols: transport
            .map_or_else(Vec::new, |value| extract_string_array(value, "symbols")),
        configured_timeframes: transport
            .map_or_else(Vec::new, |value| extract_string_array(value, "timeframes")),
    }
}

fn extract_string_array(value: &serde_json::Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn config_from_source(
    source: &DefaultFeedSource,
    existing: Option<DataFeedConfig>,
) -> anyhow::Result<DataFeedConfig> {
    let now = Timestamp::now();
    let transport = source
        .transport
        .clone()
        .ok_or_else(|| anyhow::anyhow!("finance feed source {} has no transport", source.id))?;
    let id = existing
        .as_ref()
        .map(|feed| feed.id.clone())
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let created_at = existing.as_ref().map(|feed| feed.created_at).unwrap_or(now);

    Ok(DataFeedConfig::builder()
        .id(id)
        .name(source.feed_name())
        .feed_type(source.feed_type)
        .tags(source.tags.clone())
        .transport(transport)
        .maybe_auth(source.auth.clone())
        .enabled(true)
        .status(FeedStatus::Idle)
        .maybe_last_error(None)
        .created_at(created_at)
        .updated_at(now)
        .build())
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
        io::MessageId,
        queue::{ShardedEventQueue, ShardedEventQueueConfig},
        session::SessionKey,
        tool::{AgentTool, ToolContext, ToolExecute},
    };
    use rara_trading::finance::registry::{
        FinanceDelivery, FinanceEventKind, FinanceSubscriptionRegistry,
    };

    use super::{
        FeedChange, FinanceDisableFeedSourceParams, FinanceDisableFeedSourceTool,
        FinanceEnableFeedSourceParams, FinanceEnableFeedSourceTool, FinanceListFeedSourcesParams,
        FinanceListFeedSourcesTool, FinanceRestartFeedSourceParams, FinanceRestartFeedSourceTool,
        FinanceSubscribeInstrumentsParams, FinanceSubscribeInstrumentsTool,
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
            FinanceListFeedSourcesTool::new(svc.clone(), registry.clone()),
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
        assert!(fed.can_enable);
        assert!(!fed.runtime.persisted);
        assert_eq!(fed.runtime.feed_id, None);
        assert!(!fed.runtime.enabled);
        assert!(!fed.runtime.running);
        assert_eq!(fed.runtime.status, None);
        assert_eq!(fed.runtime.event_count, 0);
        assert_eq!(fed.runtime.last_event_at, None);
        assert_eq!(fed.runtime.lag_seconds, None);

        let binance = result
            .sources
            .iter()
            .find(|source| source.id == "binance-market-candles")
            .expect("binance source should be listed");
        assert_eq!(binance.venue.as_deref(), Some("binance"));
        assert_eq!(binance.configured_symbols, ["BTCUSDT", "ETHUSDT"]);
        assert_eq!(binance.configured_timeframes, ["1m"]);
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
    async fn list_feed_sources_reports_persisted_event_summary() {
        let pools = rara_kernel::testing::build_memory_diesel_pools().await;
        bootstrap_data_feed_schema(&pools).await;
        let svc = rara_backend_admin::data_feeds::DataFeedSvc::new(pools.clone());
        let store = crate::feed_store::SqliteFeedStore::new(pools);
        let (event_tx, _event_rx) = tokio::sync::mpsc::channel(16);
        let registry = Arc::new(rara_kernel::data_feed::DataFeedRegistry::new(event_tx));
        let list = FinanceListFeedSourcesTool::new(svc.clone(), registry.clone());
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
        assert_eq!(fed.runtime.last_event_at, Some(received_at.to_string()));
        assert!(fed.runtime.lag_seconds.is_some_and(|lag| lag >= 0));
    }

    #[tokio::test]
    async fn list_feed_sources_reports_subscribed_market_instruments() {
        let (list, _enable, _disable, _restart, subscribe, _svc, _registry, _finance_registry) =
            tool().await;
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
                &context(),
            )
            .await
            .unwrap();

        let result = list
            .run(FinanceListFeedSourcesParams {}, &context())
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
    async fn subscribe_instruments_creates_market_feed_and_finance_subscription() {
        let (_list, _enable, _disable, _restart, tool, svc, registry, finance_registry) =
            tool().await;
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
