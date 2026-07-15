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

use async_trait::async_trait;
use rara_kernel::{
    data_feed::{FeedEvent, FeedStore},
    identity::UserId,
    notification::{NotifyAction, Subscription},
    session::SessionKey,
};
use rust_decimal::Decimal;

use crate::{
    anomaly::{self, AnomalySignal},
    finance::registry::{
        FinanceDeliveryAction, FinanceDeliveryDecision, FinanceSubscriptionRegistry,
    },
    market_data::{CandleRecentQuery, MarketCandle, MarketDataRepository, Timeframe},
};

/// Outcome counters for a single feed-event dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeedDispatchOutcome {
    /// Whether the event was persisted to the feed store.
    pub persisted:         bool,
    /// Number of finance-subscription delivery decisions produced.
    pub finance_decisions: usize,
}

/// Session-injection sink the dispatch facade delivers through.
///
/// The facade owns the pure market-signal orchestration; this trait abstracts
/// the one side effect it cannot own — waking or appending to a live kernel
/// session. The production implementation (wrapping a kernel handle) lives in
/// `crates/app`; tests inject an in-memory double.
#[async_trait]
pub trait FeedDispatchSink: Send + Sync {
    /// Whether `session` is currently attached to a live process.
    async fn session_active(&self, session: &SessionKey) -> bool;
    /// Wake `session` with a synthetic turn carrying `directive`.
    async fn deliver_synthetic(&self, owner: UserId, session: SessionKey, directive: String);
    /// Silently append the feed event `payload` to `session`'s tape.
    async fn append_feed_event(&self, session: SessionKey, payload: serde_json::Value);
    /// Generic (non-finance) subscriptions matching `tags`, across owners.
    async fn generic_matches(&self, _tags: &[String]) -> Vec<Subscription> { Vec::new() }
}

/// Persist, optionally upsert market data, and dispatch matching subscriptions.
///
/// This is the facade entry point: hand it a kernel `FeedEvent` plus the
/// injected feed store, market-data repository, finance registry, and delivery
/// sink, and it runs the whole persist → upsert → evaluate → match → deliver
/// pipeline, returning the [`FeedDispatchOutcome`] counters.
pub async fn on_feed_event(
    event: &FeedEvent,
    feed_store: &dyn FeedStore,
    market_repo: &dyn MarketDataRepository,
    finance_registry: &FinanceSubscriptionRegistry,
    sink: &dyn FeedDispatchSink,
) -> anyhow::Result<FeedDispatchOutcome> {
    feed_store.append(event).await?;

    // Keep the parsed candle so the anomaly evaluator can pull its rolling
    // window after the upsert (the newest bar is then already durable).
    let closed_candle = if event.event_type == "market_candle_closed" {
        let candle = market_candle_from_event(event)?;
        market_repo.upsert_closed_candle(candle.clone()).await?;
        Some(candle)
    } else {
        None
    };

    let anomaly = match &closed_candle {
        Some(candle) => evaluate_anomaly(market_repo, candle).await,
        None => None,
    };

    let event_json = serde_json::to_value(event).unwrap_or_default();
    // Feed the evaluated severity into the delivery decision so an alert-grade
    // anomaly bypasses the routine cooldown / hourly-budget throttle. The
    // registry stays the single home of that policy; the facade only supplies
    // the severity it already computed above.
    let decisions = finance_registry
        .match_event(event, anomaly.as_ref().map(|signal| signal.severity))
        .await;
    for decision in &decisions {
        dispatch_finance_decision(event, &event_json, decision, anomaly.as_ref(), sink).await;
    }

    dispatch_generic_subscriptions(event, &event_json, sink).await;

    Ok(FeedDispatchOutcome {
        persisted:         true,
        finance_decisions: decisions.len(),
    })
}

async fn dispatch_finance_decision(
    event: &FeedEvent,
    event_json: &serde_json::Value,
    decision: &FinanceDeliveryDecision,
    anomaly: Option<&AnomalySignal>,
    sink: &dyn FeedDispatchSink,
) {
    match decision.action {
        FinanceDeliveryAction::Immediate => {
            if !sink.session_active(&decision.session_key).await {
                sink.append_feed_event(decision.session_key, event_json.clone())
                    .await;
                return;
            }
            sink.deliver_synthetic(
                decision.owner.clone(),
                decision.session_key,
                finance_directive(event, anomaly),
            )
            .await;
        }
        FinanceDeliveryAction::Silent => {
            sink.append_feed_event(decision.session_key, event_json.clone())
                .await;
        }
    }
}

async fn dispatch_generic_subscriptions(
    event: &FeedEvent,
    event_json: &serde_json::Value,
    sink: &dyn FeedDispatchSink,
) {
    for sub in sink.generic_matches(&event.tags).await {
        match sub.on_receive {
            NotifyAction::ProactiveTurn => {
                if !sink.session_active(&sub.subscriber).await {
                    sink.append_feed_event(sub.subscriber, event_json.clone())
                        .await;
                    continue;
                }
                let payload_pretty =
                    serde_json::to_string_pretty(&event.payload).unwrap_or_default();
                let directive = format!(
                    "[FeedEvent] source={} type={} tags={:?}\n{}",
                    event.source_name, event.event_type, event.tags, payload_pretty
                );
                sink.deliver_synthetic(sub.owner, sub.subscriber, directive)
                    .await;
            }
            NotifyAction::SilentAppend => {
                sink.append_feed_event(sub.subscriber, event_json.clone())
                    .await;
            }
        }
    }
}

/// Pull the rolling window for `candle` and run the anomaly evaluator.
///
/// This is an application-boundary adapter: a repository read or an evaluator
/// error must never block delivery, so both degrade to `None` (the factual,
/// unenriched directive) after logging a concrete reason.
#[tracing::instrument(skip_all)]
async fn evaluate_anomaly(
    market_repo: &dyn MarketDataRepository,
    candle: &MarketCandle,
) -> Option<AnomalySignal> {
    let window = match market_repo
        .recent_candles(CandleRecentQuery {
            source_name: Some(candle.source_name.clone()),
            venue:       candle.venue.clone(),
            symbol:      candle.symbol.clone(),
            timeframe:   candle.timeframe.clone(),
            limit:       anomaly::EVAL_WINDOW,
            // Strictly earlier bars: the window is the history before this bar.
            end:         Some(candle.open_time),
        })
        .await
    {
        Ok(rows) => rows,
        Err(err) => {
            tracing::warn!(%err, "recent-candles query failed; skipping anomaly evaluation");
            return None;
        }
    };

    match anomaly::evaluate(&window, candle) {
        Ok(signal) => signal,
        Err(err) => {
            tracing::warn!(%err, "anomaly evaluation failed; delivering factual directive");
            None
        }
    }
}

fn finance_directive(event: &FeedEvent, anomaly: Option<&AnomalySignal>) -> String {
    match event.event_type.as_str() {
        "market_candle_closed" => {
            let header = format!(
                "[FinanceMarketUpdate] source={} venue={} symbol={} timeframe={} close_time={} \
                 close={}",
                event.source_name,
                event.payload["venue"].as_str().unwrap_or_default(),
                event.payload["symbol"].as_str().unwrap_or_default(),
                event.payload["timeframe"].as_str().unwrap_or_default(),
                event.payload["close_time"].as_str().unwrap_or_default(),
                event.payload["close"].as_str().unwrap_or_default(),
            );
            match anomaly {
                Some(signal) => format!(
                    "{header}\n[Anomaly severity={} reason={}]\nDescribe what happened, the \
                     related market context, and a suggested next action.",
                    signal.severity.label(),
                    signal.reason,
                ),
                None => format!(
                    "{header}\nReport the market update factually; do not infer a trade unless \
                     the user asks.",
                ),
            }
        }
        _ => format!(
            "[FinanceArticle] source={} title={} url={} published_at={}\nSummarize factual \
             relevance; do not give trade advice unless the user asks.",
            event.source_name,
            event.payload["title"].as_str().unwrap_or_default(),
            event.payload["url"].as_str().unwrap_or_default(),
            event.payload["published_at"].as_str().unwrap_or_default(),
        ),
    }
}

fn market_candle_from_event(event: &FeedEvent) -> anyhow::Result<MarketCandle> {
    Ok(MarketCandle {
        source_name:       event.source_name.clone(),
        venue:             normalize_venue(required_str(event, "venue")?),
        symbol:            normalize_symbol(required_str(event, "symbol")?),
        timeframe:         normalize_timeframe(required_str(event, "timeframe")?)?,
        open_time:         required_str(event, "open_time")?.parse()?,
        close_time:        required_str(event, "close_time")?.parse()?,
        open:              Decimal::from_str_exact(required_str(event, "open")?)?,
        high:              Decimal::from_str_exact(required_str(event, "high")?)?,
        low:               Decimal::from_str_exact(required_str(event, "low")?)?,
        close:             Decimal::from_str_exact(required_str(event, "close")?)?,
        volume:            Decimal::from_str_exact(required_str(event, "volume")?)?,
        ingested_at:       event.received_at,
        provider_sequence: event.payload["provider_sequence"]
            .as_str()
            .map(ToOwned::to_owned),
    })
}

fn required_str<'a>(event: &'a FeedEvent, field: &str) -> anyhow::Result<&'a str> {
    event.payload[field]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("market candle event missing string field '{field}'"))
}

fn normalize_venue(value: &str) -> String { value.trim().to_ascii_lowercase() }

fn normalize_symbol(value: &str) -> String { value.trim().to_ascii_uppercase() }

fn normalize_timeframe(value: &str) -> anyhow::Result<Timeframe> {
    Timeframe::parse(value.trim().to_ascii_lowercase())
}
