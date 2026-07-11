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

use super::model::{CandleRangeQuery, MarketCandle};

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

    async fn missing_open_times(&self, query: CandleRangeQuery) -> anyhow::Result<Vec<Timestamp>> {
        let rows = self.candles(query.clone()).await?;
        let present: std::collections::HashSet<Timestamp> =
            rows.into_iter().map(|row| row.open_time).collect();
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

#[cfg(test)]
mod tests {
    use rust_decimal::Decimal;

    use super::{InMemoryMarketDataRepository, MarketDataRepository, UpsertOutcome};
    use crate::market_data::{CandleRangeQuery, MarketCandle, Timeframe};

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
}
