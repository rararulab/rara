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

use rara_trading::market_data::{
    CandleRangeQuery, MarketCandle, MarketDataRepository, Timeframe, TimescaleMarketDataRepository,
    UpsertOutcome,
};
use rust_decimal::Decimal;
use sqlx_core::row::Row;
use testcontainers::{
    GenericImage, ImageExt,
    core::{IntoContainerPort, WaitFor},
    runners::AsyncRunner,
};

#[tokio::test(flavor = "multi_thread")]
async fn timescale_repository_contract_runs_against_testcontainer() -> anyhow::Result<()> {
    let container = GenericImage::new("timescale/timescaledb", "2.17.2-pg16")
        .with_exposed_port(5432.tcp())
        .with_wait_for(WaitFor::message_on_stderr(
            "database system is ready to accept connections",
        ))
        .with_env_var("POSTGRES_DB", "rara_test")
        .with_env_var("POSTGRES_USER", "rara")
        .with_env_var("POSTGRES_PASSWORD", "rara")
        .start()
        .await?;

    let port = container.get_host_port_ipv4(5432.tcp()).await?;
    let database_url = format!("postgres://rara:rara@127.0.0.1:{port}/rara_test");
    let repo = TimescaleMarketDataRepository::connect(&database_url).await?;
    repo.apply_schema().await?;

    let first = candle("2026-07-10T08:15:00Z", "61610.30");
    assert_eq!(
        repo.upsert_closed_candle(first.clone()).await?,
        UpsertOutcome::Inserted
    );
    assert_eq!(
        repo.upsert_closed_candle(first).await?,
        UpsertOutcome::DuplicateUnchanged
    );
    assert_eq!(
        repo.upsert_closed_candle(candle("2026-07-10T08:15:00Z", "61611.00"))
            .await?,
        UpsertOutcome::Corrected
    );

    let rows = repo
        .candles(CandleRangeQuery {
            source_name: Some("timescale-container-contract".to_owned()),
            venue:       "binance".to_owned(),
            symbol:      "BTCUSDT".to_owned(),
            timeframe:   Timeframe::parse("15m")?,
            start:       ts("2026-07-10T08:00:00Z"),
            end:         ts("2026-07-10T08:45:00Z"),
            limit:       10,
        })
        .await?;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].close, dec("61611.00"));

    let audit_pool = sqlx_postgres::PgPool::connect(&database_url).await?;
    let correction_count: i64 =
        sqlx_core::query::query("SELECT COUNT(*)::bigint AS count FROM market_candle_corrections")
            .fetch_one(&audit_pool)
            .await?
            .try_get("count")?;
    assert_eq!(correction_count, 1);

    Ok(())
}

fn ts(value: &str) -> jiff::Timestamp { value.parse().expect("timestamp fixture should parse") }

fn dec(value: &str) -> Decimal { value.parse().expect("decimal fixture should parse") }

fn candle(open_time: &str, close: &str) -> MarketCandle {
    MarketCandle {
        source_name:       "timescale-container-contract".to_owned(),
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
