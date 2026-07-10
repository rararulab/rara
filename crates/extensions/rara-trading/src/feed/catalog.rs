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
//! The catalog contains only sources that rara can enable without additional
//! operator secrets or provider adapters. Market-candle feeds stay manually
//! configured until an operator supplies a normalized candle endpoint.

use rara_kernel::data_feed::{AuthConfig, FeedType};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct DefaultFeedSource {
    pub id:          String,
    pub name:        String,
    pub description: String,
    pub feed_type:   FeedType,
    pub tags:        Vec<String>,
    pub transport:   serde_json::Value,
    pub auth:        Option<AuthConfig>,
}

impl DefaultFeedSource {
    #[must_use]
    pub fn feed_name(&self) -> String { format!("finance-{}", self.id) }
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
    ]
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
        id:          id.to_owned(),
        name:        name.to_owned(),
        description: description.to_owned(),
        feed_type:   FeedType::Rss,
        tags:        tags.into_iter().map(str::to_owned).collect(),
        transport:   serde_json::json!({
            "url": url,
            "interval_secs": interval_secs,
            "headers": {},
            "max_entries_per_poll": 50
        }),
        auth:        None,
    }
}
