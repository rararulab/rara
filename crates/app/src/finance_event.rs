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
use rara_trading::{
    finance::registry::{
        FinanceDeliveryAction, FinanceDeliveryDecision, FinanceSubscriptionRegistry,
    },
    market_data::{MarketCandle, MarketDataRepository, Timeframe},
};
use rust_decimal::Decimal;

/// Outcome counters for feed dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeedDispatchOutcome {
    pub persisted:         bool,
    pub finance_decisions: usize,
}

#[async_trait]
pub trait FeedDispatchSink: Send + Sync {
    async fn session_active(&self, session: &SessionKey) -> bool;
    async fn deliver_synthetic(&self, owner: UserId, session: SessionKey, directive: String);
    async fn append_feed_event(&self, session: SessionKey, payload: serde_json::Value);
    async fn generic_matches(&self, _tags: &[String]) -> Vec<Subscription> { Vec::new() }
}

/// Production sink backed by a kernel handle.
pub struct KernelFeedDispatchSink {
    handle: rara_kernel::handle::KernelHandle,
}

impl KernelFeedDispatchSink {
    #[must_use]
    pub fn new(handle: rara_kernel::handle::KernelHandle) -> Self { Self { handle } }
}

#[async_trait]
impl FeedDispatchSink for KernelFeedDispatchSink {
    async fn session_active(&self, session: &SessionKey) -> bool {
        self.handle.process_table().contains(session)
    }

    async fn deliver_synthetic(&self, owner: UserId, session: SessionKey, directive: String) {
        let msg = rara_kernel::io::InboundMessage::synthetic(directive, owner, session);
        self.handle.deliver_internal(msg).await;
    }

    async fn append_feed_event(&self, session: SessionKey, payload: serde_json::Value) {
        let _ = self
            .handle
            .tape()
            .store()
            .append(
                &session.to_string(),
                rara_kernel::memory::TapEntryKind::FeedEvent,
                payload,
                None,
            )
            .await;
    }

    async fn generic_matches(&self, tags: &[String]) -> Vec<Subscription> {
        self.handle
            .subscription_registry()
            .match_tags_any_owner(tags)
            .await
    }
}

/// Persist, optionally upsert market data, and dispatch matching subscriptions.
pub async fn dispatch_feed_event(
    event: &FeedEvent,
    feed_store: &dyn FeedStore,
    market_repo: &dyn MarketDataRepository,
    finance_registry: &FinanceSubscriptionRegistry,
    sink: &dyn FeedDispatchSink,
) -> anyhow::Result<FeedDispatchOutcome> {
    feed_store.append(event).await?;

    if event.event_type == "market_candle_closed" {
        let candle = market_candle_from_event(event)?;
        market_repo.upsert_closed_candle(candle).await?;
    }

    let event_json = serde_json::to_value(event).unwrap_or_default();
    let decisions = finance_registry.match_event(event).await;
    for decision in &decisions {
        dispatch_finance_decision(event, &event_json, decision, sink).await;
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
                finance_directive(event),
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

fn finance_directive(event: &FeedEvent) -> String {
    match event.event_type.as_str() {
        "market_candle_closed" => format!(
            "[FinanceMarketUpdate] source={} venue={} symbol={} timeframe={} close_time={} \
             close={}\nReport the market update factually; do not infer a trade unless the user \
             asks.",
            event.source_name,
            event.payload["venue"].as_str().unwrap_or_default(),
            event.payload["symbol"].as_str().unwrap_or_default(),
            event.payload["timeframe"].as_str().unwrap_or_default(),
            event.payload["close_time"].as_str().unwrap_or_default(),
            event.payload["close"].as_str().unwrap_or_default(),
        ),
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
        venue:             required_str(event, "venue")?.to_owned(),
        symbol:            required_str(event, "symbol")?.to_owned(),
        timeframe:         Timeframe::parse(required_str(event, "timeframe")?)?,
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

#[cfg(test)]
pub struct TestFeedDispatchSink {
    active:    std::collections::HashSet<SessionKey>,
    synthetic: tokio::sync::RwLock<Vec<String>>,
    silent:    tokio::sync::RwLock<Vec<serde_json::Value>>,
}

#[cfg(test)]
impl TestFeedDispatchSink {
    pub fn new(sessions: impl IntoIterator<Item = SessionKey>) -> Self {
        Self {
            active:    sessions.into_iter().collect(),
            synthetic: tokio::sync::RwLock::new(Vec::new()),
            silent:    tokio::sync::RwLock::new(Vec::new()),
        }
    }

    pub async fn synthetic_turns(&self) -> Vec<String> { self.synthetic.read().await.clone() }

    pub async fn silent_appends(&self) -> Vec<serde_json::Value> {
        self.silent.read().await.clone()
    }
}

#[cfg(test)]
#[async_trait]
impl FeedDispatchSink for TestFeedDispatchSink {
    async fn session_active(&self, session: &SessionKey) -> bool { self.active.contains(session) }

    async fn deliver_synthetic(&self, _owner: UserId, _session: SessionKey, directive: String) {
        self.synthetic.write().await.push(directive);
    }

    async fn append_feed_event(&self, _session: SessionKey, payload: serde_json::Value) {
        self.silent.write().await.push(payload);
    }
}

#[cfg(test)]
mod tests {
    use jiff::Timestamp;
    use rara_kernel::{
        data_feed::{FeedEvent, FeedEventId},
        identity::UserId,
        session::SessionKey,
    };
    use rara_trading::{
        finance::registry::{
            FinanceDelivery, FinanceEventKind, FinanceSubscription, FinanceSubscriptionRegistry,
        },
        market_data::InMemoryMarketDataRepository,
    };

    use super::{FeedDispatchOutcome, TestFeedDispatchSink, dispatch_feed_event};
    use crate::feed_store::InMemoryFeedStore;

    fn ts(value: &str) -> Timestamp { value.parse().expect("timestamp fixture should parse") }

    fn article(title: &str) -> FeedEvent {
        FeedEvent::builder()
            .id(FeedEventId::deterministic(title))
            .source_name("fed-news".to_owned())
            .event_type("rss_article".to_owned())
            .tags(vec!["finance".to_owned(), "source:fed-news".to_owned()])
            .payload(serde_json::json!({
                "title": title,
                "summary": "",
                "url": "https://example.com/article"
            }))
            .received_at(ts("2026-07-10T08:30:00Z"))
            .build()
    }

    fn candle() -> FeedEvent {
        FeedEvent::builder()
            .id(FeedEventId::deterministic(
                "binance:BTCUSDT:15m:2026-07-10T08:15:00Z",
            ))
            .source_name("binance-spot".to_owned())
            .event_type("market_candle_closed".to_owned())
            .tags(vec![
                "finance".to_owned(),
                "market-data".to_owned(),
                "venue:binance".to_owned(),
                "symbol:BTCUSDT".to_owned(),
                "timeframe:15m".to_owned(),
            ])
            .payload(serde_json::json!({
                "venue": "binance",
                "symbol": "BTCUSDT",
                "timeframe": "15m",
                "open_time": "2026-07-10T08:15:00Z",
                "close_time": "2026-07-10T08:30:00Z",
                "open": "61500.12",
                "high": "61640.00",
                "low": "61480.50",
                "close": "61610.30",
                "volume": "124.551"
            }))
            .received_at(ts("2026-07-10T08:30:00Z"))
            .build()
    }

    async fn registry_for(sub: FinanceSubscription) -> FinanceSubscriptionRegistry {
        let tmp = tempfile::tempdir().expect("tempdir should be created");
        let registry = FinanceSubscriptionRegistry::load(tmp.path().join("subs.json"));
        registry.upsert(sub).await.unwrap();
        registry
    }

    fn article_sub(session: SessionKey, delivery: FinanceDelivery) -> FinanceSubscription {
        FinanceSubscription {
            id: uuid::Uuid::new_v4(),
            owner: UserId("alice".to_owned()),
            session_key: session,
            event_kinds: vec![FinanceEventKind::RssArticle],
            source_names: vec!["fed-news".to_owned()],
            category_tags: Vec::new(),
            watch_terms: vec!["BTC".to_owned()],
            venues: Vec::new(),
            symbols: Vec::new(),
            timeframes: Vec::new(),
            delivery,
            cooldown_secs: 900,
            max_immediate_per_hour: 6,
        }
    }

    fn candle_sub(session: SessionKey, delivery: FinanceDelivery) -> FinanceSubscription {
        FinanceSubscription {
            id: uuid::Uuid::new_v4(),
            owner: UserId("alice".to_owned()),
            session_key: session,
            event_kinds: vec![FinanceEventKind::MarketCandleClosed],
            source_names: Vec::new(),
            category_tags: Vec::new(),
            watch_terms: Vec::new(),
            venues: vec!["binance".to_owned()],
            symbols: vec!["BTCUSDT".to_owned()],
            timeframes: vec!["15m".to_owned()],
            delivery,
            cooldown_secs: 900,
            max_immediate_per_hour: 6,
        }
    }

    #[tokio::test]
    async fn matched_immediate_finance_article_creates_one_synthetic_turn() {
        let session = SessionKey::new();
        let sink = TestFeedDispatchSink::new([session]);
        let outcome = dispatch_feed_event(
            &article("BTC liquidity note"),
            &InMemoryFeedStore::default(),
            &InMemoryMarketDataRepository::default(),
            &registry_for(article_sub(session, FinanceDelivery::Immediate)).await,
            &sink,
        )
        .await
        .unwrap();

        assert_eq!(
            outcome,
            FeedDispatchOutcome {
                persisted:         true,
                finance_decisions: 1,
            }
        );
        assert_eq!(sink.synthetic_turns().await.len(), 1);
        assert!(sink.synthetic_turns().await[0].contains("do not give trade advice"));
    }

    #[tokio::test]
    async fn matched_immediate_candle_creates_compact_market_update_turn() {
        let session = SessionKey::new();
        let sink = TestFeedDispatchSink::new([session]);
        dispatch_feed_event(
            &candle(),
            &InMemoryFeedStore::default(),
            &InMemoryMarketDataRepository::default(),
            &registry_for(candle_sub(session, FinanceDelivery::Immediate)).await,
            &sink,
        )
        .await
        .unwrap();

        let turns = sink.synthetic_turns().await;
        assert_eq!(turns.len(), 1);
        assert!(turns[0].contains("BTCUSDT"));
        assert!(turns[0].contains("do not infer a trade"));
    }

    #[tokio::test]
    async fn closed_candle_is_upserted_before_subscription_delivery() {
        let session = SessionKey::new();
        let market_repo = InMemoryMarketDataRepository::default();
        let sink = TestFeedDispatchSink::new([session]);
        dispatch_feed_event(
            &candle(),
            &InMemoryFeedStore::default(),
            &market_repo,
            &registry_for(candle_sub(session, FinanceDelivery::Immediate)).await,
            &sink,
        )
        .await
        .unwrap();

        assert_eq!(market_repo.correction_count().await, 0);
        assert_eq!(sink.synthetic_turns().await.len(), 1);
    }

    #[tokio::test]
    async fn silent_finance_event_appends_to_tape_without_turn() {
        let session = SessionKey::new();
        let sink = TestFeedDispatchSink::new([session]);
        dispatch_feed_event(
            &article("BTC liquidity note"),
            &InMemoryFeedStore::default(),
            &InMemoryMarketDataRepository::default(),
            &registry_for(article_sub(session, FinanceDelivery::Silent)).await,
            &sink,
        )
        .await
        .unwrap();

        assert!(sink.synthetic_turns().await.is_empty());
        assert_eq!(sink.silent_appends().await.len(), 1);
    }

    #[tokio::test]
    async fn unmatched_finance_event_does_not_wake_a_session() {
        let session = SessionKey::new();
        let sink = TestFeedDispatchSink::new([session]);
        dispatch_feed_event(
            &article("rate decision"),
            &InMemoryFeedStore::default(),
            &InMemoryMarketDataRepository::default(),
            &registry_for(article_sub(session, FinanceDelivery::Immediate)).await,
            &sink,
        )
        .await
        .unwrap();

        assert!(sink.synthetic_turns().await.is_empty());
        assert!(sink.silent_appends().await.is_empty());
    }
}
