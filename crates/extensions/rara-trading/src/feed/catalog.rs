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

//! Built-in finance feed source catalog.
//!
//! The catalog separates sources that can run immediately from provider
//! presets that need operator credentials or a normalized market-data endpoint.

use rara_kernel::data_feed::{AuthConfig, FeedType};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct DefaultFeedSource {
    pub id:                     String,
    pub name:                   String,
    pub description:            String,
    pub feed_type:              FeedType,
    pub provider:               Option<String>,
    pub tags:                   Vec<String>,
    pub transport:              Option<serde_json::Value>,
    pub auth:                   Option<AuthConfig>,
    pub requires_configuration: bool,
    pub setup_hint:             Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DefaultFeedBundle {
    pub id:                 String,
    pub name:               String,
    pub description:        String,
    pub tags:               Vec<String>,
    pub catalog_source_ids: Vec<String>,
}

impl DefaultFeedSource {
    #[must_use]
    pub fn feed_name(&self) -> String { format!("finance-{}", self.id) }

    #[must_use]
    pub fn can_enable(&self) -> bool { !self.requires_configuration && self.transport.is_some() }
}

#[must_use]
pub fn default_finance_feed_bundles() -> Vec<DefaultFeedBundle> {
    vec![
        feed_bundle(
            "macro-news",
            "Macro News",
            "Federal Reserve and SEC official RSS feeds for macro and regulatory monitoring.",
            ["finance", "news", "macro", "regulatory"],
            [
                "fed-press-releases",
                "fed-h15-announcements",
                "fed-h10-announcements",
                "sec-press-releases",
            ],
        ),
        feed_bundle(
            "binance-spot-starter",
            "Binance Spot Starter",
            "Public Binance BTCUSDT and ETHUSDT 1m closed candles.",
            ["finance", "market-data", "crypto", "binance"],
            ["binance-market-candles"],
        ),
        feed_bundle(
            "binance-major-crypto-15m",
            "Binance Major Crypto 15m",
            "Public Binance 15m closed candles for major USDT crypto pairs.",
            ["finance", "market-data", "crypto", "binance"],
            ["binance-major-crypto-15m"],
        ),
        feed_bundle(
            "yahoo-us-equities-daily",
            "Yahoo US Equities Daily",
            "Keyless best-effort daily candles for a focused US equities research watchlist.",
            ["finance", "market-data", "equities", "yahoo", "best-effort"],
            ["yahoo-market-candles"],
        ),
        feed_bundle(
            "yahoo-global-starter",
            "Yahoo Global Starter",
            "Keyless best-effort daily candles for a small cross-market research watchlist.",
            ["finance", "market-data", "global", "yahoo", "best-effort"],
            ["yahoo-global-market-candles"],
        ),
        feed_bundle(
            "fmp-us-equities-daily",
            "FMP US Equities Daily",
            "Official FMP end-of-day candles for a focused US equities research watchlist.",
            ["finance", "market-data", "equities", "fmp", "delayed"],
            ["fmp-us-equities-daily"],
        ),
        feed_bundle(
            "longbridge-equities-daily",
            "Longbridge Equities Daily",
            "Longbridge equities daily candles preset for configured normalized endpoints.",
            ["finance", "market-data", "equities", "longbridge"],
            ["longbridge-market-candles"],
        ),
    ]
}

#[must_use]
pub fn default_finance_feed_sources() -> Vec<DefaultFeedSource> {
    vec![
        rss_source(
            "fed-press-releases",
            "Federal Reserve Press Releases",
            "Official Federal Reserve Board press releases.",
            "https://www.federalreserve.gov/feeds/press_all.xml",
            ["finance", "news", "fed", "macro"],
            300,
        ),
        rss_source(
            "fed-h15-announcements",
            "Federal Reserve H.15 Announcements",
            "Announcements for selected interest rates published through the Fed Data Download \
             Program.",
            "https://www.federalreserve.gov/feeds/h15.xml",
            ["finance", "rates", "fed", "macro"],
            900,
        ),
        rss_source(
            "fed-h10-announcements",
            "Federal Reserve H.10 Announcements",
            "Announcements for foreign exchange rates published through the Fed Data Download \
             Program.",
            "https://www.federalreserve.gov/feeds/h10.xml",
            ["finance", "fx", "fed", "macro"],
            900,
        ),
        rss_source(
            "sec-press-releases",
            "SEC Press Releases",
            "Official U.S. Securities and Exchange Commission press releases.",
            "https://www.sec.gov/news/pressreleases.rss",
            ["finance", "news", "sec", "regulatory"],
            300,
        ),
        binance_market_candles_source(
            "binance-market-candles",
            "Binance Market Candles",
            "Public Binance spot OHLCV feed for BTCUSDT and ETHUSDT 1m candles.",
            ["finance", "market-data", "crypto", "binance"],
            &["BTCUSDT", "ETHUSDT"],
            &["1m"],
            60,
        ),
        binance_market_candles_source(
            "binance-major-crypto-15m",
            "Binance Major Crypto 15m",
            "Public Binance spot OHLCV feed for major crypto USDT pairs on 15m candles.",
            ["finance", "market-data", "crypto", "binance"],
            &["BTCUSDT", "ETHUSDT", "SOLUSDT", "BNBUSDT", "XRPUSDT"],
            &["15m"],
            300,
        ),
        yahoo_market_candles_source(
            "yahoo-market-candles",
            "Yahoo US Equities Daily",
            "Keyless Yahoo Finance daily candles for research. Best effort, potentially delayed, \
             unofficial, and not intended for execution-grade decisions or redistribution.",
            [
                "finance",
                "market-data",
                "equities",
                "yahoo",
                "no-auth",
                "best-effort",
                "delayed",
                "unofficial-api",
                "redistribution-restricted",
            ],
            &["AAPL", "MSFT", "NVDA", "SPY", "QQQ"],
        ),
        yahoo_market_candles_source(
            "yahoo-global-market-candles",
            "Yahoo Global Market Daily",
            "Keyless Yahoo Finance daily candles for cross-market research. Best effort, \
             potentially delayed, unofficial, and not intended for execution-grade decisions or \
             redistribution.",
            [
                "finance",
                "market-data",
                "global",
                "yahoo",
                "no-auth",
                "best-effort",
                "delayed",
                "unofficial-api",
                "redistribution-restricted",
            ],
            &["^GSPC", "^N225", "000001.SS", "BTC-USD", "EURUSD=X"],
        ),
        fmp_market_candles_source(
            "fmp-us-equities-daily",
            "FMP US Equities Daily",
            "Official FMP end-of-day OHLCV candles for research. Requires an FMP API key and may \
             be delayed according to the account's market-data plan.",
            &["AAPL", "MSFT", "NVDA", "SPY", "QQQ"],
        ),
        provider_preset(
            "longbridge-market-candles",
            "Longbridge Market Data",
            "Preset for Longbridge equities market data through a normalized candle endpoint.",
            "longbridge",
            ["finance", "market-data", "equities", "longbridge"],
            "Connect Longbridge credentials behind a normalized candle endpoint before enabling.",
            &["AAPL.US", "NVDA.US", "700.HK"],
            &["1d"],
        ),
    ]
}

fn fmp_market_candles_source(
    id: &str,
    name: &str,
    description: &str,
    symbols: &[&str],
) -> DefaultFeedSource {
    DefaultFeedSource {
        id:                     id.to_owned(),
        name:                   name.to_owned(),
        description:            description.to_owned(),
        feed_type:              FeedType::MarketCandle,
        provider:               Some("fmp".to_owned()),
        tags:                   [
            "finance",
            "market-data",
            "equities",
            "fmp",
            "official-api",
            "delayed",
            "requires-api-key",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
        transport:              Some(serde_json::json!({
            "provider": "fmp",
            "base_url": "https://financialmodelingprep.com",
            "interval_secs": 900,
            "headers": {},
            "venue": "fmp",
            "symbols": symbols,
            "timeframes": ["1d"],
            "max_candles_per_poll": 5
        })),
        auth:                   None,
        requires_configuration: true,
        setup_hint:             Some(
            "Add an FMP API key using API Key (Header), set the field name to apikey, then enable \
             the source. Query authentication is also supported."
                .to_owned(),
        ),
    }
}

fn yahoo_market_candles_source(
    id: &str,
    name: &str,
    description: &str,
    tags: impl IntoIterator<Item = &'static str>,
    symbols: &[&str],
) -> DefaultFeedSource {
    DefaultFeedSource {
        id:                     id.to_owned(),
        name:                   name.to_owned(),
        description:            description.to_owned(),
        feed_type:              FeedType::MarketCandle,
        provider:               Some("yahoo".to_owned()),
        tags:                   tags.into_iter().map(str::to_owned).collect(),
        transport:              Some(serde_json::json!({
            "provider": "yahoo",
            "base_url": "https://query1.finance.yahoo.com",
            "interval_secs": 900,
            "headers": {},
            "venue": "yahoo",
            "symbols": symbols,
            "timeframes": ["1d"],
            "max_candles_per_poll": 5
        })),
        auth:                   None,
        requires_configuration: false,
        setup_hint:             None,
    }
}

fn feed_bundle(
    id: &str,
    name: &str,
    description: &str,
    tags: impl IntoIterator<Item = &'static str>,
    catalog_source_ids: impl IntoIterator<Item = &'static str>,
) -> DefaultFeedBundle {
    DefaultFeedBundle {
        id:                 id.to_owned(),
        name:               name.to_owned(),
        description:        description.to_owned(),
        tags:               tags.into_iter().map(str::to_owned).collect(),
        catalog_source_ids: catalog_source_ids.into_iter().map(str::to_owned).collect(),
    }
}

fn rss_source(
    id: &str,
    name: &str,
    description: &str,
    url: &str,
    tags: impl IntoIterator<Item = &'static str>,
    interval_secs: u64,
) -> DefaultFeedSource {
    DefaultFeedSource {
        id:                     id.to_owned(),
        name:                   name.to_owned(),
        description:            description.to_owned(),
        feed_type:              FeedType::Rss,
        provider:               None,
        tags:                   tags.into_iter().map(str::to_owned).collect(),
        transport:              Some(serde_json::json!({
            "url": url,
            "interval_secs": interval_secs,
            "headers": {},
            "max_entries_per_poll": 50
        })),
        auth:                   None,
        requires_configuration: false,
        setup_hint:             None,
    }
}

/// Builds a public Binance spot OHLCV preset polling `/api/v3/klines`.
///
/// `interval_secs` is the polling cadence. Sub-minute monitoring (5–10s) is
/// supported for timely flash-crash detection; the transport rejects any
/// cadence below the `MIN_INTERVAL_SECS` floor at load. The effective request
/// rate is `symbols × timeframes / interval_secs` per second, so keep the
/// monitored set focused when polling near the floor to stay within Binance's
/// per-IP request-weight budget.
fn binance_market_candles_source(
    id: &str,
    name: &str,
    description: &str,
    tags: impl IntoIterator<Item = &'static str>,
    symbols: &[&str],
    timeframes: &[&str],
    interval_secs: u64,
) -> DefaultFeedSource {
    DefaultFeedSource {
        id:                     id.to_owned(),
        name:                   name.to_owned(),
        description:            description.to_owned(),
        feed_type:              FeedType::MarketCandle,
        provider:               Some("binance".to_owned()),
        tags:                   tags.into_iter().map(str::to_owned).collect(),
        transport:              Some(serde_json::json!({
            "provider": "binance",
            "base_url": "https://api.binance.com",
            "interval_secs": interval_secs,
            "headers": {},
            "venue": "binance",
            "symbols": symbols,
            "timeframes": timeframes,
            "max_candles_per_poll": 1000
        })),
        auth:                   None,
        requires_configuration: false,
        setup_hint:             None,
    }
}

fn provider_preset(
    id: &str,
    name: &str,
    description: &str,
    venue: &str,
    tags: impl IntoIterator<Item = &'static str>,
    setup_hint: &str,
    symbols: &[&str],
    timeframes: &[&str],
) -> DefaultFeedSource {
    DefaultFeedSource {
        id:                     id.to_owned(),
        name:                   name.to_owned(),
        description:            description.to_owned(),
        feed_type:              FeedType::MarketCandle,
        provider:               Some(venue.to_owned()),
        tags:                   tags.into_iter().map(str::to_owned).collect(),
        transport:              Some(serde_json::json!({
            "url": "",
            "interval_secs": 60,
            "headers": {},
            "venue": venue,
            "symbols": symbols,
            "timeframes": timeframes,
            "max_candles_per_poll": 1000
        })),
        auth:                   None,
        requires_configuration: true,
        setup_hint:             Some(setup_hint.to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use rara_kernel::data_feed::{DataFeedConfig, FeedStatus, FeedType};

    use super::{default_finance_feed_bundles, default_finance_feed_sources};
    use crate::feed::{market_candle::MarketCandleSource, rss::RssSource};

    #[test]
    fn catalog_ids_and_feed_names_are_unique() {
        let catalog = default_finance_feed_sources();
        let mut ids = HashSet::new();
        let mut feed_names = HashSet::new();

        for source in catalog {
            assert!(!source.id.trim().is_empty());
            assert!(ids.insert(source.id.clone()), "duplicate id {}", source.id);
            assert!(
                feed_names.insert(source.feed_name()),
                "duplicate feed name for {}",
                source.id
            );
            assert!(
                source.tags.iter().any(|tag| tag == "finance"),
                "{} must include finance tag",
                source.id
            );
        }
    }

    #[test]
    fn bundle_ids_are_unique_and_reference_existing_sources() {
        let source_ids = default_finance_feed_sources()
            .into_iter()
            .map(|source| source.id)
            .collect::<HashSet<_>>();
        let mut bundle_ids = HashSet::new();

        for bundle in default_finance_feed_bundles() {
            assert!(!bundle.id.trim().is_empty());
            assert!(
                bundle_ids.insert(bundle.id.clone()),
                "duplicate bundle id {}",
                bundle.id
            );
            assert!(
                bundle.tags.iter().any(|tag| tag == "finance"),
                "{} must include finance tag",
                bundle.id
            );
            assert!(
                !bundle.catalog_source_ids.is_empty(),
                "{} must reference at least one source",
                bundle.id
            );
            for source_id in bundle.catalog_source_ids {
                assert!(
                    source_ids.contains(&source_id),
                    "{} references unknown source {}",
                    bundle.id,
                    source_id
                );
            }
        }
    }

    #[test]
    fn ready_catalog_sources_have_valid_transport_templates() {
        for source in default_finance_feed_sources()
            .into_iter()
            .filter(|source| source.can_enable())
        {
            let config = DataFeedConfig::builder()
                .id(format!("test-{}", source.id))
                .name(source.feed_name())
                .feed_type(source.feed_type)
                .tags(source.tags)
                .transport(
                    source
                        .transport
                        .expect("ready source must have a transport template"),
                )
                .maybe_auth(source.auth)
                .enabled(true)
                .status(FeedStatus::Idle)
                .created_at(jiff::Timestamp::UNIX_EPOCH)
                .updated_at(jiff::Timestamp::UNIX_EPOCH)
                .build();

            match config.feed_type {
                FeedType::Rss => {
                    RssSource::from_config(&config)
                        .unwrap_or_else(|err| panic!("{} should be valid: {err}", config.name));
                }
                FeedType::MarketCandle => {
                    MarketCandleSource::from_config(&config)
                        .unwrap_or_else(|err| panic!("{} should be valid: {err}", config.name));
                }
                other => panic!("unexpected ready finance feed type {other:?}"),
            }
        }
    }

    #[test]
    fn catalog_contains_public_binance_15m_major_crypto_preset() {
        let source = default_finance_feed_sources()
            .into_iter()
            .find(|source| source.id == "binance-major-crypto-15m")
            .expect("missing Binance major crypto 15m preset");

        assert!(source.can_enable());
        assert_eq!(source.feed_type, FeedType::MarketCandle);
        let transport = source.transport.expect("transport template");
        assert_eq!(transport["provider"], "binance");
        assert_eq!(transport["venue"], "binance");
        assert_eq!(transport["timeframes"], serde_json::json!(["15m"]));
        assert_eq!(
            transport["symbols"],
            serde_json::json!(["BTCUSDT", "ETHUSDT", "SOLUSDT", "BNBUSDT", "XRPUSDT"])
        );
    }

    #[test]
    fn longbridge_preset_is_prefilled_but_requires_operator_endpoint() {
        let source = default_finance_feed_sources()
            .into_iter()
            .find(|source| source.id == "longbridge-market-candles")
            .expect("missing Longbridge preset");

        assert!(!source.can_enable());
        assert!(source.requires_configuration);
        assert!(source.setup_hint.is_some());
        assert_eq!(source.provider.as_deref(), Some("longbridge"));
        let transport = source.transport.expect("transport template");
        assert!(transport.get("provider").is_none());
        assert_eq!(transport["venue"], "longbridge");
        assert_eq!(
            transport["symbols"],
            serde_json::json!(["AAPL.US", "NVDA.US", "700.HK"])
        );
        assert_eq!(transport["timeframes"], serde_json::json!(["1d"]));
        assert_eq!(transport["url"], "");
    }

    #[test]
    fn yahoo_presets_are_keyless_best_effort_daily_sources() {
        let sources = default_finance_feed_sources();
        let yahoo = sources
            .iter()
            .find(|source| source.id == "yahoo-market-candles")
            .expect("missing Yahoo market candle preset");

        assert!(yahoo.can_enable());
        assert_eq!(yahoo.provider.as_deref(), Some("yahoo"));
        assert!(yahoo.auth.is_none());
        for tag in [
            "no-auth",
            "best-effort",
            "delayed",
            "unofficial-api",
            "redistribution-restricted",
        ] {
            assert!(yahoo.tags.iter().any(|candidate| candidate == tag));
        }
        let transport = yahoo.transport.as_ref().expect("transport template");
        assert_eq!(transport["provider"], "yahoo");
        assert_eq!(transport["venue"], "yahoo");
        assert_eq!(transport["timeframes"], serde_json::json!(["1d"]));
        assert_eq!(transport["interval_secs"], 900);

        let bundles = default_finance_feed_bundles();
        let us = bundles
            .iter()
            .find(|bundle| bundle.id == "yahoo-us-equities-daily")
            .expect("missing Yahoo US equities bundle");
        assert_eq!(us.catalog_source_ids, ["yahoo-market-candles"]);
        let global = bundles
            .iter()
            .find(|bundle| bundle.id == "yahoo-global-starter")
            .expect("missing Yahoo global starter bundle");
        assert_eq!(global.catalog_source_ids, ["yahoo-global-market-candles"]);
    }

    #[test]
    fn fmp_preset_is_an_official_configurable_equity_alternative() {
        let sources = default_finance_feed_sources();
        let fmp = sources
            .iter()
            .find(|source| source.id == "fmp-us-equities-daily")
            .expect("missing FMP US equities preset");

        assert!(!fmp.can_enable());
        assert!(fmp.requires_configuration);
        assert_eq!(fmp.provider.as_deref(), Some("fmp"));
        assert!(fmp.auth.is_none());
        for tag in ["official-api", "delayed", "requires-api-key"] {
            assert!(fmp.tags.iter().any(|candidate| candidate == tag));
        }
        assert!(
            fmp.setup_hint
                .as_deref()
                .is_some_and(|hint| hint.contains("apikey")),
            "setup should explain the required FMP auth field"
        );
        let transport = fmp.transport.as_ref().expect("transport template");
        assert_eq!(transport["provider"], "fmp");
        assert_eq!(transport["base_url"], "https://financialmodelingprep.com");
        assert_eq!(transport["venue"], "fmp");
        assert_eq!(transport["timeframes"], serde_json::json!(["1d"]));

        let bundle = default_finance_feed_bundles()
            .into_iter()
            .find(|bundle| bundle.id == "fmp-us-equities-daily")
            .expect("missing FMP US equities bundle");
        assert_eq!(bundle.catalog_source_ids, ["fmp-us-equities-daily"]);
    }
}
