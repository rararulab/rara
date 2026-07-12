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

use std::{collections::BTreeMap, sync::Arc};

use async_trait::async_trait;
use jiff::Timestamp;
use tokio::sync::RwLock;

use super::model::{
    CandleLatestQuery, CandleRangeQuery, CandleRecentQuery, CandleStreamListQuery,
    CandleStreamSummary, MarketCandle,
};

/// Result of upserting a closed candle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpsertOutcome {
    /// No candle existed for this primary key.
    Inserted,
    /// Existing row was equivalent for current candle values.
    DuplicateUnchanged,
    /// Existing current row differed and was replaced, with an audit record.
    Corrected,
}

/// Repository abstraction for durable market-data history.
#[async_trait]
pub trait MarketDataRepository: Send + Sync {
    /// Upsert a closed candle by `(source, venue, symbol, timeframe,
    /// open_time)`.
    async fn upsert_closed_candle(&self, candle: MarketCandle) -> anyhow::Result<UpsertOutcome>;

    /// Query ordered candles for a venue/symbol/timeframe range.
    async fn candles(&self, query: CandleRangeQuery) -> anyhow::Result<Vec<MarketCandle>>;

    /// Return the newest closed candle for a venue/symbol/timeframe stream.
    async fn latest_closed_candle(
        &self,
        query: CandleLatestQuery,
    ) -> anyhow::Result<Option<MarketCandle>>;

    /// Return the newest closed candles for a stream, ordered by open time.
    async fn recent_candles(&self, query: CandleRecentQuery) -> anyhow::Result<Vec<MarketCandle>>;

    /// List stored candle streams with their latest watermarks.
    async fn candle_streams(
        &self,
        query: CandleStreamListQuery,
    ) -> anyhow::Result<Vec<CandleStreamSummary>>;

    /// Return missing open times in `[start, end)` based on the query
    /// timeframe.
    async fn missing_open_times(&self, query: CandleRangeQuery) -> anyhow::Result<Vec<Timestamp>>;
}

/// Shared market-data repository reference.
pub type MarketDataRepositoryRef = Arc<dyn MarketDataRepository>;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct CandleKey {
    source_name: String,
    venue:       String,
    symbol:      String,
    timeframe:   String,
    open_time:   Timestamp,
}

impl From<&MarketCandle> for CandleKey {
    fn from(candle: &MarketCandle) -> Self {
        Self {
            source_name: candle.source_name.clone(),
            venue:       candle.venue.clone(),
            symbol:      candle.symbol.clone(),
            timeframe:   candle.timeframe.to_string(),
            open_time:   candle.open_time,
        }
    }
}

#[derive(Debug, Clone)]
struct CandleCorrection {
    previous:     serde_json::Value,
    new:          serde_json::Value,
    corrected_at: Timestamp,
}

/// In-memory repository used by tests and local no-TSDB operation.
#[derive(Debug, Default)]
pub struct InMemoryMarketDataRepository {
    candles:     RwLock<BTreeMap<CandleKey, MarketCandle>>,
    corrections: RwLock<Vec<CandleCorrection>>,
}

impl InMemoryMarketDataRepository {
    /// Number of recorded correction audit rows.
    pub async fn correction_count(&self) -> usize {
        let corrections = self.corrections.read().await;
        let _payload_bytes: usize = corrections
            .iter()
            .map(|row| {
                row.previous.to_string().len()
                    + row.new.to_string().len()
                    + row.corrected_at.to_string().len()
            })
            .sum();
        corrections.len()
    }
}

#[async_trait]
impl MarketDataRepository for InMemoryMarketDataRepository {
    async fn upsert_closed_candle(&self, candle: MarketCandle) -> anyhow::Result<UpsertOutcome> {
        let key = CandleKey::from(&candle);
        let mut candles = self.candles.write().await;

        let outcome = match candles.get(&key) {
            None => {
                candles.insert(key, candle);
                UpsertOutcome::Inserted
            }
            Some(existing) if existing.same_current_values(&candle) => {
                UpsertOutcome::DuplicateUnchanged
            }
            Some(existing) => {
                let correction = CandleCorrection {
                    previous:     existing.audit_payload(),
                    new:          candle.audit_payload(),
                    corrected_at: Timestamp::now(),
                };
                candles.insert(key, candle);
                drop(candles);
                self.corrections.write().await.push(correction);
                return Ok(UpsertOutcome::Corrected);
            }
        };

        Ok(outcome)
    }

    async fn candles(&self, query: CandleRangeQuery) -> anyhow::Result<Vec<MarketCandle>> {
        let candles = self.candles.read().await;
        let mut rows: Vec<MarketCandle> = candles
            .values()
            .filter(|candle| matches_query(candle, &query))
            .cloned()
            .collect();
        rows.sort_by_key(|candle| candle.open_time);
        rows.truncate(query.limit.min(10_000));
        Ok(rows)
    }

    async fn latest_closed_candle(
        &self,
        query: CandleLatestQuery,
    ) -> anyhow::Result<Option<MarketCandle>> {
        let candles = self.candles.read().await;
        Ok(candles
            .values()
            .filter(|candle| matches_latest_query(candle, &query))
            .max_by_key(|candle| candle.open_time)
            .cloned())
    }

    async fn recent_candles(&self, query: CandleRecentQuery) -> anyhow::Result<Vec<MarketCandle>> {
        let candles = self.candles.read().await;
        let mut rows: Vec<MarketCandle> = candles
            .values()
            .filter(|candle| matches_recent_query(candle, &query))
            .cloned()
            .collect();
        rows.sort_by_key(|candle| std::cmp::Reverse(candle.open_time));
        rows.truncate(query.limit.min(10_000));
        rows.sort_by_key(|candle| candle.open_time);
        Ok(rows)
    }

    async fn candle_streams(
        &self,
        query: CandleStreamListQuery,
    ) -> anyhow::Result<Vec<CandleStreamSummary>> {
        let candles = self.candles.read().await;
        let mut streams = BTreeMap::<StreamKey, CandleStreamSummary>::new();

        for candle in candles
            .values()
            .filter(|candle| matches_stream_query(candle, &query))
        {
            let key = StreamKey::from(candle);
            streams
                .entry(key)
                .and_modify(|summary| update_stream_summary(summary, candle))
                .or_insert_with(|| CandleStreamSummary {
                    source_name:        candle.source_name.clone(),
                    venue:              candle.venue.clone(),
                    symbol:             candle.symbol.clone(),
                    timeframe:          candle.timeframe.clone(),
                    candle_count:       1,
                    first_open_time:    candle.open_time,
                    latest_open_time:   candle.open_time,
                    latest_close_time:  candle.close_time,
                    latest_ingested_at: candle.ingested_at,
                });
        }

        let mut rows: Vec<_> = streams.into_values().collect();
        rows.sort_by(|left, right| {
            right
                .latest_open_time
                .cmp(&left.latest_open_time)
                .then_with(|| left.venue.cmp(&right.venue))
                .then_with(|| left.symbol.cmp(&right.symbol))
                .then_with(|| left.timeframe.cmp(&right.timeframe))
                .then_with(|| left.source_name.cmp(&right.source_name))
        });
        rows.truncate(query.limit.min(10_000));
        Ok(rows)
    }

    async fn missing_open_times(&self, query: CandleRangeQuery) -> anyhow::Result<Vec<Timestamp>> {
        let candles = self.candles.read().await;
        let present: std::collections::HashSet<Timestamp> = candles
            .values()
            .filter(|candle| matches_query(candle, &query))
            .map(|row| row.open_time)
            .collect();
        let step = query.timeframe.step()?;
        let mut missing = Vec::new();
        let mut cursor = query.start;

        while cursor < query.end {
            if !present.contains(&cursor) {
                missing.push(cursor);
            }
            cursor = cursor
                .checked_add(step)
                .map_err(|err| anyhow::anyhow!("timeframe addition overflowed: {err}"))?;
        }

        Ok(missing)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct StreamKey {
    source_name: String,
    venue:       String,
    symbol:      String,
    timeframe:   String,
}

impl From<&MarketCandle> for StreamKey {
    fn from(candle: &MarketCandle) -> Self {
        Self {
            source_name: candle.source_name.clone(),
            venue:       candle.venue.clone(),
            symbol:      candle.symbol.clone(),
            timeframe:   candle.timeframe.to_string(),
        }
    }
}

fn update_stream_summary(summary: &mut CandleStreamSummary, candle: &MarketCandle) {
    summary.candle_count = summary.candle_count.saturating_add(1);
    summary.first_open_time = summary.first_open_time.min(candle.open_time);
    if candle.open_time > summary.latest_open_time
        || (candle.open_time == summary.latest_open_time
            && candle.ingested_at > summary.latest_ingested_at)
    {
        summary.latest_open_time = candle.open_time;
        summary.latest_close_time = candle.close_time;
        summary.latest_ingested_at = candle.ingested_at;
    }
}

fn matches_query(candle: &MarketCandle, query: &CandleRangeQuery) -> bool {
    query
        .source_name
        .as_ref()
        .is_none_or(|source| source == &candle.source_name)
        && candle.venue == query.venue
        && candle.symbol == query.symbol
        && candle.timeframe == query.timeframe
        && candle.open_time >= query.start
        && candle.open_time < query.end
}

fn matches_latest_query(candle: &MarketCandle, query: &CandleLatestQuery) -> bool {
    query
        .source_name
        .as_ref()
        .is_none_or(|source| source == &candle.source_name)
        && candle.venue == query.venue
        && candle.symbol == query.symbol
        && candle.timeframe == query.timeframe
}

fn matches_recent_query(candle: &MarketCandle, query: &CandleRecentQuery) -> bool {
    query
        .source_name
        .as_ref()
        .is_none_or(|source| source == &candle.source_name)
        && candle.venue == query.venue
        && candle.symbol == query.symbol
        && candle.timeframe == query.timeframe
}

fn matches_stream_query(candle: &MarketCandle, query: &CandleStreamListQuery) -> bool {
    query
        .source_name
        .as_ref()
        .is_none_or(|source| source == &candle.source_name)
        && query
            .venue
            .as_ref()
            .is_none_or(|venue| venue == &candle.venue)
        && query
            .symbol
            .as_ref()
            .is_none_or(|symbol| symbol == &candle.symbol)
        && query
            .timeframe
            .as_ref()
            .is_none_or(|timeframe| timeframe == &candle.timeframe)
}

#[cfg(test)]
mod tests {
    use rust_decimal::Decimal;

    use super::{InMemoryMarketDataRepository, MarketDataRepository, UpsertOutcome};
    use crate::market_data::{
        CandleLatestQuery, CandleRangeQuery, CandleRecentQuery, CandleStreamListQuery,
        MarketCandle, Timeframe,
    };

    fn ts(value: &str) -> jiff::Timestamp { value.parse().expect("timestamp fixture should parse") }

    fn dec(value: &str) -> Decimal { value.parse().expect("decimal fixture should parse") }

    fn candle(open_time: &str, close: &str) -> MarketCandle {
        MarketCandle {
            source_name:       "binance-spot".to_owned(),
            venue:             "binance".to_owned(),
            symbol:            "BTCUSDT".to_owned(),
            timeframe:         Timeframe::parse("15m").expect("timeframe fixture should parse"),
            open_time:         ts(open_time),
            close_time:        ts("2026-07-10T08:30:00Z"),
            open:              dec("61500.12"),
            high:              dec("61640.00"),
            low:               dec("61480.50"),
            close:             dec(close),
            volume:            dec("124.551"),
            ingested_at:       ts("2026-07-10T08:30:01Z"),
            provider_sequence: None,
        }
    }

    fn query() -> CandleRangeQuery {
        CandleRangeQuery {
            source_name: Some("binance-spot".to_owned()),
            venue:       "binance".to_owned(),
            symbol:      "BTCUSDT".to_owned(),
            timeframe:   Timeframe::parse("15m").expect("timeframe fixture should parse"),
            start:       ts("2026-07-10T08:00:00Z"),
            end:         ts("2026-07-10T08:45:00Z"),
            limit:       100,
        }
    }

    #[tokio::test]
    async fn upsert_candle_is_idempotent_for_same_primary_key() {
        let repo = InMemoryMarketDataRepository::default();
        let candle = candle("2026-07-10T08:15:00Z", "61610.30");

        assert_eq!(
            repo.upsert_closed_candle(candle.clone()).await.unwrap(),
            UpsertOutcome::Inserted
        );
        assert_eq!(
            repo.upsert_closed_candle(candle).await.unwrap(),
            UpsertOutcome::DuplicateUnchanged
        );
        assert_eq!(repo.candles(query()).await.unwrap().len(), 1);
        assert_eq!(repo.correction_count().await, 0);
    }

    #[tokio::test]
    async fn corrected_candle_updates_current_row_and_records_audit() {
        let repo = InMemoryMarketDataRepository::default();
        repo.upsert_closed_candle(candle("2026-07-10T08:15:00Z", "61610.30"))
            .await
            .unwrap();

        assert_eq!(
            repo.upsert_closed_candle(candle("2026-07-10T08:15:00Z", "61611.00"))
                .await
                .unwrap(),
            UpsertOutcome::Corrected
        );

        let rows = repo.candles(query()).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].close, dec("61611.00"));
        assert_eq!(repo.correction_count().await, 1);
    }

    #[tokio::test]
    async fn query_range_returns_ordered_candles_for_symbol_timeframe() {
        let repo = InMemoryMarketDataRepository::default();
        repo.upsert_closed_candle(candle("2026-07-10T08:30:00Z", "61700.00"))
            .await
            .unwrap();
        repo.upsert_closed_candle(candle("2026-07-10T08:00:00Z", "61500.00"))
            .await
            .unwrap();
        repo.upsert_closed_candle(candle("2026-07-10T08:15:00Z", "61610.30"))
            .await
            .unwrap();

        let rows = repo.candles(query()).await.unwrap();
        assert_eq!(
            rows.iter()
                .map(|row| row.open_time.to_string())
                .collect::<Vec<_>>(),
            vec![
                "2026-07-10T08:00:00Z",
                "2026-07-10T08:15:00Z",
                "2026-07-10T08:30:00Z",
            ]
        );
    }

    #[tokio::test]
    async fn gap_detection_reports_missing_open_times() {
        let repo = InMemoryMarketDataRepository::default();
        repo.upsert_closed_candle(candle("2026-07-10T08:00:00Z", "61500.00"))
            .await
            .unwrap();
        repo.upsert_closed_candle(candle("2026-07-10T08:30:00Z", "61700.00"))
            .await
            .unwrap();

        let missing = repo.missing_open_times(query()).await.unwrap();
        assert_eq!(missing, vec![ts("2026-07-10T08:15:00Z")]);
    }

    #[tokio::test]
    async fn gap_detection_ignores_range_query_limit() {
        let repo = InMemoryMarketDataRepository::default();
        repo.upsert_closed_candle(candle("2026-07-10T08:00:00Z", "61500.00"))
            .await
            .unwrap();
        repo.upsert_closed_candle(candle("2026-07-10T08:15:00Z", "61610.30"))
            .await
            .unwrap();
        repo.upsert_closed_candle(candle("2026-07-10T08:30:00Z", "61700.00"))
            .await
            .unwrap();

        let mut query = query();
        query.limit = 1;

        let missing = repo.missing_open_times(query).await.unwrap();
        assert!(missing.is_empty());
    }

    #[tokio::test]
    async fn latest_closed_candle_returns_newest_matching_stream() {
        let repo = InMemoryMarketDataRepository::default();
        repo.upsert_closed_candle(candle("2026-07-10T08:00:00Z", "61500.00"))
            .await
            .unwrap();
        repo.upsert_closed_candle(candle("2026-07-10T08:30:00Z", "61700.00"))
            .await
            .unwrap();
        repo.upsert_closed_candle(MarketCandle {
            source_name: "other-source".to_owned(),
            ..candle("2026-07-10T08:45:00Z", "61800.00")
        })
        .await
        .unwrap();

        let latest = repo
            .latest_closed_candle(CandleLatestQuery {
                source_name: Some("binance-spot".to_owned()),
                venue:       "binance".to_owned(),
                symbol:      "BTCUSDT".to_owned(),
                timeframe:   Timeframe::parse("15m").unwrap(),
            })
            .await
            .unwrap()
            .expect("matching candle should exist");

        assert_eq!(latest.open_time, ts("2026-07-10T08:30:00Z"));
        assert_eq!(latest.close, dec("61700.00"));
    }

    #[tokio::test]
    async fn recent_candles_returns_latest_n_in_ascending_order() {
        let repo = InMemoryMarketDataRepository::default();
        for (open_time, close) in [
            ("2026-07-10T08:00:00Z", "61500.00"),
            ("2026-07-10T08:15:00Z", "61610.30"),
            ("2026-07-10T08:30:00Z", "61700.00"),
        ] {
            repo.upsert_closed_candle(candle(open_time, close))
                .await
                .unwrap();
        }
        repo.upsert_closed_candle(MarketCandle {
            source_name: "other-source".to_owned(),
            ..candle("2026-07-10T08:45:00Z", "61800.00")
        })
        .await
        .unwrap();

        let rows = repo
            .recent_candles(CandleRecentQuery {
                source_name: Some("binance-spot".to_owned()),
                venue:       "binance".to_owned(),
                symbol:      "BTCUSDT".to_owned(),
                timeframe:   Timeframe::parse("15m").unwrap(),
                limit:       2,
            })
            .await
            .unwrap();

        assert_eq!(
            rows.iter()
                .map(|row| row.open_time.to_string())
                .collect::<Vec<_>>(),
            vec!["2026-07-10T08:15:00Z", "2026-07-10T08:30:00Z"]
        );
    }

    #[tokio::test]
    async fn candle_streams_returns_latest_watermarks_per_stream() {
        let repo = InMemoryMarketDataRepository::default();
        repo.upsert_closed_candle(candle("2026-07-10T08:00:00Z", "61500.00"))
            .await
            .unwrap();
        repo.upsert_closed_candle(candle("2026-07-10T08:30:00Z", "61700.00"))
            .await
            .unwrap();
        repo.upsert_closed_candle(MarketCandle {
            symbol: "ETHUSDT".to_owned(),
            open_time: ts("2026-07-10T08:45:00Z"),
            close_time: ts("2026-07-10T09:00:00Z"),
            close: dec("3200.00"),
            ..candle("2026-07-10T08:45:00Z", "3200.00")
        })
        .await
        .unwrap();

        let streams = repo
            .candle_streams(CandleStreamListQuery {
                source_name: Some("binance-spot".to_owned()),
                venue:       Some("binance".to_owned()),
                symbol:      None,
                timeframe:   Some(Timeframe::parse("15m").unwrap()),
                limit:       10,
            })
            .await
            .unwrap();

        assert_eq!(streams.len(), 2);
        assert_eq!(streams[0].symbol, "ETHUSDT");
        assert_eq!(streams[0].candle_count, 1);
        assert_eq!(streams[0].latest_open_time, ts("2026-07-10T08:45:00Z"));
        assert_eq!(streams[0].latest_close_time, ts("2026-07-10T09:00:00Z"));
        assert_eq!(streams[1].symbol, "BTCUSDT");
        assert_eq!(streams[1].candle_count, 2);
        assert_eq!(streams[1].first_open_time, ts("2026-07-10T08:00:00Z"));
        assert_eq!(streams[1].latest_open_time, ts("2026-07-10T08:30:00Z"));
    }
}
