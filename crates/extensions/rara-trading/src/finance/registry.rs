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
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicI64, Ordering},
    },
};

use jiff::{SignedDuration, Timestamp};
use rara_kernel::{
    data_feed::{FeedEvent, FeedEventId},
    identity::UserId,
    session::SessionKey,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::warn;
use uuid::Uuid;

use crate::{anomaly::Severity, market_data::Timeframe};

/// Finance event kinds supported by the MVP.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FinanceEventKind {
    RssArticle,
    MarketCandleClosed,
}

impl FinanceEventKind {
    fn from_event(event: &FeedEvent) -> Option<Self> {
        match event.event_type.as_str() {
            "rss_article" => Some(Self::RssArticle),
            "market_candle_closed" => Some(Self::MarketCandleClosed),
            _ => None,
        }
    }
}

/// Requested delivery policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FinanceDelivery {
    Immediate,
    Silent,
}

/// Actual delivery action after budget/cooldown enforcement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinanceDeliveryAction {
    Immediate,
    Silent,
}

/// User subscription to finance information events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FinanceSubscription {
    pub id:                     Uuid,
    pub owner:                  UserId,
    pub session_key:            SessionKey,
    pub event_kinds:            Vec<FinanceEventKind>,
    pub source_names:           Vec<String>,
    pub category_tags:          Vec<String>,
    pub watch_terms:            Vec<String>,
    pub venues:                 Vec<String>,
    pub symbols:                Vec<String>,
    pub timeframes:             Vec<String>,
    pub delivery:               FinanceDelivery,
    pub cooldown_secs:          u64,
    pub max_immediate_per_hour: u16,
}

/// Match decision returned by the registry. Side effects are left to the app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinanceDeliveryDecision {
    pub subscription_id:   Uuid,
    pub owner:             UserId,
    pub session_key:       SessionKey,
    pub event_id:          FeedEventId,
    pub action:            FinanceDeliveryAction,
    /// Set when the routine lane silenced a low-severity event, naming the
    /// throttle that fired (`cooldown` / `hourly_budget`).
    pub downgraded_reason: Option<String>,
    /// Set when a bypass-eligible severity overrode a throttle that would
    /// otherwise have silenced the event, naming the bypassed constraint. Keeps
    /// the alert-bypass decision inspectable rather than a silent control path.
    pub bypass_reason:     Option<String>,
}

/// Clock abstraction for deterministic budget tests.
pub trait FinanceClock: Send + Sync {
    fn now(&self) -> Timestamp;
}

/// System wall-clock implementation.
#[derive(Debug, Default)]
pub struct SystemFinanceClock;

impl FinanceClock for SystemFinanceClock {
    fn now(&self) -> Timestamp { Timestamp::now() }
}

/// Manually controlled clock for tests.
#[derive(Debug)]
pub struct ManualFinanceClock {
    seconds: AtomicI64,
}

impl ManualFinanceClock {
    pub fn new(now: Timestamp) -> Self {
        Self {
            seconds: AtomicI64::new(now.as_second()),
        }
    }

    pub fn advance(&self, delta: SignedDuration) {
        let secs = delta.as_secs();
        self.seconds.fetch_add(secs, Ordering::SeqCst);
    }
}

impl FinanceClock for ManualFinanceClock {
    fn now(&self) -> Timestamp {
        Timestamp::from_second(self.seconds.load(Ordering::SeqCst))
            .expect("manual test clock should stay in timestamp range")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DeliveryLedgerEntry {
    subscription_id: Uuid,
    event_id:        String,
    delivered_at:    Timestamp,
    action:          FinanceDeliveryAction,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct PersistedState {
    subscriptions: Vec<FinanceSubscription>,
    ledger:        Vec<DeliveryLedgerEntry>,
}

#[derive(Debug)]
struct RegistryInner {
    subscriptions: HashMap<Uuid, FinanceSubscription>,
    ledger:        Vec<DeliveryLedgerEntry>,
    path:          PathBuf,
}

/// JSON-backed finance subscription registry.
pub struct FinanceSubscriptionRegistry {
    inner: RwLock<RegistryInner>,
    clock: Arc<dyn FinanceClock>,
}

impl FinanceSubscriptionRegistry {
    /// Load a registry using the system clock.
    #[must_use]
    pub fn load(path: PathBuf) -> Self { Self::load_with_clock(path, Arc::new(SystemFinanceClock)) }

    /// Load a registry with an injected clock.
    #[must_use]
    pub fn load_with_clock(path: PathBuf, clock: Arc<dyn FinanceClock>) -> Self {
        let state = match std::fs::read_to_string(&path) {
            Ok(content) => match serde_json::from_str::<PersistedState>(&content) {
                Ok(state) => state,
                Err(err) => {
                    warn!(error = %err, path = %path.display(), "failed to parse finance subscriptions");
                    PersistedState::default()
                }
            },
            Err(_) => PersistedState::default(),
        };

        Self {
            inner: RwLock::new(RegistryInner {
                subscriptions: state
                    .subscriptions
                    .into_iter()
                    .map(|subscription| (subscription.id, normalize_subscription(subscription)))
                    .collect(),
                ledger: state.ledger,
                path,
            }),
            clock,
        }
    }

    /// Insert or replace a subscription, then persist state.
    pub async fn upsert(&self, subscription: FinanceSubscription) -> anyhow::Result<Uuid> {
        let subscription = normalize_subscription(subscription);
        let id = subscription.id;
        let mut inner = self.inner.write().await;
        inner.subscriptions.insert(id, subscription);
        persist(&inner)?;
        Ok(id)
    }

    /// Remove a subscription owned by `owner`.
    pub async fn remove(&self, owner: &UserId, id: Uuid) -> anyhow::Result<bool> {
        let mut inner = self.inner.write().await;
        let can_remove = inner
            .subscriptions
            .get(&id)
            .is_some_and(|subscription| &subscription.owner == owner);
        if !can_remove {
            return Ok(false);
        }
        inner.subscriptions.remove(&id);
        inner.ledger.retain(|entry| entry.subscription_id != id);
        persist(&inner)?;
        Ok(true)
    }

    /// List subscriptions for one owner.
    pub async fn list_for_owner(&self, owner: &UserId) -> Vec<FinanceSubscription> {
        let inner = self.inner.read().await;
        inner
            .subscriptions
            .values()
            .filter(|subscription| &subscription.owner == owner)
            .cloned()
            .collect()
    }

    /// Match an incoming feed event and record delivery decisions.
    ///
    /// `severity` is the anomaly severity the app evaluated for this event (see
    /// `crates/app/src/finance_event.rs`), or `None` when the event carried no
    /// anomaly. A severity at or above the private `BYPASS_SEVERITY` threshold
    /// overrides the routine cooldown / hourly-budget throttle so a genuine
    /// alert is never the thing that gets silenced.
    pub async fn match_event(
        &self,
        event: &FeedEvent,
        severity: Option<Severity>,
    ) -> Vec<FinanceDeliveryDecision> {
        if !event.tags.iter().any(|tag| tag == "finance") {
            return Vec::new();
        }
        let Some(kind) = FinanceEventKind::from_event(event) else {
            return Vec::new();
        };

        let now = self.clock.now();
        let mut inner = self.inner.write().await;
        let event_id = event.id.to_string();
        let mut decisions = Vec::new();
        let mut new_ledger_entries = Vec::new();

        for subscription in inner.subscriptions.values() {
            if already_delivered(&inner.ledger, subscription.id, &event_id) {
                continue;
            }
            if !matches_subscription(subscription, kind, event) {
                continue;
            }

            let verdict = delivery_action(subscription, &inner.ledger, now, severity);
            decisions.push(FinanceDeliveryDecision {
                subscription_id:   subscription.id,
                owner:             subscription.owner.clone(),
                session_key:       subscription.session_key,
                event_id:          event.id.clone(),
                action:            verdict.action,
                downgraded_reason: verdict.downgraded_reason,
                bypass_reason:     verdict.bypass_reason,
            });
            new_ledger_entries.push(DeliveryLedgerEntry {
                subscription_id: subscription.id,
                event_id:        event_id.clone(),
                delivered_at:    now,
                action:          verdict.action,
            });
        }

        if !new_ledger_entries.is_empty() {
            inner.ledger.extend(new_ledger_entries);
            if let Err(err) = persist(&inner) {
                warn!(error = %err, "failed to persist finance subscription decisions");
            }
        }

        decisions
    }
}

fn persist(inner: &RegistryInner) -> anyhow::Result<()> {
    let state = PersistedState {
        subscriptions: inner.subscriptions.values().cloned().collect(),
        ledger:        inner.ledger.clone(),
    };
    if let Some(parent) = inner.path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&inner.path, serde_json::to_string_pretty(&state)?)?;
    Ok(())
}

fn already_delivered(
    ledger: &[DeliveryLedgerEntry],
    subscription_id: Uuid,
    event_id: &str,
) -> bool {
    ledger
        .iter()
        .any(|entry| entry.subscription_id == subscription_id && entry.event_id == event_id)
}

/// Severity at or above which an anomaly bypasses the routine throttle.
///
/// This is a mechanism constant, not a per-deployment knob: "which severity
/// counts as an alert" is a property of the alerting design, and a deploy
/// operator has no principled reason to retune it. A YAML knob here would
/// recreate the config-silently-disables-the-fix footgun
/// (`docs/guides/anti-patterns.md`). The per-subscription `cooldown_secs` /
/// `max_immediate_per_hour` remain genuine user preferences.
const BYPASS_SEVERITY: Severity = Severity::Critical;

/// Outcome of the budget / cooldown / bypass rule for one matched event.
struct DeliveryVerdict {
    action:            FinanceDeliveryAction,
    /// Set when the routine lane silenced a low-severity event (`cooldown` /
    /// `hourly_budget`).
    downgraded_reason: Option<String>,
    /// Set when a bypass-eligible severity overrode a throttle that would
    /// otherwise have silenced the event, naming the bypassed constraint.
    bypass_reason:     Option<String>,
}

fn delivery_action(
    subscription: &FinanceSubscription,
    ledger: &[DeliveryLedgerEntry],
    now: Timestamp,
    severity: Option<Severity>,
) -> DeliveryVerdict {
    if subscription.delivery == FinanceDelivery::Silent {
        return DeliveryVerdict {
            action:            FinanceDeliveryAction::Silent,
            downgraded_reason: None,
            bypass_reason:     None,
        };
    }

    // What the budget / cooldown rule would do on its own, ignoring severity:
    // `Some(reason)` means the routine throttle would silence this event.
    let throttle_reason = routine_throttle_reason(subscription, ledger, now);
    let is_alert = severity.is_some_and(|level| level >= BYPASS_SEVERITY);

    match throttle_reason {
        // An alert-grade severity overrides a throttle that would have silenced
        // the event: deliver immediately and record which constraint it bypassed.
        Some(reason) if is_alert => DeliveryVerdict {
            action:            FinanceDeliveryAction::Immediate,
            downgraded_reason: None,
            bypass_reason:     Some(reason),
        },
        // Routine lane unchanged: a below-threshold event stays silenced.
        Some(reason) => DeliveryVerdict {
            action:            FinanceDeliveryAction::Silent,
            downgraded_reason: Some(reason),
            bypass_reason:     None,
        },
        // No throttle fired; deliver immediately regardless of severity.
        None => DeliveryVerdict {
            action:            FinanceDeliveryAction::Immediate,
            downgraded_reason: None,
            bypass_reason:     None,
        },
    }
}

/// Return the routine-throttle disposition for an `Immediate` subscription.
///
/// `Some(reason)` when the trailing-hour cooldown / hourly-budget rule would
/// downgrade this event to `Silent` (naming the throttle that fired), `None`
/// when the event may be delivered immediately.
fn routine_throttle_reason(
    subscription: &FinanceSubscription,
    ledger: &[DeliveryLedgerEntry],
    now: Timestamp,
) -> Option<String> {
    let immediate_entries: Vec<&DeliveryLedgerEntry> = ledger
        .iter()
        .filter(|entry| {
            entry.subscription_id == subscription.id
                && entry.action == FinanceDeliveryAction::Immediate
                && now.duration_since(entry.delivered_at) < SignedDuration::from_secs(60 * 60)
        })
        .collect();

    if subscription.cooldown_secs > 0
        && immediate_entries.iter().any(|entry| {
            now.duration_since(entry.delivered_at)
                < SignedDuration::from_secs(subscription.cooldown_secs as i64)
        })
    {
        return Some("cooldown".to_owned());
    }

    if immediate_entries.len() >= usize::from(subscription.max_immediate_per_hour) {
        return Some("hourly_budget".to_owned());
    }

    None
}

fn matches_subscription(
    subscription: &FinanceSubscription,
    kind: FinanceEventKind,
    event: &FeedEvent,
) -> bool {
    group_matches(&subscription.event_kinds, &kind)
        && string_group_matches(&subscription.source_names, &event.source_name)
        && tags_group_matches(&subscription.category_tags, &event.tags)
        && watch_terms_match(subscription, event)
        && candle_fields_match(subscription, event)
}

fn group_matches<T: PartialEq>(values: &[T], actual: &T) -> bool {
    values.is_empty() || values.iter().any(|value| value == actual)
}

fn string_group_matches(values: &[String], actual: &str) -> bool {
    values.is_empty() || values.iter().any(|value| value == actual)
}

fn tags_group_matches(values: &[String], tags: &[String]) -> bool {
    values.is_empty()
        || values
            .iter()
            .any(|value| tags.iter().any(|tag| tag == value))
}

fn watch_terms_match(subscription: &FinanceSubscription, event: &FeedEvent) -> bool {
    if subscription.watch_terms.is_empty() {
        return true;
    }
    if event.event_type != "rss_article" {
        return true;
    }
    let haystack = normalize_text(&format!(
        "{} {}",
        event.payload["title"].as_str().unwrap_or_default(),
        event.payload["summary"].as_str().unwrap_or_default()
    ));
    subscription
        .watch_terms
        .iter()
        .any(|term| haystack.contains(&normalize_text(term)))
}

fn candle_fields_match(subscription: &FinanceSubscription, event: &FeedEvent) -> bool {
    if event.event_type != "market_candle_closed" {
        return true;
    }
    let venue = event
        .payload
        .get("venue")
        .and_then(serde_json::Value::as_str)
        .map(normalize_venue)
        .unwrap_or_default();
    let symbol = event
        .payload
        .get("symbol")
        .and_then(serde_json::Value::as_str)
        .map(normalize_symbol)
        .unwrap_or_default();
    let timeframe = event
        .payload
        .get("timeframe")
        .and_then(serde_json::Value::as_str)
        .map(normalize_timeframe)
        .transpose()
        .unwrap_or(None)
        .unwrap_or_default();

    string_group_matches(&subscription.venues, &venue)
        && string_group_matches(&subscription.symbols, &symbol)
        && string_group_matches(&subscription.timeframes, &timeframe)
}

fn normalize_subscription(mut subscription: FinanceSubscription) -> FinanceSubscription {
    subscription.source_names = dedupe(
        subscription
            .source_names
            .into_iter()
            .map(|source_name| source_name.trim().to_owned())
            .filter(|source_name| !source_name.is_empty())
            .collect(),
    );
    subscription.category_tags = dedupe(
        subscription
            .category_tags
            .into_iter()
            .filter_map(|tag| {
                let tag = tag.trim();
                let normalized = if tag
                    .get(.."category:".len())
                    .is_some_and(|prefix| prefix.eq_ignore_ascii_case("category:"))
                {
                    normalize_tag(&tag["category:".len()..])
                } else {
                    normalize_tag(tag)
                };
                (!normalized.is_empty()).then(|| format!("category:{normalized}"))
            })
            .collect(),
    );
    subscription.watch_terms = dedupe(
        subscription
            .watch_terms
            .into_iter()
            .map(|term| normalize_text(&term))
            .filter(|term| !term.is_empty())
            .collect(),
    );
    subscription.venues = dedupe(
        subscription
            .venues
            .into_iter()
            .map(|venue| normalize_venue(&venue))
            .filter(|venue| !venue.is_empty())
            .collect(),
    );
    subscription.symbols = dedupe(
        subscription
            .symbols
            .into_iter()
            .map(|symbol| normalize_symbol(&symbol))
            .filter(|symbol| !symbol.is_empty())
            .collect(),
    );
    subscription.timeframes = dedupe(
        subscription
            .timeframes
            .into_iter()
            .map(|timeframe| {
                normalize_timeframe(&timeframe)
                    .unwrap_or_else(|_| timeframe.trim().to_ascii_lowercase())
            })
            .filter(|timeframe| !timeframe.is_empty())
            .collect(),
    );
    subscription
}

fn normalize_venue(value: &str) -> String { value.trim().to_ascii_lowercase() }

fn normalize_symbol(value: &str) -> String { value.trim().to_ascii_uppercase() }

fn normalize_timeframe(value: &str) -> anyhow::Result<String> {
    let value = value.trim().to_ascii_lowercase();
    if value.is_empty() {
        return Ok(String::new());
    }
    Ok(Timeframe::parse(value)?.to_string())
}

fn dedupe(mut values: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    values.retain(|value| seen.insert(value.clone()));
    values
}

fn normalize_text(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .flat_map(char::to_lowercase)
        .collect()
}

fn normalize_tag(value: &str) -> String {
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
    if last_dash {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use jiff::{SignedDuration, Timestamp};
    use rara_kernel::{
        data_feed::{FeedEvent, FeedEventId},
        identity::UserId,
        session::SessionKey,
    };
    use tempfile::TempDir;

    use super::{
        FinanceDelivery, FinanceDeliveryAction, FinanceEventKind, FinanceSubscription,
        FinanceSubscriptionRegistry, ManualFinanceClock, Severity,
    };

    fn ts(value: &str) -> Timestamp { value.parse().expect("timestamp fixture should parse") }

    fn owner() -> UserId { UserId("alice".to_owned()) }

    fn registry(
        now: Timestamp,
    ) -> (
        FinanceSubscriptionRegistry,
        Arc<ManualFinanceClock>,
        TempDir,
    ) {
        let tmp = tempfile::tempdir().expect("tempdir should be created");
        let clock = Arc::new(ManualFinanceClock::new(now));
        let registry = FinanceSubscriptionRegistry::load_with_clock(
            tmp.path().join("subs.json"),
            clock.clone(),
        );
        (registry, clock, tmp)
    }

    fn sub(session_key: SessionKey) -> FinanceSubscription {
        FinanceSubscription {
            id: uuid::Uuid::new_v4(),
            owner: owner(),
            session_key,
            event_kinds: vec![FinanceEventKind::RssArticle],
            source_names: vec!["fed-news".to_owned()],
            category_tags: Vec::new(),
            watch_terms: vec!["BTC".to_owned()],
            venues: Vec::new(),
            symbols: Vec::new(),
            timeframes: Vec::new(),
            delivery: FinanceDelivery::Immediate,
            cooldown_secs: 900,
            max_immediate_per_hour: 6,
        }
    }

    fn article(source: &str, title: &str, summary: &str) -> FeedEvent {
        FeedEvent::builder()
            .id(FeedEventId::deterministic(&format!("{source}:{title}")))
            .source_name(source.to_owned())
            .event_type("rss_article".to_owned())
            .tags(vec![
                "finance".to_owned(),
                format!("source:{source}"),
                "category:monetary-policy".to_owned(),
            ])
            .payload(serde_json::json!({
                "title": title,
                "summary": summary,
                "url": "https://example.com/article"
            }))
            .received_at(ts("2026-07-10T08:30:00Z"))
            .build()
    }

    fn candle(symbol: &str, timeframe: &str) -> FeedEvent {
        FeedEvent::builder()
            .id(FeedEventId::deterministic(&format!(
                "binance:{symbol}:{timeframe}"
            )))
            .source_name("binance-spot".to_owned())
            .event_type("market_candle_closed".to_owned())
            .tags(vec![
                "finance".to_owned(),
                "market-data".to_owned(),
                "venue:binance".to_owned(),
                format!("symbol:{symbol}"),
                format!("timeframe:{timeframe}"),
            ])
            .payload(serde_json::json!({
                "venue": "binance",
                "symbol": symbol,
                "timeframe": timeframe,
                "close_time": "2026-07-10T08:30:00Z",
                "close": "61610.30"
            }))
            .received_at(ts("2026-07-10T08:30:00Z"))
            .build()
    }

    fn candle_with_payload(venue: &str, symbol: &str, timeframe: &str) -> FeedEvent {
        FeedEvent::builder()
            .id(FeedEventId::deterministic(&format!(
                "{venue}:{symbol}:{timeframe}"
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
                "close_time": "2026-07-10T08:30:00Z",
                "close": "61610.30"
            }))
            .received_at(ts("2026-07-10T08:30:00Z"))
            .build()
    }

    #[tokio::test]
    async fn article_source_and_watch_terms_are_anded() {
        let (registry, _clock, _tmp) = registry(ts("2026-07-10T08:31:00Z"));
        let session = SessionKey::new();
        registry.upsert(sub(session)).await.unwrap();

        assert_eq!(
            registry
                .match_event(&article("fed-news", "Fed mentions BTC liquidity", ""), None)
                .await
                .len(),
            1
        );
        assert!(
            registry
                .match_event(&article("fed-news", "Fed holds rates", ""), None)
                .await
                .is_empty()
        );
        assert!(
            registry
                .match_event(&article("other-news", "BTC headline", ""), None)
                .await
                .is_empty()
        );
    }

    #[tokio::test]
    async fn candle_symbol_and_timeframe_are_anded() {
        let (registry, _clock, _tmp) = registry(ts("2026-07-10T08:31:00Z"));
        let session = SessionKey::new();
        registry
            .upsert(FinanceSubscription {
                id:                     uuid::Uuid::new_v4(),
                owner:                  owner(),
                session_key:            session,
                event_kinds:            vec![FinanceEventKind::MarketCandleClosed],
                source_names:           Vec::new(),
                category_tags:          Vec::new(),
                watch_terms:            Vec::new(),
                venues:                 vec!["binance".to_owned()],
                symbols:                vec!["BTCUSDT".to_owned()],
                timeframes:             vec!["15m".to_owned()],
                delivery:               FinanceDelivery::Silent,
                cooldown_secs:          900,
                max_immediate_per_hour: 6,
            })
            .await
            .unwrap();

        assert_eq!(
            registry
                .match_event(&candle("BTCUSDT", "15m"), None)
                .await
                .len(),
            1
        );
        assert!(
            registry
                .match_event(&candle("ETHUSDT", "15m"), None)
                .await
                .is_empty()
        );
        assert!(
            registry
                .match_event(&candle("BTCUSDT", "1h"), None)
                .await
                .is_empty()
        );
    }

    #[tokio::test]
    async fn candle_selectors_are_normalized_before_matching() {
        let (registry, _clock, _tmp) = registry(ts("2026-07-10T08:31:00Z"));
        let session = SessionKey::new();
        registry
            .upsert(FinanceSubscription {
                id:                     uuid::Uuid::new_v4(),
                owner:                  owner(),
                session_key:            session,
                event_kinds:            vec![FinanceEventKind::MarketCandleClosed],
                source_names:           Vec::new(),
                category_tags:          Vec::new(),
                watch_terms:            Vec::new(),
                venues:                 vec![" Binance ".to_owned()],
                symbols:                vec![" btcusdt ".to_owned()],
                timeframes:             vec![" 15M ".to_owned()],
                delivery:               FinanceDelivery::Silent,
                cooldown_secs:          900,
                max_immediate_per_hour: 6,
            })
            .await
            .unwrap();

        assert_eq!(
            registry
                .match_event(&candle("BTCUSDT", "15m"), None)
                .await
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn candle_event_payload_selectors_are_normalized_before_matching() {
        let (registry, _clock, _tmp) = registry(ts("2026-07-10T08:31:00Z"));
        let session = SessionKey::new();
        registry
            .upsert(FinanceSubscription {
                id:                     uuid::Uuid::new_v4(),
                owner:                  owner(),
                session_key:            session,
                event_kinds:            vec![FinanceEventKind::MarketCandleClosed],
                source_names:           Vec::new(),
                category_tags:          Vec::new(),
                watch_terms:            Vec::new(),
                venues:                 vec!["binance".to_owned()],
                symbols:                vec!["BTCUSDT".to_owned()],
                timeframes:             vec!["15m".to_owned()],
                delivery:               FinanceDelivery::Silent,
                cooldown_secs:          900,
                max_immediate_per_hour: 6,
            })
            .await
            .unwrap();

        assert_eq!(
            registry
                .match_event(
                    &candle_with_payload(" Binance ", " btcusdt ", " 15M "),
                    None
                )
                .await
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn values_inside_filter_groups_are_ored() {
        let (registry, _clock, _tmp) = registry(ts("2026-07-10T08:31:00Z"));
        let session = SessionKey::new();
        registry
            .upsert(FinanceSubscription {
                id:                     uuid::Uuid::new_v4(),
                owner:                  owner(),
                session_key:            session,
                event_kinds:            vec![FinanceEventKind::RssArticle],
                source_names:           vec!["fed-news".to_owned(), "sec-news".to_owned()],
                category_tags:          Vec::new(),
                watch_terms:            vec!["BTC".to_owned(), "NVDA".to_owned()],
                venues:                 Vec::new(),
                symbols:                Vec::new(),
                timeframes:             Vec::new(),
                delivery:               FinanceDelivery::Silent,
                cooldown_secs:          900,
                max_immediate_per_hour: 6,
            })
            .await
            .unwrap();

        assert_eq!(
            registry
                .match_event(&article("sec-news", "NVDA filing update", ""), None)
                .await
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn duplicate_event_is_delivered_once_after_reload() {
        let tmp = tempfile::tempdir().expect("tempdir should be created");
        let path = tmp.path().join("subs.json");
        let session = SessionKey::new();
        let clock = Arc::new(ManualFinanceClock::new(ts("2026-07-10T08:31:00Z")));
        let registry = FinanceSubscriptionRegistry::load_with_clock(path.clone(), clock.clone());
        registry.upsert(sub(session)).await.unwrap();

        let event = article("fed-news", "BTC first notice", "");
        assert_eq!(registry.match_event(&event, None).await.len(), 1);

        let reloaded = FinanceSubscriptionRegistry::load_with_clock(path, clock);
        assert!(reloaded.match_event(&event, None).await.is_empty());
    }

    #[tokio::test]
    async fn immediate_budget_downgrades_excess_events_to_silent() {
        let (registry, clock, _tmp) = registry(ts("2026-07-10T08:00:00Z"));
        let session = SessionKey::new();
        let mut subscription = sub(session);
        subscription.max_immediate_per_hour = 1;
        subscription.cooldown_secs = 0;
        registry.upsert(subscription).await.unwrap();

        let first = registry
            .match_event(&article("fed-news", "BTC first", ""), None)
            .await;
        assert_eq!(first[0].action, FinanceDeliveryAction::Immediate);

        clock.advance(SignedDuration::from_secs(60));
        let second = registry
            .match_event(&article("fed-news", "BTC second", ""), None)
            .await;
        assert_eq!(second[0].action, FinanceDeliveryAction::Silent);
        assert_eq!(
            second[0].downgraded_reason.as_deref(),
            Some("hourly_budget")
        );
    }

    #[tokio::test]
    async fn default_immediate_budget_allows_sixth_and_silences_seventh() {
        let (registry, clock, _tmp) = registry(ts("2026-07-10T08:00:00Z"));
        let session = SessionKey::new();
        let mut subscription = sub(session);
        subscription.cooldown_secs = 0;
        registry.upsert(subscription).await.unwrap();

        for index in 1..=6 {
            let matched = registry
                .match_event(
                    &article("fed-news", &format!("BTC notice {index}"), ""),
                    None,
                )
                .await;
            assert_eq!(matched.len(), 1);
            assert_eq!(
                matched[0].action,
                FinanceDeliveryAction::Immediate,
                "event {index} should remain immediate under the default hourly budget"
            );
            assert_eq!(matched[0].downgraded_reason, None);
            clock.advance(SignedDuration::from_secs(60));
        }

        let seventh = registry
            .match_event(&article("fed-news", "BTC notice 7", ""), None)
            .await;
        assert_eq!(seventh.len(), 1);
        assert_eq!(seventh[0].action, FinanceDeliveryAction::Silent);
        assert_eq!(
            seventh[0].downgraded_reason.as_deref(),
            Some("hourly_budget")
        );
    }

    #[tokio::test]
    async fn high_severity_bypasses_cooldown() {
        let (registry, clock, _tmp) = registry(ts("2026-07-10T08:00:00Z"));
        let session = SessionKey::new();
        // Default sub: cooldown 900s, budget 6.
        registry.upsert(sub(session)).await.unwrap();

        // First immediate delivery seeds the cooldown window.
        let first = registry
            .match_event(&article("fed-news", "BTC first", ""), None)
            .await;
        assert_eq!(first[0].action, FinanceDeliveryAction::Immediate);

        // A second matched event 60s later sits well inside the 900s cooldown,
        // but a critical severity must override it.
        clock.advance(SignedDuration::from_secs(60));
        let bypass = registry
            .match_event(
                &article("fed-news", "BTC crash", ""),
                Some(Severity::Critical),
            )
            .await;
        assert_eq!(bypass[0].action, FinanceDeliveryAction::Immediate);
        assert_eq!(bypass[0].bypass_reason.as_deref(), Some("cooldown"));
        assert_eq!(bypass[0].downgraded_reason, None);
    }

    #[tokio::test]
    async fn high_severity_bypasses_hourly_budget() {
        let (registry, clock, _tmp) = registry(ts("2026-07-10T08:00:00Z"));
        let session = SessionKey::new();
        let mut subscription = sub(session);
        subscription.max_immediate_per_hour = 1;
        subscription.cooldown_secs = 0;
        registry.upsert(subscription).await.unwrap();

        // Spend the single-event budget.
        let first = registry
            .match_event(&article("fed-news", "BTC first", ""), None)
            .await;
        assert_eq!(first[0].action, FinanceDeliveryAction::Immediate);

        // The budget is now exhausted, but a critical severity bypasses it
        // instead of taking the hourly_budget downgrade.
        clock.advance(SignedDuration::from_secs(60));
        let bypass = registry
            .match_event(
                &article("fed-news", "BTC crash", ""),
                Some(Severity::Critical),
            )
            .await;
        assert_eq!(bypass[0].action, FinanceDeliveryAction::Immediate);
        assert_eq!(bypass[0].bypass_reason.as_deref(), Some("hourly_budget"));
        assert_eq!(bypass[0].downgraded_reason, None);
    }

    #[tokio::test]
    async fn low_severity_still_downgrades_under_budget() {
        let (registry, clock, _tmp) = registry(ts("2026-07-10T08:00:00Z"));
        let session = SessionKey::new();
        let mut subscription = sub(session);
        subscription.max_immediate_per_hour = 1;
        subscription.cooldown_secs = 0;
        registry.upsert(subscription).await.unwrap();

        // Spend the single-event budget.
        let first = registry
            .match_event(&article("fed-news", "BTC first", ""), None)
            .await;
        assert_eq!(first[0].action, FinanceDeliveryAction::Immediate);

        // A below-threshold severity stays on the routine lane: silenced with
        // the unchanged hourly_budget reason, no bypass.
        clock.advance(SignedDuration::from_secs(60));
        let routine = registry
            .match_event(&article("fed-news", "BTC drift", ""), Some(Severity::Watch))
            .await;
        assert_eq!(routine[0].action, FinanceDeliveryAction::Silent);
        assert_eq!(
            routine[0].downgraded_reason.as_deref(),
            Some("hourly_budget")
        );
        assert_eq!(routine[0].bypass_reason, None);
    }
}
