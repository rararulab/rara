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

//! Behavior-parity tests for the market-signal dispatch facade.
//!
//! These drive [`on_feed_event`] end-to-end through in-memory doubles and pin
//! the exact delivery behavior (severity grading, budget/bypass, directive
//! wording) that must remain identical after the consolidation.

use std::sync::Arc;

use async_trait::async_trait;
use jiff::Timestamp;
use rara_kernel::{
    data_feed::{FeedEvent, FeedEventId, FeedFilter, FeedStore},
    identity::UserId,
    io::MessageId,
    notification::Subscription,
    queue::{ShardedEventQueue, ShardedEventQueueConfig},
    session::SessionKey,
    tool::{ToolContext, ToolExecute},
};
use rust_decimal::Decimal;

use super::{FeedDispatchOutcome, FeedDispatchSink, on_feed_event};
use crate::{
    finance::registry::{
        FinanceDelivery, FinanceEventKind, FinanceSubscription, FinanceSubscriptionRegistry,
        ManualFinanceClock,
    },
    market_data::{
        CandleLatestQuery, InMemoryMarketDataRepository, MarketCandle, MarketDataRepository,
        Timeframe,
        tools::{FinanceGetLatestCandleParams, FinanceGetLatestCandleTool},
    },
};

/// In-memory [`FeedStore`] double: dedupes by event id, filters on query.
#[derive(Default)]
struct InMemoryFeedStore {
    events: tokio::sync::RwLock<Vec<FeedEvent>>,
}

#[async_trait]
impl FeedStore for InMemoryFeedStore {
    async fn append(&self, event: &FeedEvent) -> rara_kernel::Result<()> {
        let mut events = self.events.write().await;
        if !events.iter().any(|existing| existing.id == event.id) {
            events.push(event.clone());
        }
        Ok(())
    }

    async fn query(&self, filter: FeedFilter) -> rara_kernel::Result<Vec<FeedEvent>> {
        let events = self.events.read().await;
        Ok(events
            .iter()
            .filter(|event| {
                filter
                    .source_name
                    .as_ref()
                    .is_none_or(|source| source == &event.source_name)
                    && filter
                        .since
                        .as_ref()
                        .is_none_or(|since| event.received_at >= *since)
                    && filter.tags.iter().all(|tag| event.tags.contains(tag))
            })
            .take(filter.limit)
            .cloned()
            .collect())
    }
}

/// In-memory [`FeedDispatchSink`] double recording synthetic turns and silent
/// appends, with a fixed set of "active" sessions.
struct TestFeedDispatchSink {
    active:    std::collections::HashSet<SessionKey>,
    synthetic: tokio::sync::RwLock<Vec<String>>,
    silent:    tokio::sync::RwLock<Vec<serde_json::Value>>,
}

impl TestFeedDispatchSink {
    fn new(sessions: impl IntoIterator<Item = SessionKey>) -> Self {
        Self {
            active:    sessions.into_iter().collect(),
            synthetic: tokio::sync::RwLock::new(Vec::new()),
            silent:    tokio::sync::RwLock::new(Vec::new()),
        }
    }

    async fn synthetic_turns(&self) -> Vec<String> { self.synthetic.read().await.clone() }

    async fn silent_appends(&self) -> Vec<serde_json::Value> { self.silent.read().await.clone() }
}

#[async_trait]
impl FeedDispatchSink for TestFeedDispatchSink {
    async fn session_active(&self, session: &SessionKey) -> bool { self.active.contains(session) }

    async fn deliver_synthetic(&self, _owner: UserId, _session: SessionKey, directive: String) {
        self.synthetic.write().await.push(directive);
    }

    async fn append_feed_event(&self, _session: SessionKey, payload: serde_json::Value) {
        self.silent.write().await.push(payload);
    }

    async fn generic_matches(&self, _tags: &[String]) -> Vec<Subscription> { Vec::new() }
}

fn ts(value: &str) -> Timestamp { value.parse().expect("timestamp fixture should parse") }

fn tool_context() -> ToolContext {
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

fn candle_with_selectors(venue: &str, symbol: &str, timeframe: &str) -> FeedEvent {
    FeedEvent::builder()
        .id(FeedEventId::deterministic(&format!(
            "{venue}:{symbol}:{timeframe}:2026-07-10T08:15:00Z"
        )))
        .source_name("binance-spot".to_owned())
        .event_type("market_candle_closed".to_owned())
        .tags(vec![
            "finance".to_owned(),
            "market-data".to_owned(),
            format!("venue:{venue}"),
            format!("symbol:{symbol}"),
            format!("timeframe:{timeframe}"),
        ])
        .payload(serde_json::json!({
            "venue": venue,
            "symbol": symbol,
            "timeframe": timeframe,
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

/// Build a `binance / BTCUSDT / 15m` candle at bar `index` (15m apart from
/// a fixed base), with `open = high = low = close` so drawdown/return
/// read the close series directly.
fn mk_candle(index: i64, close: i64, volume: i64) -> MarketCandle {
    let base = ts("2026-07-10T08:00:00Z");
    let open_time = base + jiff::SignedDuration::from_secs(index * 900);
    MarketCandle {
        source_name: "binance-spot".to_owned(),
        venue: "binance".to_owned(),
        symbol: "BTCUSDT".to_owned(),
        timeframe: Timeframe::parse("15m").expect("timeframe fixture should parse"),
        open_time,
        close_time: open_time + jiff::SignedDuration::from_secs(900),
        open: Decimal::from(close),
        high: Decimal::from(close),
        low: Decimal::from(close),
        close: Decimal::from(close),
        volume: Decimal::from(volume),
        ingested_at: open_time,
        provider_sequence: None,
    }
}

/// Render a `market_candle_closed` feed event carrying `candle`'s fields,
/// so the dispatched event and the seeded history share one shape.
fn candle_event_from(candle: &MarketCandle) -> FeedEvent {
    FeedEvent::builder()
        .id(FeedEventId::deterministic(&format!(
            "binance:BTCUSDT:15m:{}",
            candle.open_time
        )))
        .source_name(candle.source_name.clone())
        .event_type("market_candle_closed".to_owned())
        .tags(vec![
            "finance".to_owned(),
            "market-data".to_owned(),
            "venue:binance".to_owned(),
            "symbol:BTCUSDT".to_owned(),
            "timeframe:15m".to_owned(),
        ])
        .payload(serde_json::json!({
            "venue": candle.venue,
            "symbol": candle.symbol,
            "timeframe": candle.timeframe.as_str(),
            "open_time": candle.open_time.to_string(),
            "close_time": candle.close_time.to_string(),
            "open": candle.open.to_string(),
            "high": candle.high.to_string(),
            "low": candle.low.to_string(),
            "close": candle.close.to_string(),
            "volume": candle.volume.to_string(),
        }))
        .received_at(candle.ingested_at)
        .build()
}

#[tokio::test]
async fn matched_immediate_finance_article_creates_one_synthetic_turn() {
    let session = SessionKey::new();
    let sink = TestFeedDispatchSink::new([session]);
    let outcome = on_feed_event(
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
async fn duplicate_immediate_finance_event_does_not_wake_session_twice() {
    let session = SessionKey::new();
    let sink = TestFeedDispatchSink::new([session]);
    let registry = registry_for(article_sub(session, FinanceDelivery::Immediate)).await;
    let event = article("BTC liquidity note");

    let first = on_feed_event(
        &event,
        &InMemoryFeedStore::default(),
        &InMemoryMarketDataRepository::default(),
        &registry,
        &sink,
    )
    .await
    .unwrap();
    let second = on_feed_event(
        &event,
        &InMemoryFeedStore::default(),
        &InMemoryMarketDataRepository::default(),
        &registry,
        &sink,
    )
    .await
    .unwrap();

    assert_eq!(first.finance_decisions, 1);
    assert_eq!(second.finance_decisions, 0);
    assert_eq!(sink.synthetic_turns().await.len(), 1);
    assert!(sink.silent_appends().await.is_empty());
}

#[tokio::test]
async fn default_immediate_budget_silences_seventh_finance_event() {
    let session = SessionKey::new();
    let sink = TestFeedDispatchSink::new([session]);
    let mut subscription = article_sub(session, FinanceDelivery::Immediate);
    subscription.cooldown_secs = 0;
    let registry = registry_for(subscription).await;

    for index in 1..=7 {
        on_feed_event(
            &article(&format!("BTC notice {index}")),
            &InMemoryFeedStore::default(),
            &InMemoryMarketDataRepository::default(),
            &registry,
            &sink,
        )
        .await
        .unwrap();
    }

    assert_eq!(sink.synthetic_turns().await.len(), 6);
    let silent = sink.silent_appends().await;
    assert_eq!(silent.len(), 1);
    assert_eq!(silent[0]["payload"]["title"], "BTC notice 7");
}

#[tokio::test]
async fn matched_immediate_candle_creates_compact_market_update_turn() {
    let session = SessionKey::new();
    let sink = TestFeedDispatchSink::new([session]);
    on_feed_event(
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
    on_feed_event(
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
async fn closed_candle_event_selectors_are_normalized_before_upsert() {
    let session = SessionKey::new();
    let market_repo = InMemoryMarketDataRepository::default();
    let sink = TestFeedDispatchSink::new([session]);
    on_feed_event(
        &candle_with_selectors(" Binance ", " btcusdt ", " 15M "),
        &InMemoryFeedStore::default(),
        &market_repo,
        &registry_for(candle_sub(session, FinanceDelivery::Immediate)).await,
        &sink,
    )
    .await
    .unwrap();

    let candle = market_repo
        .latest_closed_candle(CandleLatestQuery {
            source_name: Some("binance-spot".to_owned()),
            venue:       "binance".to_owned(),
            symbol:      "BTCUSDT".to_owned(),
            timeframe:   Timeframe::parse("15m").unwrap(),
        })
        .await
        .unwrap()
        .expect("canonical candle should be stored");

    assert_eq!(candle.venue, "binance");
    assert_eq!(candle.symbol, "BTCUSDT");
    assert_eq!(candle.timeframe.to_string(), "15m");
    assert_eq!(sink.synthetic_turns().await.len(), 1);
}

#[tokio::test]
async fn closed_candle_dispatch_is_queryable_through_latest_candle_tool() {
    let session = SessionKey::new();
    let market_repo = Arc::new(InMemoryMarketDataRepository::default());
    let sink = TestFeedDispatchSink::new([session]);
    on_feed_event(
        &candle_with_selectors(" Binance ", " btcusdt ", " 15M "),
        &InMemoryFeedStore::default(),
        market_repo.as_ref(),
        &registry_for(candle_sub(session, FinanceDelivery::Silent)).await,
        &sink,
    )
    .await
    .unwrap();

    let result = FinanceGetLatestCandleTool::new(market_repo)
        .run(
            FinanceGetLatestCandleParams {
                source_name: Some("binance-spot".to_owned()),
                venue:       " Binance ".to_owned(),
                symbol:      " btcusdt ".to_owned(),
                timeframe:   " 15M ".to_owned(),
            },
            &tool_context(),
        )
        .await
        .unwrap();
    let candle = result.candle.expect("latest candle should be queryable");

    assert_eq!(candle.source_name, "binance-spot");
    assert_eq!(candle.venue, "binance");
    assert_eq!(candle.symbol, "BTCUSDT");
    assert_eq!(candle.timeframe, "15m");
    assert_eq!(candle.close, "61610.30");
}

#[tokio::test]
async fn silent_finance_event_appends_to_tape_without_turn() {
    let session = SessionKey::new();
    let sink = TestFeedDispatchSink::new([session]);
    on_feed_event(
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
    on_feed_event(
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

#[tokio::test]
async fn anomaly_signal_enriches_finance_directive() {
    // A crash-shaped rolling window: six flat bars, a four-bar decline, and
    // a newly closed bar that completes a ~7% drop on a 5x volume spike.
    let session = SessionKey::new();
    let market_repo = InMemoryMarketDataRepository::default();
    let sink = TestFeedDispatchSink::new([session]);
    let registry = registry_for(candle_sub(session, FinanceDelivery::Immediate)).await;

    let history = [
        (61_500, 120),
        (61_500, 120),
        (61_500, 120),
        (61_500, 120),
        (61_500, 120),
        (61_500, 120),
        (61_000, 120),
        (60_000, 120),
        (59_000, 120),
        (58_000, 120),
    ];
    for (index, (close, volume)) in history.iter().enumerate() {
        market_repo
            .upsert_closed_candle(mk_candle(index as i64, *close, *volume))
            .await
            .unwrap();
    }
    let crash = mk_candle(10, 57_000, 600);

    on_feed_event(
        &candle_event_from(&crash),
        &InMemoryFeedStore::default(),
        &market_repo,
        &registry,
        &sink,
    )
    .await
    .unwrap();

    let turns = sink.synthetic_turns().await;
    assert_eq!(turns.len(), 1);
    assert!(
        turns[0].contains("[Anomaly severity=critical"),
        "directive should name the severity: {}",
        turns[0]
    );
    assert!(
        turns[0].contains("drawdown"),
        "directive should carry the anomaly reason: {}",
        turns[0]
    );
    assert!(
        turns[0].contains("Describe what happened") && turns[0].contains("suggested next action"),
        "directive should instruct narration and a suggested action: {}",
        turns[0]
    );

    // A flat window over the same stream keeps the unchanged factual wording.
    let flat_session = SessionKey::new();
    let flat_repo = InMemoryMarketDataRepository::default();
    let flat_sink = TestFeedDispatchSink::new([flat_session]);
    let flat_registry = registry_for(candle_sub(flat_session, FinanceDelivery::Immediate)).await;

    for index in 0..10 {
        let close = if index % 2 == 0 { 61_500 } else { 61_510 };
        flat_repo
            .upsert_closed_candle(mk_candle(index, close, 120))
            .await
            .unwrap();
    }
    let flat = mk_candle(10, 61_505, 122);

    on_feed_event(
        &candle_event_from(&flat),
        &InMemoryFeedStore::default(),
        &flat_repo,
        &flat_registry,
        &flat_sink,
    )
    .await
    .unwrap();

    let flat_turns = flat_sink.synthetic_turns().await;
    assert_eq!(flat_turns.len(), 1);
    assert!(
        flat_turns[0].contains("Report the market update factually"),
        "flat window keeps the factual wording: {}",
        flat_turns[0]
    );
    assert!(
        !flat_turns[0].contains("[Anomaly"),
        "flat window must not be enriched: {}",
        flat_turns[0]
    );
}

#[tokio::test]
async fn critical_candle_burst_wakes_session_every_bar() {
    // A crash burst: 30 flat bars of history, then eight consecutive bars
    // each dropping well past the 6% critical-drawdown floor, so every
    // burst bar evaluates to Severity::Critical.
    let crash_prices = [
        57_000, 55_000, 53_000, 51_000, 49_000, 47_000, 45_000, 43_000,
    ];

    let session = SessionKey::new();
    let market_repo = InMemoryMarketDataRepository::default();
    let sink = TestFeedDispatchSink::new([session]);
    let clock = Arc::new(ManualFinanceClock::new(ts("2026-07-10T08:00:00Z")));

    // Default budget (6); cooldown disabled so the hourly budget is the sole
    // routine throttle — the lane the scenario names ("after the budget is
    // spent"). A high-severity anomaly must bypass it on every bar.
    let tmp = tempfile::tempdir().expect("tempdir should be created");
    let registry =
        FinanceSubscriptionRegistry::load_with_clock(tmp.path().join("subs.json"), clock.clone());
    let mut subscription = candle_sub(session, FinanceDelivery::Immediate);
    subscription.cooldown_secs = 0;
    registry.upsert(subscription).await.unwrap();

    for index in 0..30 {
        market_repo
            .upsert_closed_candle(mk_candle(index, 61_500, 120))
            .await
            .unwrap();
    }

    // Dispatch the eight-bar crash burst (indices 30..38), advancing 60s per
    // bar so every delivery falls inside one hourly-budget window.
    for (offset, &close) in crash_prices.iter().enumerate() {
        let crash = mk_candle(30 + offset as i64, close, 600);
        on_feed_event(
            &candle_event_from(&crash),
            &InMemoryFeedStore::default(),
            &market_repo,
            &registry,
            &sink,
        )
        .await
        .unwrap();
        clock.advance(jiff::SignedDuration::from_secs(60));
    }

    let turns = sink.synthetic_turns().await;
    assert_eq!(
        turns.len(),
        8,
        "each critical bar must wake the session, not be throttled"
    );
    assert!(sink.silent_appends().await.is_empty());
    assert!(
        turns
            .iter()
            .all(|turn| turn.contains("[Anomaly severity=critical")),
        "every burst turn should carry the critical anomaly directive: {turns:?}"
    );

    // Contrast: a flat burst carries no anomaly (severity None), so the
    // routine budget applies — the first six wake the session, the rest are
    // silenced. Same stream, same budget, only the severity differs.
    let flat_session = SessionKey::new();
    let flat_repo = InMemoryMarketDataRepository::default();
    let flat_sink = TestFeedDispatchSink::new([flat_session]);
    let flat_clock = Arc::new(ManualFinanceClock::new(ts("2026-07-10T08:00:00Z")));
    let flat_tmp = tempfile::tempdir().expect("tempdir should be created");
    let flat_registry = FinanceSubscriptionRegistry::load_with_clock(
        flat_tmp.path().join("subs.json"),
        flat_clock.clone(),
    );
    let mut flat_sub = candle_sub(flat_session, FinanceDelivery::Immediate);
    flat_sub.cooldown_secs = 0;
    flat_registry.upsert(flat_sub).await.unwrap();

    for index in 0..30 {
        flat_repo
            .upsert_closed_candle(mk_candle(index, 61_500, 120))
            .await
            .unwrap();
    }
    for offset in 0..8 {
        let flat = mk_candle(30 + offset, 61_500, 120);
        on_feed_event(
            &candle_event_from(&flat),
            &InMemoryFeedStore::default(),
            &flat_repo,
            &flat_registry,
            &flat_sink,
        )
        .await
        .unwrap();
        flat_clock.advance(jiff::SignedDuration::from_secs(60));
    }

    assert_eq!(
        flat_sink.synthetic_turns().await.len(),
        6,
        "the routine budget still caps a low-severity burst at six turns"
    );
    assert_eq!(flat_sink.silent_appends().await.len(), 2);
}
