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
use jiff::Timestamp;
use rust_decimal::Decimal;
use sqlx_core::row::Row;
use sqlx_postgres::{PgPool, Postgres};
use uuid::Uuid;

use super::{
    model::{
        CandleLatestQuery, CandleRangeQuery, CandleStreamListQuery, CandleStreamSummary,
        MarketCandle, Timeframe,
    },
    repository::{MarketDataRepository, UpsertOutcome},
};

/// TimescaleDB/PostgreSQL-backed market-data repository.
#[derive(Debug, Clone)]
pub struct TimescaleMarketDataRepository {
    pool: PgPool,
}

impl TimescaleMarketDataRepository {
    /// Connect to PostgreSQL/TimescaleDB.
    pub async fn connect(database_url: &str) -> anyhow::Result<Self> {
        let pool = PgPool::connect(database_url).await?;
        Ok(Self { pool })
    }

    /// Construct from an existing pool.
    #[must_use]
    pub fn new(pool: PgPool) -> Self { Self { pool } }

    /// Apply the MVP schema. Operators can also run the SQL migration directly.
    pub async fn apply_schema(&self) -> anyhow::Result<()> {
        for statement in include_str!("../../migrations/0001_market_candles.sql")
            .split(';')
            .map(str::trim)
            .filter(|statement| !statement.is_empty())
        {
            sqlx_core::query::query(statement)
                .execute(&self.pool)
                .await?;
        }
        Ok(())
    }
}

#[async_trait]
impl MarketDataRepository for TimescaleMarketDataRepository {
    async fn upsert_closed_candle(&self, candle: MarketCandle) -> anyhow::Result<UpsertOutcome> {
        let mut tx = self.pool.begin().await?;
        let existing = find_existing(&mut tx, &candle).await?;

        let outcome = match existing {
            None => {
                insert_candle(&mut tx, &candle).await?;
                UpsertOutcome::Inserted
            }
            Some(existing) if existing.same_current_values(&candle) => {
                UpsertOutcome::DuplicateUnchanged
            }
            Some(existing) => {
                sqlx_core::query::query(
                    r#"
                    INSERT INTO market_candle_corrections (
                      id, source_name, venue, symbol, timeframe, open_time,
                      corrected_at, previous_payload, new_payload
                    )
                    VALUES (
                      $1, $2, $3, $4, $5, $6::timestamptz,
                      $7::timestamptz, $8::jsonb, $9::jsonb
                    )
                    "#,
                )
                .bind(Uuid::new_v4())
                .bind(&candle.source_name)
                .bind(&candle.venue)
                .bind(&candle.symbol)
                .bind(candle.timeframe.as_str())
                .bind(candle.open_time.to_string())
                .bind(Timestamp::now().to_string())
                .bind(existing.audit_payload().to_string())
                .bind(candle.audit_payload().to_string())
                .execute(&mut *tx)
                .await?;

                update_candle(&mut tx, &candle).await?;
                UpsertOutcome::Corrected
            }
        };

        tx.commit().await?;
        Ok(outcome)
    }

    async fn candles(&self, query: CandleRangeQuery) -> anyhow::Result<Vec<MarketCandle>> {
        let limit = i64::try_from(query.limit.min(10_000))?;
        let rows = sqlx_core::query::query(
            r#"
            SELECT
              source_name, venue, symbol, timeframe,
              open_time::text AS open_time,
              close_time::text AS close_time,
              open::text AS open,
              high::text AS high,
              low::text AS low,
              close::text AS close,
              volume::text AS volume,
              ingested_at::text AS ingested_at,
              provider_sequence
            FROM market_candles
            WHERE ($1::text IS NULL OR source_name = $1)
              AND venue = $2
              AND symbol = $3
              AND timeframe = $4
              AND open_time >= $5::timestamptz
              AND open_time < $6::timestamptz
            ORDER BY open_time ASC
            LIMIT $7
            "#,
        )
        .bind(query.source_name.as_deref())
        .bind(&query.venue)
        .bind(&query.symbol)
        .bind(query.timeframe.as_str())
        .bind(query.start.to_string())
        .bind(query.end.to_string())
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(row_to_candle).collect()
    }

    async fn latest_closed_candle(
        &self,
        query: CandleLatestQuery,
    ) -> anyhow::Result<Option<MarketCandle>> {
        let row = sqlx_core::query::query(
            r#"
            SELECT
              source_name, venue, symbol, timeframe,
              open_time::text AS open_time,
              close_time::text AS close_time,
              open::text AS open,
              high::text AS high,
              low::text AS low,
              close::text AS close,
              volume::text AS volume,
              ingested_at::text AS ingested_at,
              provider_sequence
            FROM market_candles
            WHERE ($1::text IS NULL OR source_name = $1)
              AND venue = $2
              AND symbol = $3
              AND timeframe = $4
            ORDER BY open_time DESC
            LIMIT 1
            "#,
        )
        .bind(query.source_name.as_deref())
        .bind(&query.venue)
        .bind(&query.symbol)
        .bind(query.timeframe.as_str())
        .fetch_optional(&self.pool)
        .await?;

        row.map(row_to_candle).transpose()
    }

    async fn candle_streams(
        &self,
        query: CandleStreamListQuery,
    ) -> anyhow::Result<Vec<CandleStreamSummary>> {
        let limit = i64::try_from(query.limit.min(10_000))?;
        let rows = sqlx_core::query::query(
            r#"
            WITH grouped AS (
              SELECT
                source_name,
                venue,
                symbol,
                timeframe,
                COUNT(*)::bigint AS candle_count,
                MIN(open_time)::text AS first_open_time,
                MAX(open_time)::text AS latest_open_time
              FROM market_candles
              WHERE ($1::text IS NULL OR source_name = $1)
                AND ($2::text IS NULL OR venue = $2)
                AND ($3::text IS NULL OR symbol = $3)
                AND ($4::text IS NULL OR timeframe = $4)
              GROUP BY source_name, venue, symbol, timeframe
            )
            SELECT
              grouped.source_name,
              grouped.venue,
              grouped.symbol,
              grouped.timeframe,
              grouped.candle_count,
              grouped.first_open_time,
              grouped.latest_open_time,
              candles.close_time::text AS latest_close_time,
              candles.ingested_at::text AS latest_ingested_at
            FROM grouped
            JOIN market_candles candles
              ON candles.source_name = grouped.source_name
             AND candles.venue = grouped.venue
             AND candles.symbol = grouped.symbol
             AND candles.timeframe = grouped.timeframe
             AND candles.open_time = grouped.latest_open_time::timestamptz
            ORDER BY grouped.latest_open_time::timestamptz DESC,
                     grouped.venue ASC,
                     grouped.symbol ASC,
                     grouped.timeframe ASC,
                     grouped.source_name ASC
            LIMIT $5
            "#,
        )
        .bind(query.source_name.as_deref())
        .bind(query.venue.as_deref())
        .bind(query.symbol.as_deref())
        .bind(query.timeframe.as_ref().map(Timeframe::as_str))
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(row_to_stream_summary).collect()
    }

    async fn missing_open_times(&self, query: CandleRangeQuery) -> anyhow::Result<Vec<Timestamp>> {
        let rows = sqlx_core::query::query(
            r#"
            SELECT open_time::text AS open_time
            FROM market_candles
            WHERE ($1::text IS NULL OR source_name = $1)
              AND venue = $2
              AND symbol = $3
              AND timeframe = $4
              AND open_time >= $5::timestamptz
              AND open_time < $6::timestamptz
            ORDER BY open_time ASC
            "#,
        )
        .bind(query.source_name.as_deref())
        .bind(&query.venue)
        .bind(&query.symbol)
        .bind(query.timeframe.as_str())
        .bind(query.start.to_string())
        .bind(query.end.to_string())
        .fetch_all(&self.pool)
        .await?;
        let present: std::collections::HashSet<Timestamp> = rows
            .into_iter()
            .map(|row| {
                row.try_get::<String, _>("open_time")?
                    .parse()
                    .map_err(Into::into)
            })
            .collect::<anyhow::Result<_>>()?;
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

async fn find_existing(
    tx: &mut sqlx_core::transaction::Transaction<'_, Postgres>,
    candle: &MarketCandle,
) -> anyhow::Result<Option<MarketCandle>> {
    let row = sqlx_core::query::query(
        r#"
        SELECT
          source_name, venue, symbol, timeframe,
          open_time::text AS open_time,
          close_time::text AS close_time,
          open::text AS open,
          high::text AS high,
          low::text AS low,
          close::text AS close,
          volume::text AS volume,
          ingested_at::text AS ingested_at,
          provider_sequence
        FROM market_candles
        WHERE source_name = $1
          AND venue = $2
          AND symbol = $3
          AND timeframe = $4
          AND open_time = $5::timestamptz
        "#,
    )
    .bind(&candle.source_name)
    .bind(&candle.venue)
    .bind(&candle.symbol)
    .bind(candle.timeframe.as_str())
    .bind(candle.open_time.to_string())
    .fetch_optional(&mut **tx)
    .await?;

    row.map(row_to_candle).transpose()
}

async fn insert_candle(
    tx: &mut sqlx_core::transaction::Transaction<'_, Postgres>,
    candle: &MarketCandle,
) -> anyhow::Result<()> {
    sqlx_core::query::query(
        r#"
        INSERT INTO market_candles (
          source_name, venue, symbol, timeframe, open_time, close_time,
          open, high, low, close, volume, ingested_at, provider_sequence
        )
        VALUES (
          $1, $2, $3, $4, $5::timestamptz, $6::timestamptz,
          $7::numeric, $8::numeric, $9::numeric, $10::numeric, $11::numeric,
          $12::timestamptz, $13
        )
        "#,
    )
    .bind(&candle.source_name)
    .bind(&candle.venue)
    .bind(&candle.symbol)
    .bind(candle.timeframe.as_str())
    .bind(candle.open_time.to_string())
    .bind(candle.close_time.to_string())
    .bind(candle.open.to_string())
    .bind(candle.high.to_string())
    .bind(candle.low.to_string())
    .bind(candle.close.to_string())
    .bind(candle.volume.to_string())
    .bind(candle.ingested_at.to_string())
    .bind(&candle.provider_sequence)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn update_candle(
    tx: &mut sqlx_core::transaction::Transaction<'_, Postgres>,
    candle: &MarketCandle,
) -> anyhow::Result<()> {
    sqlx_core::query::query(
        r#"
        UPDATE market_candles
        SET close_time = $6::timestamptz,
            open = $7::numeric,
            high = $8::numeric,
            low = $9::numeric,
            close = $10::numeric,
            volume = $11::numeric,
            ingested_at = $12::timestamptz,
            provider_sequence = $13
        WHERE source_name = $1
          AND venue = $2
          AND symbol = $3
          AND timeframe = $4
          AND open_time = $5::timestamptz
        "#,
    )
    .bind(&candle.source_name)
    .bind(&candle.venue)
    .bind(&candle.symbol)
    .bind(candle.timeframe.as_str())
    .bind(candle.open_time.to_string())
    .bind(candle.close_time.to_string())
    .bind(candle.open.to_string())
    .bind(candle.high.to_string())
    .bind(candle.low.to_string())
    .bind(candle.close.to_string())
    .bind(candle.volume.to_string())
    .bind(candle.ingested_at.to_string())
    .bind(&candle.provider_sequence)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

fn row_to_candle(row: sqlx_postgres::PgRow) -> anyhow::Result<MarketCandle> {
    Ok(MarketCandle {
        source_name:       row.try_get("source_name")?,
        venue:             row.try_get("venue")?,
        symbol:            row.try_get("symbol")?,
        timeframe:         Timeframe::parse(row.try_get::<String, _>("timeframe")?)?,
        open_time:         row.try_get::<String, _>("open_time")?.parse()?,
        close_time:        row.try_get::<String, _>("close_time")?.parse()?,
        open:              Decimal::from_str_exact(&row.try_get::<String, _>("open")?)?,
        high:              Decimal::from_str_exact(&row.try_get::<String, _>("high")?)?,
        low:               Decimal::from_str_exact(&row.try_get::<String, _>("low")?)?,
        close:             Decimal::from_str_exact(&row.try_get::<String, _>("close")?)?,
        volume:            Decimal::from_str_exact(&row.try_get::<String, _>("volume")?)?,
        ingested_at:       row.try_get::<String, _>("ingested_at")?.parse()?,
        provider_sequence: row.try_get("provider_sequence")?,
    })
}

fn row_to_stream_summary(row: sqlx_postgres::PgRow) -> anyhow::Result<CandleStreamSummary> {
    let candle_count: i64 = row.try_get("candle_count")?;
    Ok(CandleStreamSummary {
        source_name:        row.try_get("source_name")?,
        venue:              row.try_get("venue")?,
        symbol:             row.try_get("symbol")?,
        timeframe:          Timeframe::parse(row.try_get::<String, _>("timeframe")?)?,
        candle_count:       usize::try_from(candle_count)?,
        first_open_time:    row.try_get::<String, _>("first_open_time")?.parse()?,
        latest_open_time:   row.try_get::<String, _>("latest_open_time")?.parse()?,
        latest_close_time:  row.try_get::<String, _>("latest_close_time")?.parse()?,
        latest_ingested_at: row.try_get::<String, _>("latest_ingested_at")?.parse()?,
    })
}
