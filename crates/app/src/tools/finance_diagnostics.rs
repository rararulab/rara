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

use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use jiff::Timestamp;
use rara_backend_admin::data_feeds::DataFeedSvc;
use rara_kernel::{
    data_feed::{DataFeedConfig, DataFeedRegistry, FeedType},
    identity::UserId,
    tool::{ToolContext, ToolExecute},
};
use rara_tool_macro::ToolDef;
use rara_trading::{
    feed::catalog::{DefaultFeedSource, default_finance_feed_sources},
    finance::registry::{FinanceEventKind, FinanceSubscription, FinanceSubscriptionRegistry},
    market_data::{CandleLatestQuery, MarketDataRepositoryRef, Timeframe, tools::FinanceCandle},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const MAX_STALE_AFTER_SECS: u64 = 31_536_000;

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct FinanceDiagnoseCandleSubscriptionsParams {
    /// Optional current-user subscription id filter.
    #[serde(default)]
    pub subscription_id:  Option<Uuid>,
    /// Comparison timestamp. Defaults to server now.
    #[serde(default)]
    pub as_of:            Option<String>,
    /// Stale threshold in seconds. Defaults to 2x each timeframe step.
    #[serde(default)]
    pub stale_after_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct FinanceDiagnoseCandleSubscriptionsResult {
    pub as_of:         String,
    pub subscriptions: Vec<CandleSubscriptionDiagnostic>,
    pub count:         usize,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct CandleSubscriptionDiagnostic {
    pub subscription_id:  Uuid,
    pub source_names:     Vec<String>,
    pub venues:           Vec<String>,
    pub symbols:          Vec<String>,
    pub timeframes:       Vec<String>,
    pub delivery:         String,
    pub status:           SubscriptionHealth,
    pub diagnostic:       Option<String>,
    pub next_action_hint: Option<FinanceDiagnosticNextActionHint>,
    pub feed_sources:     Vec<FeedSourceDiagnostic>,
    pub streams:          Vec<CandleStreamDiagnostic>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct FinanceDiagnosticNextActionHint {
    pub tool:            String,
    pub default_params:  serde_json::Value,
    pub required_params: Vec<String>,
    pub optional_params: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct FeedSourceDiagnostic {
    pub source_name:           String,
    pub catalog_source_id:     Option<String>,
    pub catalog_name:          Option<String>,
    pub feed_id:               Option<String>,
    pub feed_type:             Option<String>,
    pub configured_provider:   Option<String>,
    pub configured_venue:      Option<String>,
    pub configured_symbols:    Vec<String>,
    pub configured_timeframes: Vec<String>,
    pub selector_coverage:     FeedSelectorCoverage,
    pub selector_diagnostic:   Option<String>,
    pub enabled:               Option<bool>,
    pub configured_status:     Option<String>,
    pub runtime_state:         FeedRuntimeState,
    pub last_error:            Option<String>,
    pub event_count:           i64,
    pub last_event_type:       Option<String>,
    pub last_event_at:         Option<String>,
    pub lag_seconds:           Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct CandleStreamDiagnostic {
    pub source_name:      Option<String>,
    pub venue:            String,
    pub symbol:           String,
    pub timeframe:        String,
    pub latest:           Option<FinanceCandle>,
    pub stale_after_secs: Option<u64>,
    pub lag_secs:         Option<i64>,
    pub status:           CandleStreamStatus,
    pub diagnostic:       Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum SubscriptionHealth {
    Ok,
    NeedsData,
    NeedsRuntime,
    SelectorMismatch,
    Unconfigured,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum FeedRuntimeState {
    Running,
    Stopped,
    Disabled,
    NotRegistered,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum FeedSelectorCoverage {
    Covered,
    MissingSelectors,
    Unavailable,
}

#[derive(Debug, Clone)]
struct FeedEventSummary {
    event_count:     i64,
    last_event_type: Option<String>,
    last_event_at:   Option<Timestamp>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum CandleStreamStatus {
    Fresh,
    Stale,
    Missing,
    Future,
    InvalidSelector,
}

#[derive(ToolDef)]
#[tool(
    name = "finance_diagnose_candle_subscriptions",
    description = "Diagnose current-user finance candle subscriptions by combining subscription \
                   selectors, feed config/runtime status, and latest stored candle freshness. Use \
                   this after finance_subscribe_instruments to verify that data is actually \
                   flowing. This is read-only and never places trades.",
    tier = "deferred",
    read_only,
    concurrency_safe
)]
pub(super) struct FinanceDiagnoseCandleSubscriptionsTool {
    data_feed_svc:      DataFeedSvc,
    data_feed_registry: Arc<DataFeedRegistry>,
    finance_registry:   Arc<FinanceSubscriptionRegistry>,
    market_data_repo:   MarketDataRepositoryRef,
}

impl FinanceDiagnoseCandleSubscriptionsTool {
    pub(super) fn new(
        data_feed_svc: DataFeedSvc,
        data_feed_registry: Arc<DataFeedRegistry>,
        finance_registry: Arc<FinanceSubscriptionRegistry>,
        market_data_repo: MarketDataRepositoryRef,
    ) -> Self {
        Self {
            data_feed_svc,
            data_feed_registry,
            finance_registry,
            market_data_repo,
        }
    }
}

#[async_trait]
impl ToolExecute for FinanceDiagnoseCandleSubscriptionsTool {
    type Output = FinanceDiagnoseCandleSubscriptionsResult;
    type Params = FinanceDiagnoseCandleSubscriptionsParams;

    async fn run(
        &self,
        params: FinanceDiagnoseCandleSubscriptionsParams,
        context: &ToolContext,
    ) -> anyhow::Result<FinanceDiagnoseCandleSubscriptionsResult> {
        let as_of = parse_as_of(params.as_of)?;
        let stale_after_secs = params.stale_after_secs;
        validate_stale_after(stale_after_secs)?;
        let owner = UserId(context.user_id.clone());
        let feed_configs = self.data_feed_svc.list_feeds().await?;
        let feeds_by_name = feed_configs
            .into_iter()
            .map(|feed| (feed.name.clone(), feed))
            .collect::<HashMap<_, _>>();
        let event_summaries = self
            .data_feed_svc
            .event_summaries()
            .await?
            .into_iter()
            .map(|summary| {
                (
                    summary.source_name,
                    FeedEventSummary {
                        event_count:     summary.event_count,
                        last_event_type: summary.last_event_type,
                        last_event_at:   summary.last_event_at,
                    },
                )
            })
            .collect::<HashMap<_, _>>();

        let subscriptions = self
            .finance_registry
            .list_for_owner(&owner)
            .await
            .into_iter()
            .filter(is_candle_subscription)
            .filter(|subscription| {
                params
                    .subscription_id
                    .is_none_or(|id| id == subscription.id)
            })
            .collect::<Vec<_>>();

        let mut diagnostics = Vec::with_capacity(subscriptions.len());
        for subscription in subscriptions {
            diagnostics.push(
                self.diagnose_subscription(
                    subscription,
                    &feeds_by_name,
                    &event_summaries,
                    as_of,
                    stale_after_secs,
                )
                .await?,
            );
        }

        let count = diagnostics.len();
        Ok(FinanceDiagnoseCandleSubscriptionsResult {
            as_of: as_of.to_string(),
            subscriptions: diagnostics,
            count,
        })
    }
}

impl FinanceDiagnoseCandleSubscriptionsTool {
    async fn diagnose_subscription(
        &self,
        subscription: FinanceSubscription,
        feeds_by_name: &HashMap<String, DataFeedConfig>,
        event_summaries: &HashMap<String, FeedEventSummary>,
        as_of: Timestamp,
        stale_after_secs: Option<u64>,
    ) -> anyhow::Result<CandleSubscriptionDiagnostic> {
        let feed_sources =
            self.feed_diagnostics(&subscription, feeds_by_name, event_summaries, as_of);
        let streams = self
            .stream_diagnostics(&subscription, as_of, stale_after_secs)
            .await?;
        let status = subscription_health(&subscription, &feed_sources, &streams);
        let diagnostic = subscription_diagnostic(&subscription, status, &feed_sources, &streams);
        let next_action_hint = next_action_hint(&subscription, status, &feed_sources, &streams);

        Ok(CandleSubscriptionDiagnostic {
            subscription_id: subscription.id,
            source_names: subscription.source_names,
            venues: subscription.venues,
            symbols: subscription.symbols,
            timeframes: subscription.timeframes,
            delivery: format!("{:?}", subscription.delivery),
            status,
            diagnostic,
            next_action_hint,
            feed_sources,
            streams,
        })
    }

    fn feed_diagnostics(
        &self,
        subscription: &FinanceSubscription,
        feeds_by_name: &HashMap<String, DataFeedConfig>,
        event_summaries: &HashMap<String, FeedEventSummary>,
        as_of: Timestamp,
    ) -> Vec<FeedSourceDiagnostic> {
        let catalog_by_source_name = catalog_by_source_name();
        subscription
            .source_names
            .iter()
            .map(|source_name| {
                let feed = feeds_by_name.get(source_name);
                let catalog_source = catalog_by_source_name.get(source_name);
                let event_summary = event_summaries.get(source_name);
                let last_event_at = event_summary.and_then(|summary| summary.last_event_at);
                let lag_seconds =
                    last_event_at.map(|timestamp| as_of.duration_since(timestamp).as_secs().max(0));
                let configured_provider =
                    feed.and_then(|feed| transport_string(&feed.transport, "provider"));
                let configured_venue =
                    feed.and_then(|feed| transport_string(&feed.transport, "venue"));
                let configured_symbols = feed.map_or_else(Vec::new, |feed| {
                    transport_string_array(&feed.transport, "symbols", true)
                });
                let configured_timeframes = feed.map_or_else(Vec::new, |feed| {
                    transport_string_array(&feed.transport, "timeframes", false)
                });
                let (selector_coverage, selector_diagnostic) = feed_selector_coverage(
                    feed,
                    configured_venue.as_deref(),
                    &configured_symbols,
                    &configured_timeframes,
                    subscription,
                );
                FeedSourceDiagnostic {
                    source_name: source_name.clone(),
                    catalog_source_id: catalog_source.map(|source| source.id.clone()),
                    catalog_name: catalog_source.map(|source| source.name.clone()),
                    feed_id: feed.map(|feed| feed.id.clone()),
                    feed_type: feed.map(|feed| feed.feed_type.to_string()),
                    configured_provider,
                    configured_venue,
                    configured_symbols,
                    configured_timeframes,
                    selector_coverage,
                    selector_diagnostic,
                    enabled: feed.map(|feed| feed.enabled),
                    configured_status: feed.map(|feed| feed.status.to_string()),
                    runtime_state: runtime_state(
                        feed,
                        self.data_feed_registry.is_running(source_name),
                    ),
                    last_error: feed.and_then(|feed| feed.last_error.clone()),
                    event_count: event_summary.map_or(0, |summary| summary.event_count),
                    last_event_type: event_summary
                        .and_then(|summary| summary.last_event_type.clone()),
                    last_event_at: last_event_at.map(|timestamp| timestamp.to_string()),
                    lag_seconds,
                }
            })
            .collect()
    }

    async fn stream_diagnostics(
        &self,
        subscription: &FinanceSubscription,
        as_of: Timestamp,
        stale_after_secs: Option<u64>,
    ) -> anyhow::Result<Vec<CandleStreamDiagnostic>> {
        let source_names = optional_source_names(&subscription.source_names);
        let mut rows = Vec::new();
        for source_name in source_names {
            for venue in &subscription.venues {
                for symbol in &subscription.symbols {
                    for timeframe in &subscription.timeframes {
                        rows.push(
                            self.stream_diagnostic(
                                source_name.clone(),
                                venue,
                                symbol,
                                timeframe,
                                as_of,
                                stale_after_secs,
                            )
                            .await?,
                        );
                    }
                }
            }
        }
        Ok(rows)
    }

    async fn stream_diagnostic(
        &self,
        source_name: Option<String>,
        venue: &str,
        symbol: &str,
        timeframe: &str,
        as_of: Timestamp,
        stale_after_secs: Option<u64>,
    ) -> anyhow::Result<CandleStreamDiagnostic> {
        let Ok(timeframe_value) = Timeframe::parse(timeframe) else {
            return Ok(invalid_stream(source_name, venue, symbol, timeframe));
        };
        let stale_after_secs = match stale_after_secs {
            Some(value) => value,
            None => u64::try_from(timeframe_value.step()?.as_secs())?.saturating_mul(2),
        };
        let latest = self
            .market_data_repo
            .latest_closed_candle(CandleLatestQuery {
                source_name: source_name.clone(),
                venue:       venue.to_owned(),
                symbol:      symbol.to_owned(),
                timeframe:   timeframe_value,
            })
            .await?;
        Ok(stream_with_latest(
            source_name,
            venue,
            symbol,
            timeframe,
            latest,
            as_of,
            stale_after_secs,
        ))
    }
}

fn is_candle_subscription(subscription: &FinanceSubscription) -> bool {
    subscription.event_kinds.is_empty()
        || subscription
            .event_kinds
            .contains(&FinanceEventKind::MarketCandleClosed)
}

fn parse_as_of(value: Option<String>) -> anyhow::Result<Timestamp> {
    value
        .as_deref()
        .map(|value| {
            value
                .parse()
                .map_err(|err| anyhow::anyhow!("as_of must be an RFC3339 timestamp: {err}"))
        })
        .transpose()
        .map(|value| value.unwrap_or_else(Timestamp::now))
}

fn validate_stale_after(value: Option<u64>) -> anyhow::Result<()> {
    if let Some(value) = value {
        anyhow::ensure!(value > 0, "stale_after_secs must be positive");
        anyhow::ensure!(
            value <= MAX_STALE_AFTER_SECS,
            "stale_after_secs must be <= {MAX_STALE_AFTER_SECS}"
        );
    }
    Ok(())
}

fn runtime_state(feed: Option<&DataFeedConfig>, running: bool) -> FeedRuntimeState {
    match (feed, running) {
        (None, _) => FeedRuntimeState::NotRegistered,
        (Some(feed), _) if !feed.enabled => FeedRuntimeState::Disabled,
        (Some(_), true) => FeedRuntimeState::Running,
        (Some(_), false) => FeedRuntimeState::Stopped,
    }
}

fn optional_source_names(source_names: &[String]) -> Vec<Option<String>> {
    if source_names.is_empty() {
        vec![None]
    } else {
        source_names.iter().cloned().map(Some).collect()
    }
}

fn catalog_by_source_name() -> HashMap<String, DefaultFeedSource> {
    default_finance_feed_sources()
        .into_iter()
        .map(|source| (source.feed_name(), source))
        .collect()
}

fn transport_string(transport: &serde_json::Value, key: &str) -> Option<String> {
    transport
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn transport_string_array(
    transport: &serde_json::Value,
    key: &str,
    uppercase: bool,
) -> Vec<String> {
    transport
        .get(key)
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            if uppercase {
                value.to_ascii_uppercase()
            } else {
                value.to_ascii_lowercase()
            }
        })
        .collect()
}

fn feed_selector_coverage(
    feed: Option<&DataFeedConfig>,
    configured_venue: Option<&str>,
    configured_symbols: &[String],
    configured_timeframes: &[String],
    subscription: &FinanceSubscription,
) -> (FeedSelectorCoverage, Option<String>) {
    let Some(feed) = feed else {
        return (
            FeedSelectorCoverage::Unavailable,
            Some("feed source is not registered".to_owned()),
        );
    };
    if feed.feed_type != FeedType::MarketCandle {
        return (
            FeedSelectorCoverage::Unavailable,
            Some(format!(
                "feed type {} cannot emit market_candle_closed events",
                feed.feed_type
            )),
        );
    }

    let mut diagnostics = Vec::new();
    let subscription_venues = normalize_lowercase(&subscription.venues);
    if !subscription_venues.is_empty() {
        match configured_venue.map(normalize_scalar_lowercase) {
            Some(configured) if subscription_venues.iter().any(|venue| venue == &configured) => {}
            Some(configured) => diagnostics.push(format!(
                "configured venue {configured} does not cover subscription venues: {}",
                subscription_venues.join(", ")
            )),
            None => diagnostics.push(format!(
                "missing venue for subscription venues: {}",
                subscription_venues.join(", ")
            )),
        }
    }

    let missing_symbols = missing_values(
        &normalize_uppercase(&subscription.symbols),
        configured_symbols,
    );
    if !missing_symbols.is_empty() {
        diagnostics.push(format!("missing symbols: {}", missing_symbols.join(", ")));
    }

    let missing_timeframes = missing_values(
        &normalize_lowercase(&subscription.timeframes),
        configured_timeframes,
    );
    if !missing_timeframes.is_empty() {
        diagnostics.push(format!(
            "missing timeframes: {}",
            missing_timeframes.join(", ")
        ));
    }

    if diagnostics.is_empty() {
        (FeedSelectorCoverage::Covered, None)
    } else {
        (
            FeedSelectorCoverage::MissingSelectors,
            Some(diagnostics.join("; ")),
        )
    }
}

fn normalize_uppercase(values: &[String]) -> Vec<String> {
    values
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_uppercase)
        .collect()
}

fn normalize_lowercase(values: &[String]) -> Vec<String> {
    values
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
}

fn normalize_scalar_lowercase(value: &str) -> String { value.trim().to_ascii_lowercase() }

fn missing_values(required: &[String], configured: &[String]) -> Vec<String> {
    required
        .iter()
        .filter(|required| !configured.iter().any(|configured| configured == *required))
        .cloned()
        .collect()
}

fn invalid_stream(
    source_name: Option<String>,
    venue: &str,
    symbol: &str,
    timeframe: &str,
) -> CandleStreamDiagnostic {
    CandleStreamDiagnostic {
        source_name,
        venue: venue.to_owned(),
        symbol: symbol.to_owned(),
        timeframe: timeframe.to_owned(),
        latest: None,
        stale_after_secs: None,
        lag_secs: None,
        status: CandleStreamStatus::InvalidSelector,
        diagnostic: Some("invalid timeframe".to_owned()),
    }
}

fn stream_with_latest(
    source_name: Option<String>,
    venue: &str,
    symbol: &str,
    timeframe: &str,
    latest: Option<rara_trading::market_data::MarketCandle>,
    as_of: Timestamp,
    stale_after_secs: u64,
) -> CandleStreamDiagnostic {
    let Some(candle) = latest else {
        return CandleStreamDiagnostic {
            source_name,
            venue: venue.to_owned(),
            symbol: symbol.to_owned(),
            timeframe: timeframe.to_owned(),
            latest: None,
            stale_after_secs: Some(stale_after_secs),
            lag_secs: None,
            status: CandleStreamStatus::Missing,
            diagnostic: Some("no stored closed candle matched this stream".to_owned()),
        };
    };
    let lag_secs = as_of.as_second() - candle.close_time.as_second();
    let status = if lag_secs < 0 {
        CandleStreamStatus::Future
    } else if lag_secs as u64 > stale_after_secs {
        CandleStreamStatus::Stale
    } else {
        CandleStreamStatus::Fresh
    };
    CandleStreamDiagnostic {
        source_name,
        venue: venue.to_owned(),
        symbol: symbol.to_owned(),
        timeframe: timeframe.to_owned(),
        latest: Some(FinanceCandle::from(candle)),
        stale_after_secs: Some(stale_after_secs),
        lag_secs: Some(lag_secs),
        status,
        diagnostic: None,
    }
}

fn subscription_health(
    subscription: &FinanceSubscription,
    feed_sources: &[FeedSourceDiagnostic],
    streams: &[CandleStreamDiagnostic],
) -> SubscriptionHealth {
    if subscription.venues.is_empty()
        || subscription.symbols.is_empty()
        || subscription.timeframes.is_empty()
    {
        return SubscriptionHealth::Unconfigured;
    }
    if feed_sources.iter().any(|feed| {
        matches!(
            feed.runtime_state,
            FeedRuntimeState::Disabled | FeedRuntimeState::NotRegistered
        ) || feed.selector_coverage == FeedSelectorCoverage::Unavailable
    }) {
        return SubscriptionHealth::Unconfigured;
    }
    if feed_sources
        .iter()
        .any(|feed| feed.selector_coverage == FeedSelectorCoverage::MissingSelectors)
    {
        return SubscriptionHealth::SelectorMismatch;
    }
    if feed_sources
        .iter()
        .any(|feed| feed.runtime_state != FeedRuntimeState::Running)
    {
        return SubscriptionHealth::NeedsRuntime;
    }
    if streams.is_empty()
        || streams
            .iter()
            .any(|stream| stream.status != CandleStreamStatus::Fresh)
    {
        return SubscriptionHealth::NeedsData;
    }
    SubscriptionHealth::Ok
}

fn subscription_diagnostic(
    subscription: &FinanceSubscription,
    status: SubscriptionHealth,
    feed_sources: &[FeedSourceDiagnostic],
    streams: &[CandleStreamDiagnostic],
) -> Option<String> {
    match status {
        SubscriptionHealth::Ok => None,
        SubscriptionHealth::Unconfigured => {
            if subscription.venues.is_empty()
                || subscription.symbols.is_empty()
                || subscription.timeframes.is_empty()
            {
                return Some(
                    "subscription is missing venue, symbol, or timeframe selectors".into(),
                );
            }
            feed_sources
                .iter()
                .find_map(|feed| match feed.runtime_state {
                    FeedRuntimeState::NotRegistered => Some(format!(
                        "feed source {} is not registered",
                        feed.source_name
                    )),
                    FeedRuntimeState::Disabled => {
                        Some(format!("feed source {} is disabled", feed.source_name))
                    }
                    FeedRuntimeState::Running | FeedRuntimeState::Stopped => None,
                })
                .or_else(|| {
                    feed_sources.iter().find_map(|feed| {
                        (feed.selector_coverage == FeedSelectorCoverage::Unavailable).then(|| {
                            feed.selector_diagnostic.clone().unwrap_or_else(|| {
                                format!("feed source {} is unavailable", feed.source_name)
                            })
                        })
                    })
                })
        }
        SubscriptionHealth::SelectorMismatch => feed_sources.iter().find_map(|feed| {
            feed.selector_diagnostic.as_ref().map(|diagnostic| {
                format!(
                    "feed source {} selector mismatch: {diagnostic}",
                    feed.source_name
                )
            })
        }),
        SubscriptionHealth::NeedsRuntime => feed_sources.iter().find_map(|feed| {
            if feed.runtime_state == FeedRuntimeState::Running {
                None
            } else {
                Some(format!(
                    "feed source {} is {}",
                    feed.source_name,
                    feed.runtime_state.as_diagnostic_str()
                ))
            }
        }),
        SubscriptionHealth::NeedsData => streams.iter().find_map(stream_diagnostic_summary),
    }
}

fn next_action_hint(
    subscription: &FinanceSubscription,
    status: SubscriptionHealth,
    feed_sources: &[FeedSourceDiagnostic],
    streams: &[CandleStreamDiagnostic],
) -> Option<FinanceDiagnosticNextActionHint> {
    match status {
        SubscriptionHealth::Ok | SubscriptionHealth::Unconfigured => {
            feed_sources
                .iter()
                .find_map(|feed| match feed.runtime_state {
                    FeedRuntimeState::Disabled | FeedRuntimeState::NotRegistered => {
                        enable_feed_source_hint(feed)
                    }
                    FeedRuntimeState::Running | FeedRuntimeState::Stopped => None,
                })
        }
        SubscriptionHealth::SelectorMismatch => selector_repair_hint(subscription, feed_sources),
        SubscriptionHealth::NeedsRuntime => feed_sources.iter().find_map(|feed| {
            (feed.runtime_state == FeedRuntimeState::Stopped)
                .then(|| restart_feed_source_hint(feed))
                .flatten()
        }),
        SubscriptionHealth::NeedsData => feed_events_hint(feed_sources, streams),
    }
}

fn selector_repair_hint(
    subscription: &FinanceSubscription,
    feed_sources: &[FeedSourceDiagnostic],
) -> Option<FinanceDiagnosticNextActionHint> {
    let feed = feed_sources
        .iter()
        .find(|feed| feed.selector_coverage == FeedSelectorCoverage::MissingSelectors)?;
    let venue = single_value(&subscription.venues)?;
    if let Some(catalog_source_id) = &feed.catalog_source_id {
        return Some(subscribe_instruments_hint(
            "catalog_source_id",
            catalog_source_id,
            "feed_id",
            venue,
            &subscription.symbols,
            &subscription.timeframes,
        ));
    }

    feed.feed_id.as_ref().map(|feed_id| {
        subscribe_instruments_hint(
            "feed_id",
            feed_id,
            "catalog_source_id",
            venue,
            &subscription.symbols,
            &subscription.timeframes,
        )
    })
}

fn single_value(values: &[String]) -> Option<&String> {
    match values {
        [value] => Some(value),
        [] | [_, _, ..] => None,
    }
}

fn subscribe_instruments_hint(
    source_param: &str,
    source_value: &str,
    alternate_source_param: &str,
    venue: &str,
    symbols: &[String],
    timeframes: &[String],
) -> FinanceDiagnosticNextActionHint {
    let mut default_params = serde_json::Map::new();
    default_params.insert(
        source_param.to_owned(),
        serde_json::Value::String(source_value.to_owned()),
    );
    default_params.insert(
        "venue".to_owned(),
        serde_json::Value::String(venue.to_owned()),
    );
    default_params.insert("symbols".to_owned(), serde_json::json!(symbols));
    default_params.insert("timeframes".to_owned(), serde_json::json!(timeframes));
    default_params.insert("start_now".to_owned(), serde_json::Value::Bool(true));

    FinanceDiagnosticNextActionHint {
        tool:            "finance_subscribe_instruments".to_owned(),
        default_params:  serde_json::Value::Object(default_params),
        required_params: Vec::new(),
        optional_params: [
            alternate_source_param,
            "start_now",
            "delivery",
            "cooldown_secs",
            "max_immediate_per_hour",
        ]
        .map(str::to_owned)
        .to_vec(),
    }
}

fn enable_feed_source_hint(feed: &FeedSourceDiagnostic) -> Option<FinanceDiagnosticNextActionHint> {
    let catalog_source_id = feed.catalog_source_id.as_ref()?;
    Some(FinanceDiagnosticNextActionHint {
        tool:            "finance_enable_feed_source".to_owned(),
        default_params:  serde_json::json!({
            "catalog_source_id": catalog_source_id,
            "start_now": true
        }),
        required_params: Vec::new(),
        optional_params: vec!["start_now".to_owned()],
    })
}

fn restart_feed_source_hint(
    feed: &FeedSourceDiagnostic,
) -> Option<FinanceDiagnosticNextActionHint> {
    if let Some(catalog_source_id) = &feed.catalog_source_id {
        return Some(FinanceDiagnosticNextActionHint {
            tool:            "finance_restart_feed_source".to_owned(),
            default_params:  serde_json::json!({
                "catalog_source_id": catalog_source_id
            }),
            required_params: Vec::new(),
            optional_params: vec!["feed_id".to_owned()],
        });
    }

    feed.feed_id
        .as_ref()
        .map(|feed_id| FinanceDiagnosticNextActionHint {
            tool:            "finance_restart_feed_source".to_owned(),
            default_params:  serde_json::json!({
                "feed_id": feed_id
            }),
            required_params: Vec::new(),
            optional_params: vec!["catalog_source_id".to_owned()],
        })
}

fn feed_events_hint(
    feed_sources: &[FeedSourceDiagnostic],
    streams: &[CandleStreamDiagnostic],
) -> Option<FinanceDiagnosticNextActionHint> {
    let source_names = feed_sources
        .iter()
        .filter(|feed| feed.runtime_state == FeedRuntimeState::Running)
        .map(|feed| feed.source_name.clone())
        .collect::<Vec<_>>();
    if source_names.is_empty()
        || streams
            .iter()
            .all(|stream| stream.status == CandleStreamStatus::Fresh)
    {
        return None;
    }

    Some(FinanceDiagnosticNextActionHint {
        tool:            "finance_list_feed_events".to_owned(),
        default_params:  serde_json::json!({
            "source_names": source_names,
            "event_types": ["market_candle_closed"],
            "limit": 20
        }),
        required_params: Vec::new(),
        optional_params: ["after", "before", "offset"].map(str::to_owned).to_vec(),
    })
}

impl FeedRuntimeState {
    fn as_diagnostic_str(self) -> &'static str {
        match self {
            FeedRuntimeState::Running => "running",
            FeedRuntimeState::Stopped => "stopped",
            FeedRuntimeState::Disabled => "disabled",
            FeedRuntimeState::NotRegistered => "not registered",
        }
    }
}

fn stream_diagnostic_summary(stream: &CandleStreamDiagnostic) -> Option<String> {
    if stream.status == CandleStreamStatus::Fresh {
        return None;
    }
    let source = stream
        .source_name
        .as_deref()
        .map_or_else(|| "any source".to_owned(), str::to_owned);
    let reason = stream
        .diagnostic
        .as_deref()
        .unwrap_or_else(|| stream.status.as_diagnostic_str());
    Some(format!(
        "stream {source}/{}/{}/{} is {}: {reason}",
        stream.venue,
        stream.symbol,
        stream.timeframe,
        stream.status.as_diagnostic_str()
    ))
}

impl CandleStreamStatus {
    fn as_diagnostic_str(self) -> &'static str {
        match self {
            CandleStreamStatus::Fresh => "fresh",
            CandleStreamStatus::Stale => "stale",
            CandleStreamStatus::Missing => "missing",
            CandleStreamStatus::Future => "future-dated",
            CandleStreamStatus::InvalidSelector => "invalid",
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use diesel_async::RunQueryDsl;
    use jiff::Timestamp;
    use rara_backend_admin::data_feeds::DataFeedSvc;
    use rara_kernel::{
        data_feed::{
            DataFeedConfig, DataFeedRegistry, FeedEvent, FeedEventId, FeedStatus, FeedStore,
            FeedType,
        },
        identity::UserId,
        io::MessageId,
        queue::{ShardedEventQueue, ShardedEventQueueConfig},
        session::SessionKey,
        tool::{AgentTool, ToolContext, ToolExecute},
    };
    use rara_trading::{
        finance::registry::{
            FinanceDelivery, FinanceEventKind, FinanceSubscription, FinanceSubscriptionRegistry,
        },
        market_data::{
            InMemoryMarketDataRepository, MarketCandle, MarketDataRepository, Timeframe,
        },
    };
    use rust_decimal::Decimal;
    use tokio_util::sync::CancellationToken;

    use super::{
        CandleStreamStatus, FeedRuntimeState, FeedSelectorCoverage,
        FinanceDiagnoseCandleSubscriptionsParams, FinanceDiagnoseCandleSubscriptionsTool,
        SubscriptionHealth,
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

    fn ts(value: &str) -> Timestamp { value.parse().expect("timestamp fixture should parse") }

    fn dec(value: &str) -> Decimal { value.parse().expect("decimal fixture should parse") }

    fn candle(open_time: &str, close_time: &str) -> MarketCandle {
        MarketCandle {
            source_name:       "finance-binance-market-candles".to_owned(),
            venue:             "binance".to_owned(),
            symbol:            "BTCUSDT".to_owned(),
            timeframe:         Timeframe::parse("1m").expect("timeframe fixture should parse"),
            open_time:         ts(open_time),
            close_time:        ts(close_time),
            open:              dec("61500.12"),
            high:              dec("61640.00"),
            low:               dec("61480.50"),
            close:             dec("61610.30"),
            volume:            dec("124.551"),
            ingested_at:       ts(close_time),
            provider_sequence: None,
        }
    }

    fn feed_config() -> DataFeedConfig {
        let now = ts("2026-07-10T08:30:00Z");
        DataFeedConfig::builder()
            .id("feed-1".to_owned())
            .name("finance-binance-market-candles".to_owned())
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
            .status(FeedStatus::Running)
            .created_at(now)
            .updated_at(now)
            .build()
    }

    async fn fixture() -> (
        FinanceDiagnoseCandleSubscriptionsTool,
        Arc<FinanceSubscriptionRegistry>,
        Arc<DataFeedRegistry>,
        Arc<InMemoryMarketDataRepository>,
        crate::feed_store::SqliteFeedStore,
        ToolContext,
    ) {
        let pools = rara_kernel::testing::build_memory_diesel_pools().await;
        bootstrap_data_feed_schema(&pools).await;
        let svc = DataFeedSvc::new(pools.clone());
        let feed_store = crate::feed_store::SqliteFeedStore::new(pools);
        svc.create_feed(&feed_config()).await.unwrap();
        let (event_tx, _event_rx) = tokio::sync::mpsc::channel(16);
        let data_feed_registry = Arc::new(DataFeedRegistry::new(event_tx));
        data_feed_registry.register(feed_config()).unwrap();
        let finance_path = std::env::temp_dir().join(format!(
            "rara-finance-diagnostics-{}.json",
            uuid::Uuid::new_v4()
        ));
        let finance_registry = Arc::new(FinanceSubscriptionRegistry::load(finance_path));
        let market_repo = Arc::new(InMemoryMarketDataRepository::default());
        let tool = FinanceDiagnoseCandleSubscriptionsTool::new(
            svc,
            data_feed_registry.clone(),
            finance_registry.clone(),
            market_repo.clone(),
        );
        (
            tool,
            finance_registry,
            data_feed_registry,
            market_repo,
            feed_store,
            context(),
        )
    }

    async fn insert_subscription(
        registry: &FinanceSubscriptionRegistry,
        ctx: &ToolContext,
    ) -> uuid::Uuid {
        let id = uuid::Uuid::new_v4();
        registry
            .upsert(FinanceSubscription {
                id,
                owner: UserId(ctx.user_id.clone()),
                session_key: ctx.session_key,
                event_kinds: vec![FinanceEventKind::MarketCandleClosed],
                source_names: vec!["finance-binance-market-candles".to_owned()],
                category_tags: Vec::new(),
                watch_terms: Vec::new(),
                venues: vec!["binance".to_owned()],
                symbols: vec!["BTCUSDT".to_owned()],
                timeframes: vec!["1m".to_owned()],
                delivery: FinanceDelivery::Silent,
                cooldown_secs: 900,
                max_immediate_per_hour: 6,
            })
            .await
            .unwrap();
        id
    }

    async fn append_feed_event(store: &crate::feed_store::SqliteFeedStore, received_at: Timestamp) {
        let event = FeedEvent::builder()
            .id(FeedEventId::deterministic(
                "finance-binance-market-candles:event",
            ))
            .source_name("finance-binance-market-candles".to_owned())
            .event_type("market_candle_closed".to_owned())
            .tags(vec!["finance".to_owned(), "market-data".to_owned()])
            .payload(serde_json::json!({
                "venue": "binance",
                "symbol": "BTCUSDT",
                "timeframe": "1m"
            }))
            .received_at(received_at)
            .build();
        store.append(&event).await.unwrap();
    }

    #[tokio::test]
    async fn diagnose_reports_running_fresh_subscription() {
        let (tool, finance_registry, data_feed_registry, market_repo, feed_store, ctx) =
            fixture().await;
        insert_subscription(&finance_registry, &ctx).await;
        data_feed_registry.set_running(
            "finance-binance-market-candles".to_owned(),
            CancellationToken::new(),
        );
        append_feed_event(&feed_store, ts("2026-07-10T08:30:30Z")).await;
        market_repo
            .upsert_closed_candle(candle("2026-07-10T08:29:00Z", "2026-07-10T08:30:00Z"))
            .await
            .unwrap();

        let result = tool
            .run(
                FinanceDiagnoseCandleSubscriptionsParams {
                    subscription_id:  None,
                    as_of:            Some("2026-07-10T08:31:00Z".to_owned()),
                    stale_after_secs: Some(120),
                },
                &ctx,
            )
            .await
            .unwrap();

        assert_eq!(result.count, 1);
        let sub = &result.subscriptions[0];
        assert_eq!(sub.status, SubscriptionHealth::Ok);
        assert_eq!(sub.diagnostic, None);
        assert_eq!(sub.feed_sources[0].runtime_state, FeedRuntimeState::Running);
        assert_eq!(sub.feed_sources[0].event_count, 1);
        assert_eq!(
            sub.feed_sources[0].catalog_source_id.as_deref(),
            Some("binance-market-candles")
        );
        assert_eq!(
            sub.feed_sources[0].catalog_name.as_deref(),
            Some("Binance Market Candles")
        );
        assert_eq!(
            sub.feed_sources[0].last_event_type.as_deref(),
            Some("market_candle_closed")
        );
        assert_eq!(
            sub.feed_sources[0].configured_venue.as_deref(),
            Some("binance")
        );
        assert_eq!(
            sub.feed_sources[0].configured_provider.as_deref(),
            Some("binance")
        );
        assert_eq!(
            sub.feed_sources[0].configured_symbols,
            ["BTCUSDT".to_owned()]
        );
        assert_eq!(sub.feed_sources[0].configured_timeframes, ["1m".to_owned()]);
        assert_eq!(
            sub.feed_sources[0].selector_coverage,
            FeedSelectorCoverage::Covered
        );
        assert_eq!(sub.feed_sources[0].selector_diagnostic, None);
        assert_eq!(
            sub.feed_sources[0].last_event_at.as_deref(),
            Some("2026-07-10T08:30:30Z")
        );
        assert_eq!(sub.feed_sources[0].lag_seconds, Some(30));
        assert_eq!(sub.streams[0].status, CandleStreamStatus::Fresh);
        assert_eq!(sub.streams[0].lag_secs, Some(60));
        assert_eq!(
            sub.streams[0]
                .latest
                .as_ref()
                .map(|candle| candle.close.as_str()),
            Some("61610.30")
        );
    }

    #[tokio::test]
    async fn diagnose_reports_selector_mismatch_when_feed_transport_does_not_cover_subscription() {
        let (tool, finance_registry, data_feed_registry, _market_repo, _feed_store, ctx) =
            fixture().await;
        let id = uuid::Uuid::new_v4();
        finance_registry
            .upsert(FinanceSubscription {
                id,
                owner: UserId(ctx.user_id.clone()),
                session_key: ctx.session_key,
                event_kinds: vec![FinanceEventKind::MarketCandleClosed],
                source_names: vec!["finance-binance-market-candles".to_owned()],
                category_tags: Vec::new(),
                watch_terms: Vec::new(),
                venues: vec!["binance".to_owned()],
                symbols: vec!["ETHUSDT".to_owned()],
                timeframes: vec!["5m".to_owned()],
                delivery: FinanceDelivery::Silent,
                cooldown_secs: 900,
                max_immediate_per_hour: 6,
            })
            .await
            .unwrap();
        data_feed_registry.set_running(
            "finance-binance-market-candles".to_owned(),
            CancellationToken::new(),
        );

        let result = tool
            .run(
                FinanceDiagnoseCandleSubscriptionsParams {
                    subscription_id:  Some(id),
                    as_of:            Some("2026-07-10T08:31:00Z".to_owned()),
                    stale_after_secs: Some(120),
                },
                &ctx,
            )
            .await
            .unwrap();

        let sub = &result.subscriptions[0];
        assert_eq!(sub.status, SubscriptionHealth::SelectorMismatch);
        assert_eq!(
            sub.diagnostic.as_deref(),
            Some(
                "feed source finance-binance-market-candles selector mismatch: missing symbols: \
                 ETHUSDT; missing timeframes: 5m"
            )
        );
        assert_eq!(
            sub.feed_sources[0].selector_coverage,
            FeedSelectorCoverage::MissingSelectors
        );
        assert_eq!(
            sub.feed_sources[0].selector_diagnostic.as_deref(),
            Some("missing symbols: ETHUSDT; missing timeframes: 5m")
        );
        let next_action = sub
            .next_action_hint
            .as_ref()
            .expect("selector mismatch should expose a subscribe-instruments repair hint");
        assert_eq!(next_action.tool, "finance_subscribe_instruments");
        assert_eq!(
            next_action.default_params,
            serde_json::json!({
                "catalog_source_id": "binance-market-candles",
                "venue": "binance",
                "symbols": ["ETHUSDT"],
                "timeframes": ["5m"],
                "start_now": true
            })
        );
        assert_eq!(next_action.required_params, Vec::<String>::new());
        assert_eq!(
            next_action.optional_params,
            [
                "feed_id",
                "start_now",
                "delivery",
                "cooldown_secs",
                "max_immediate_per_hour"
            ]
            .map(str::to_owned)
        );
    }

    #[tokio::test]
    async fn diagnose_reports_unconfigured_when_feed_type_cannot_emit_candles() {
        let (tool, finance_registry, data_feed_registry, _market_repo, _feed_store, ctx) =
            fixture().await;
        insert_subscription(&finance_registry, &ctx).await;
        let mut rss_feed = feed_config();
        rss_feed.feed_type = FeedType::Rss;
        rss_feed.transport = serde_json::json!({
            "url": "https://example.com/feed.xml",
            "interval_secs": 300,
            "headers": {},
            "max_entries_per_poll": 20
        });
        assert!(
            tool.data_feed_svc
                .update_feed(&rss_feed)
                .await
                .expect("rss feed update should succeed")
        );
        data_feed_registry.set_running(
            "finance-binance-market-candles".to_owned(),
            CancellationToken::new(),
        );

        let result = tool
            .run(
                FinanceDiagnoseCandleSubscriptionsParams {
                    subscription_id:  None,
                    as_of:            Some("2026-07-10T08:31:00Z".to_owned()),
                    stale_after_secs: None,
                },
                &ctx,
            )
            .await
            .unwrap();

        let sub = &result.subscriptions[0];
        assert_eq!(sub.status, SubscriptionHealth::Unconfigured);
        assert_eq!(
            sub.diagnostic.as_deref(),
            Some("feed type rss cannot emit market_candle_closed events")
        );
        assert_eq!(sub.feed_sources[0].feed_type.as_deref(), Some("rss"));
        assert_eq!(sub.feed_sources[0].runtime_state, FeedRuntimeState::Running);
        assert_eq!(
            sub.feed_sources[0].selector_coverage,
            FeedSelectorCoverage::Unavailable
        );
        assert_eq!(
            sub.feed_sources[0].selector_diagnostic.as_deref(),
            Some("feed type rss cannot emit market_candle_closed events")
        );
    }

    #[tokio::test]
    async fn diagnose_reports_missing_data_when_feed_is_stopped() {
        let (tool, finance_registry, _data_feed_registry, _market_repo, _feed_store, ctx) =
            fixture().await;
        insert_subscription(&finance_registry, &ctx).await;

        let result = tool
            .run(
                FinanceDiagnoseCandleSubscriptionsParams {
                    subscription_id:  None,
                    as_of:            Some("2026-07-10T08:31:00Z".to_owned()),
                    stale_after_secs: None,
                },
                &ctx,
            )
            .await
            .unwrap();

        let sub = &result.subscriptions[0];
        assert_eq!(sub.status, SubscriptionHealth::NeedsRuntime);
        assert_eq!(
            sub.diagnostic.as_deref(),
            Some("feed source finance-binance-market-candles is stopped")
        );
        let next_action = sub
            .next_action_hint
            .as_ref()
            .expect("stopped feed should expose a restart hint");
        assert_eq!(next_action.tool, "finance_restart_feed_source");
        assert_eq!(
            next_action.default_params,
            serde_json::json!({"catalog_source_id": "binance-market-candles"})
        );
        assert_eq!(next_action.required_params, Vec::<String>::new());
        assert_eq!(next_action.optional_params, ["feed_id".to_owned()]);
        assert_eq!(sub.feed_sources[0].runtime_state, FeedRuntimeState::Stopped);
        assert_eq!(sub.feed_sources[0].event_count, 0);
        assert_eq!(sub.feed_sources[0].last_event_type, None);
        assert_eq!(sub.feed_sources[0].last_event_at, None);
        assert_eq!(sub.feed_sources[0].lag_seconds, None);
        assert_eq!(sub.streams[0].status, CandleStreamStatus::Missing);
        assert!(sub.streams[0].latest.is_none());
    }

    #[tokio::test]
    async fn diagnose_reports_missing_data_when_feed_is_running_without_candles() {
        let (tool, finance_registry, data_feed_registry, _market_repo, _feed_store, ctx) =
            fixture().await;
        insert_subscription(&finance_registry, &ctx).await;
        data_feed_registry.set_running(
            "finance-binance-market-candles".to_owned(),
            CancellationToken::new(),
        );

        let result = tool
            .run(
                FinanceDiagnoseCandleSubscriptionsParams {
                    subscription_id:  None,
                    as_of:            Some("2026-07-10T08:31:00Z".to_owned()),
                    stale_after_secs: None,
                },
                &ctx,
            )
            .await
            .unwrap();

        let sub = &result.subscriptions[0];
        assert_eq!(sub.status, SubscriptionHealth::NeedsData);
        assert_eq!(
            sub.diagnostic.as_deref(),
            Some(
                "stream finance-binance-market-candles/binance/BTCUSDT/1m is missing: no stored \
                 closed candle matched this stream"
            )
        );
        let next_action = sub
            .next_action_hint
            .as_ref()
            .expect("running feed without candles should expose an event-inspection hint");
        assert_eq!(next_action.tool, "finance_list_feed_events");
        assert_eq!(
            next_action.default_params,
            serde_json::json!({
                "source_names": ["finance-binance-market-candles"],
                "event_types": ["market_candle_closed"],
                "limit": 20
            })
        );
        assert_eq!(next_action.required_params, Vec::<String>::new());
        assert_eq!(
            next_action.optional_params,
            ["after", "before", "offset"].map(str::to_owned)
        );
        assert_eq!(sub.feed_sources[0].runtime_state, FeedRuntimeState::Running);
        assert_eq!(sub.streams[0].status, CandleStreamStatus::Missing);
        assert!(sub.streams[0].latest.is_none());
    }

    #[tokio::test]
    async fn diagnose_reports_unconfigured_when_feed_source_is_not_registered() {
        let (tool, finance_registry, _data_feed_registry, _market_repo, _feed_store, ctx) =
            fixture().await;
        let id = uuid::Uuid::new_v4();
        finance_registry
            .upsert(FinanceSubscription {
                id,
                owner: UserId(ctx.user_id.clone()),
                session_key: ctx.session_key,
                event_kinds: vec![FinanceEventKind::MarketCandleClosed],
                source_names: vec!["finance-missing-market-candles".to_owned()],
                category_tags: Vec::new(),
                watch_terms: Vec::new(),
                venues: vec!["binance".to_owned()],
                symbols: vec!["BTCUSDT".to_owned()],
                timeframes: vec!["1m".to_owned()],
                delivery: FinanceDelivery::Silent,
                cooldown_secs: 900,
                max_immediate_per_hour: 6,
            })
            .await
            .unwrap();

        let result = tool
            .run(
                FinanceDiagnoseCandleSubscriptionsParams {
                    subscription_id:  Some(id),
                    as_of:            Some("2026-07-10T08:31:00Z".to_owned()),
                    stale_after_secs: None,
                },
                &ctx,
            )
            .await
            .unwrap();

        let sub = &result.subscriptions[0];
        assert_eq!(sub.status, SubscriptionHealth::Unconfigured);
        assert_eq!(
            sub.diagnostic.as_deref(),
            Some("feed source finance-missing-market-candles is not registered")
        );
        assert_eq!(
            sub.feed_sources[0].runtime_state,
            FeedRuntimeState::NotRegistered
        );
        assert_eq!(
            sub.feed_sources[0].selector_coverage,
            FeedSelectorCoverage::Unavailable
        );
        assert_eq!(
            sub.feed_sources[0].selector_diagnostic.as_deref(),
            Some("feed source is not registered")
        );
        assert_eq!(sub.feed_sources[0].feed_id, None);
        assert_eq!(sub.streams[0].status, CandleStreamStatus::Missing);
    }

    #[tokio::test]
    async fn diagnose_reports_enable_hint_when_builtin_feed_source_is_not_registered() {
        let (tool, finance_registry, _data_feed_registry, _market_repo, _feed_store, ctx) =
            fixture().await;
        insert_subscription(&finance_registry, &ctx).await;
        assert!(
            tool.data_feed_svc
                .delete_feed("feed-1")
                .await
                .expect("delete feed should succeed")
        );

        let result = tool
            .run(
                FinanceDiagnoseCandleSubscriptionsParams {
                    subscription_id:  None,
                    as_of:            Some("2026-07-10T08:31:00Z".to_owned()),
                    stale_after_secs: None,
                },
                &ctx,
            )
            .await
            .unwrap();

        let sub = &result.subscriptions[0];
        assert_eq!(sub.status, SubscriptionHealth::Unconfigured);
        assert_eq!(
            sub.diagnostic.as_deref(),
            Some("feed source finance-binance-market-candles is not registered")
        );
        assert_eq!(
            sub.feed_sources[0].catalog_source_id.as_deref(),
            Some("binance-market-candles")
        );
        assert_eq!(
            sub.feed_sources[0].runtime_state,
            FeedRuntimeState::NotRegistered
        );
        let next_action = sub
            .next_action_hint
            .as_ref()
            .expect("unregistered built-in source should expose an enable hint");
        assert_eq!(next_action.tool, "finance_enable_feed_source");
        assert_eq!(
            next_action.default_params,
            serde_json::json!({
                "catalog_source_id": "binance-market-candles",
                "start_now": true
            })
        );
        assert_eq!(next_action.required_params, Vec::<String>::new());
        assert_eq!(next_action.optional_params, ["start_now".to_owned()]);
    }

    #[tokio::test]
    async fn diagnose_reports_unconfigured_when_feed_source_is_disabled() {
        let (tool, finance_registry, _data_feed_registry, _market_repo, _feed_store, ctx) =
            fixture().await;
        insert_subscription(&finance_registry, &ctx).await;
        let mut disabled = feed_config();
        disabled.enabled = false;
        disabled.status = FeedStatus::Idle;
        disabled.updated_at = ts("2026-07-10T08:31:00Z");
        assert!(
            tool.data_feed_svc
                .update_feed(&disabled)
                .await
                .expect("disabled feed update should succeed")
        );

        let result = tool
            .run(
                FinanceDiagnoseCandleSubscriptionsParams {
                    subscription_id:  None,
                    as_of:            Some("2026-07-10T08:31:00Z".to_owned()),
                    stale_after_secs: None,
                },
                &ctx,
            )
            .await
            .unwrap();

        let sub = &result.subscriptions[0];
        assert_eq!(sub.status, SubscriptionHealth::Unconfigured);
        assert_eq!(
            sub.diagnostic.as_deref(),
            Some("feed source finance-binance-market-candles is disabled")
        );
        assert_eq!(sub.feed_sources[0].enabled, Some(false));
        assert_eq!(
            sub.feed_sources[0].runtime_state,
            FeedRuntimeState::Disabled
        );
        let next_action = sub
            .next_action_hint
            .as_ref()
            .expect("disabled feed should expose an enable hint");
        assert_eq!(next_action.tool, "finance_enable_feed_source");
        assert_eq!(
            next_action.default_params,
            serde_json::json!({
                "catalog_source_id": "binance-market-candles",
                "start_now": true
            })
        );
        assert_eq!(next_action.required_params, Vec::<String>::new());
        assert_eq!(next_action.optional_params, ["start_now".to_owned()]);
    }

    #[tokio::test]
    async fn diagnose_filters_by_subscription_id_for_current_user() {
        let (tool, finance_registry, _data_feed_registry, _market_repo, _feed_store, ctx) =
            fixture().await;
        let id = insert_subscription(&finance_registry, &ctx).await;
        let other_id = uuid::Uuid::new_v4();
        finance_registry
            .upsert(FinanceSubscription {
                id:                     other_id,
                owner:                  UserId("bob".to_owned()),
                session_key:            ctx.session_key,
                event_kinds:            vec![FinanceEventKind::MarketCandleClosed],
                source_names:           vec!["finance-binance-market-candles".to_owned()],
                category_tags:          Vec::new(),
                watch_terms:            Vec::new(),
                venues:                 vec!["binance".to_owned()],
                symbols:                vec!["ETHUSDT".to_owned()],
                timeframes:             vec!["1m".to_owned()],
                delivery:               FinanceDelivery::Silent,
                cooldown_secs:          900,
                max_immediate_per_hour: 6,
            })
            .await
            .unwrap();

        let result = tool
            .run(
                FinanceDiagnoseCandleSubscriptionsParams {
                    subscription_id:  Some(id),
                    as_of:            Some("2026-07-10T08:31:00Z".to_owned()),
                    stale_after_secs: None,
                },
                &ctx,
            )
            .await
            .unwrap();

        assert_eq!(result.count, 1);
        assert_eq!(result.subscriptions[0].subscription_id, id);
    }

    #[tokio::test]
    async fn diagnose_tool_is_read_only() {
        let (tool, _finance_registry, _data_feed_registry, _market_repo, _feed_store, _ctx) =
            fixture().await;

        assert!(tool.is_read_only(&serde_json::json!({})));
    }
}
