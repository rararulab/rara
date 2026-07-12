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

use std::{fmt, str::FromStr};

use jiff::{SignedDuration, Timestamp};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// Bar timeframe such as `15m`, `1h`, or `1d`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Timeframe(String);

impl Timeframe {
    /// Parse and validate a bar timeframe.
    pub fn parse(value: impl AsRef<str>) -> anyhow::Result<Self> {
        let value = value.as_ref().trim();
        anyhow::ensure!(!value.is_empty(), "timeframe must not be empty");
        anyhow::ensure!(value.len() >= 2, "timeframe must include amount and unit");
        let (number, unit) = value.split_at(value.len() - 1);
        let amount: i64 = number.parse()?;
        anyhow::ensure!(amount > 0, "timeframe amount must be positive");
        anyhow::ensure!(
            matches!(unit, "s" | "m" | "h" | "d"),
            "timeframe unit must be one of s, m, h, d"
        );
        Ok(Self(format!("{amount}{unit}")))
    }

    /// Return the canonical string representation.
    #[must_use]
    pub fn as_str(&self) -> &str { &self.0 }

    /// Return the timeframe step as whole seconds.
    pub fn step(&self) -> anyhow::Result<SignedDuration> {
        let (number, unit) = self.0.split_at(self.0.len() - 1);
        let amount: i64 = number.parse()?;
        let seconds = match unit {
            "s" => amount,
            "m" => amount.saturating_mul(60),
            "h" => amount.saturating_mul(60 * 60),
            "d" => amount.saturating_mul(24 * 60 * 60),
            _ => anyhow::bail!("invalid timeframe unit"),
        };
        Ok(SignedDuration::from_secs(seconds))
    }
}

impl fmt::Display for Timeframe {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(&self.0) }
}

impl FromStr for Timeframe {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> { Self::parse(s) }
}

/// Durable closed OHLCV candle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarketCandle {
    pub source_name:       String,
    pub venue:             String,
    pub symbol:            String,
    pub timeframe:         Timeframe,
    pub open_time:         Timestamp,
    pub close_time:        Timestamp,
    pub open:              Decimal,
    pub high:              Decimal,
    pub low:               Decimal,
    pub close:             Decimal,
    pub volume:            Decimal,
    pub ingested_at:       Timestamp,
    pub provider_sequence: Option<String>,
}

impl MarketCandle {
    /// Return true when the current-row values are identical.
    #[must_use]
    pub fn same_current_values(&self, other: &Self) -> bool {
        self.close_time == other.close_time
            && self.open == other.open
            && self.high == other.high
            && self.low == other.low
            && self.close == other.close
            && self.volume == other.volume
            && self.provider_sequence == other.provider_sequence
    }

    /// JSON payload used for correction/audit rows.
    #[must_use]
    pub fn audit_payload(&self) -> serde_json::Value {
        serde_json::json!({
            "source_name": self.source_name,
            "venue": self.venue,
            "symbol": self.symbol,
            "timeframe": self.timeframe.as_str(),
            "open_time": self.open_time.to_string(),
            "close_time": self.close_time.to_string(),
            "open": self.open.to_string(),
            "high": self.high.to_string(),
            "low": self.low.to_string(),
            "close": self.close.to_string(),
            "volume": self.volume.to_string(),
            "ingested_at": self.ingested_at.to_string(),
            "provider_sequence": self.provider_sequence,
        })
    }
}

/// Range query for historical candles.
#[derive(Debug, Clone)]
pub struct CandleRangeQuery {
    pub source_name: Option<String>,
    pub venue:       String,
    pub symbol:      String,
    pub timeframe:   Timeframe,
    /// Inclusive start open time.
    pub start:       Timestamp,
    /// Exclusive end open time.
    pub end:         Timestamp,
    pub limit:       usize,
}

/// Latest-candle query for a venue/symbol/timeframe stream.
#[derive(Debug, Clone)]
pub struct CandleLatestQuery {
    pub source_name: Option<String>,
    pub venue:       String,
    pub symbol:      String,
    pub timeframe:   Timeframe,
}

/// Recent-candle query for the newest closed candles in a stream.
#[derive(Debug, Clone)]
pub struct CandleRecentQuery {
    pub source_name: Option<String>,
    pub venue:       String,
    pub symbol:      String,
    pub timeframe:   Timeframe,
    pub limit:       usize,
}

/// Stream inventory query for stored candle data.
#[derive(Debug, Clone)]
pub struct CandleStreamListQuery {
    pub source_name: Option<String>,
    pub venue:       Option<String>,
    pub symbol:      Option<String>,
    pub timeframe:   Option<Timeframe>,
    pub limit:       usize,
}

/// Aggregated summary for one `(source, venue, symbol, timeframe)` candle
/// stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandleStreamSummary {
    pub source_name:        String,
    pub venue:              String,
    pub symbol:             String,
    pub timeframe:          Timeframe,
    pub candle_count:       usize,
    pub first_open_time:    Timestamp,
    pub latest_open_time:   Timestamp,
    pub latest_close_time:  Timestamp,
    pub latest_ingested_at: Timestamp,
}
