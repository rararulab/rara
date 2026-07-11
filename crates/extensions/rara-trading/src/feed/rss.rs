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

//! Config-driven RSS/Atom data feed.
//!
//! This transport fetches an operator-configured feed URL and emits one
//! normalised `rss_article` event per feed item.

use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use jiff::Timestamp;
use rara_kernel::data_feed::{
    AuthConfig, DataFeed, DataFeedConfig, FeedEvent, FeedEventId, FeedStatus, StatusReporterRef,
    polling::apply_request_auth,
};
use reqwest::{
    Url,
    header::{HeaderName, HeaderValue},
};
use serde::Deserialize;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, instrument, warn};

const MAX_ENTRIES_PER_POLL: usize = 500;
const MAX_BODY_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, Deserialize)]
pub struct RssTransport {
    pub url:                  String,
    pub interval_secs:        u64,
    #[serde(default)]
    pub headers:              HashMap<String, String>,
    pub max_entries_per_poll: usize,
}

pub struct RssSource {
    name:      String,
    tags:      Vec<String>,
    transport: RssTransport,
    auth:      Option<AuthConfig>,
    client:    reqwest::Client,
    reporter:  Option<StatusReporterRef>,
    in_error:  Arc<AtomicBool>,
}

impl RssSource {
    pub fn from_config(config: &DataFeedConfig) -> anyhow::Result<Self> {
        let transport: RssTransport = serde_json::from_value(config.transport.clone())?;
        validate_transport(&transport)?;
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()?;
        Ok(Self {
            name: config.name.clone(),
            tags: config.tags.clone(),
            transport,
            auth: config.auth.clone(),
            client,
            reporter: None,
            in_error: Arc::new(AtomicBool::new(false)),
        })
    }

    #[must_use]
    pub fn with_reporter(mut self, reporter: StatusReporterRef) -> Self {
        self.reporter = Some(reporter);
        self
    }

    fn build_url(&self) -> anyhow::Result<Url> {
        let mut url = Url::parse(&self.transport.url)?;
        if let Some(AuthConfig::Query {
            ref name,
            ref value,
        }) = self.auth
        {
            url.query_pairs_mut().append_pair(name, value);
        }
        Ok(url)
    }

    fn record_error(&self, message: String) {
        let was_in_error = self.in_error.swap(true, Ordering::SeqCst);
        if was_in_error {
            debug!(feed = %self.name, error = %message, "rss fetch failed");
        } else {
            warn!(feed = %self.name, error = %message, "rss fetch failed");
            if let Some(reporter) = self.reporter.clone() {
                let name = self.name.clone();
                tokio::spawn(async move {
                    reporter
                        .report(&name, FeedStatus::Error, Some(message))
                        .await;
                });
            }
        }
    }

    fn record_success(&self) {
        if self.in_error.swap(false, Ordering::SeqCst) {
            info!(feed = %self.name, "rss feed recovered");
            if let Some(reporter) = self.reporter.clone() {
                let name = self.name.clone();
                tokio::spawn(async move {
                    reporter.report(&name, FeedStatus::Running, None).await;
                });
            }
        }
    }

    fn parse_feed_events(&self, body: &[u8]) -> anyhow::Result<Vec<FeedEvent>> {
        let feed = feed_rs::parser::parse(body)?;
        let received_at = Timestamp::now();
        let mut events = Vec::new();

        for entry in feed
            .entries
            .into_iter()
            .take(self.transport.max_entries_per_poll)
        {
            let url = entry.links.first().map(|link| link.href.clone());
            let title = entry.title.map(|text| text.content).unwrap_or_default();
            let summary = entry.summary.map(|text| text.content).unwrap_or_default();
            let author = entry
                .authors
                .first()
                .map(|person| person.email.clone().unwrap_or_else(|| person.name.clone()));
            let published_at = entry.published.map(|published| published.to_rfc3339());
            let categories: Vec<String> = entry
                .categories
                .iter()
                .map(|category| {
                    category
                        .label
                        .clone()
                        .unwrap_or_else(|| category.term.clone())
                })
                .filter(|category| !category.trim().is_empty())
                .collect();

            let identity = entry_identity(&entry.id, url.as_deref(), &title, &summary);
            let mut tags = self.tags.clone();
            tags.push(format!("source:{}", self.name));
            for category in &categories {
                let tag = normalize_tag(category);
                if !tag.is_empty() {
                    tags.push(format!("category:{tag}"));
                }
            }
            dedupe_sort(&mut tags);

            let payload = serde_json::json!({
                "title": title,
                "url": url,
                "summary": summary,
                "author": author,
                "published_at": published_at,
                "categories": categories,
            });

            events.push(
                FeedEvent::builder()
                    .id(FeedEventId::deterministic(&format!(
                        "{}:{identity}",
                        self.transport.url
                    )))
                    .source_name(self.name.clone())
                    .event_type("rss_article".to_owned())
                    .tags(tags)
                    .payload(payload)
                    .received_at(received_at)
                    .build(),
            );
        }

        Ok(events)
    }

    async fn poll_once(&self, tx: &mpsc::Sender<FeedEvent>) -> bool {
        let url = match self.build_url() {
            Ok(url) => url,
            Err(err) => {
                self.record_error(format!("failed to build RSS URL: {err}"));
                return true;
            }
        };

        let mut request = self.client.get(url);
        for (key, value) in &self.transport.headers {
            request = request.header(key.as_str(), value.as_str());
        }
        request = apply_request_auth(request, &self.auth);

        let response = match request.send().await {
            Ok(response) => response,
            Err(err) => {
                self.record_error(format!("RSS fetch failed: {err}"));
                return true;
            }
        };

        let status = response.status();
        if !status.is_success() {
            self.record_error(format!("RSS fetch received non-success status: {status}"));
            return true;
        }

        let body = match response.bytes().await {
            Ok(body) => body,
            Err(err) => {
                self.record_error(format!("failed to read RSS response body: {err}"));
                return true;
            }
        };
        if body.len() > MAX_BODY_BYTES {
            self.record_error(format!("RSS response exceeded {MAX_BODY_BYTES} bytes"));
            return true;
        }

        let events = match self.parse_feed_events(&body) {
            Ok(events) => events,
            Err(err) => {
                self.record_error(format!("failed to parse RSS response: {err}"));
                return true;
            }
        };

        for event in events {
            if tx.send(event).await.is_err() {
                tracing::info!("event channel closed, stopping rss loop");
                return false;
            }
        }

        self.record_success();
        true
    }
}

#[async_trait]
impl DataFeed for RssSource {
    fn name(&self) -> &str { &self.name }

    fn tags(&self) -> &[String] { &self.tags }

    #[instrument(skip_all, fields(feed = %self.name))]
    async fn run(
        &self,
        tx: mpsc::Sender<FeedEvent>,
        cancel: CancellationToken,
    ) -> anyhow::Result<()> {
        let interval = Duration::from_secs(self.transport.interval_secs);
        tracing::info!(url = %self.transport.url, ?interval, "rss feed started");

        let mut interval_timer = tokio::time::interval(interval);
        interval_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            tokio::select! {
                () = cancel.cancelled() => {
                    tracing::info!("rss feed cancelled, shutting down");
                    break;
                }
                _ = interval_timer.tick() => {
                    if !self.poll_once(&tx).await {
                        break;
                    }
                }
            }
        }

        Ok(())
    }
}

fn validate_transport(transport: &RssTransport) -> anyhow::Result<()> {
    let url = Url::parse(&transport.url)?;
    anyhow::ensure!(url.scheme() == "https", "RSS feed URL must use HTTPS");
    anyhow::ensure!(
        transport.interval_secs > 0,
        "RSS interval_secs must be greater than zero"
    );
    anyhow::ensure!(
        transport.max_entries_per_poll > 0,
        "RSS max_entries_per_poll must be greater than zero"
    );
    anyhow::ensure!(
        transport.max_entries_per_poll <= MAX_ENTRIES_PER_POLL,
        "RSS max_entries_per_poll must be <= {MAX_ENTRIES_PER_POLL}"
    );
    for (name, value) in &transport.headers {
        HeaderName::from_bytes(name.as_bytes())?;
        HeaderValue::from_str(value)?;
    }
    Ok(())
}

fn entry_identity(id: &str, url: Option<&str>, title: &str, summary: &str) -> String {
    let id = id.trim();
    if !id.is_empty() && !looks_generated_id(id) {
        return id.to_owned();
    }
    if let Some(url) = url.filter(|url| !url.trim().is_empty()) {
        return url.to_owned();
    }
    let content_identity = format!("{}:{}", title.trim(), summary.trim());
    if content_identity != ":" {
        return content_identity;
    }
    id.to_owned()
}

fn looks_generated_id(id: &str) -> bool {
    id.starts_with("urn:uuid:") || uuid::Uuid::parse_str(id).is_ok()
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

fn dedupe_sort(values: &mut Vec<String>) {
    let mut seen = HashSet::new();
    values.retain(|value| seen.insert(value.clone()));
    values.sort();
}

#[cfg(test)]
mod tests {
    use rara_kernel::data_feed::{DataFeedConfig, FeedStatus, FeedType};

    use super::RssSource;

    fn rss_source() -> RssSource {
        let config = DataFeedConfig::builder()
            .id("rss-test".to_owned())
            .name("fed-news".to_owned())
            .feed_type(FeedType::Rss)
            .tags(vec!["finance".to_owned(), "macro".to_owned()])
            .transport(serde_json::json!({
                "url": "https://example.com/feed.xml",
                "interval_secs": 300,
                "max_entries_per_poll": 20
            }))
            .enabled(true)
            .status(FeedStatus::Idle)
            .created_at(jiff::Timestamp::UNIX_EPOCH)
            .updated_at(jiff::Timestamp::UNIX_EPOCH)
            .build();

        RssSource::from_config(&config).expect("rss config should parse")
    }

    #[test]
    fn rss_item_becomes_one_normalized_article_event() {
        let source = rss_source();
        let events = source
            .parse_feed_events(
                br#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0">
  <channel>
    <title>Fed News</title>
    <item>
      <guid>fed-guid-1</guid>
      <title>Fed holds rates</title>
      <link>https://example.com/fed-holds-rates</link>
      <description>Policy summary</description>
      <author>press@example.com</author>
      <pubDate>Fri, 10 Jul 2026 08:30:00 GMT</pubDate>
      <category>Monetary Policy</category>
    </item>
  </channel>
</rss>"#,
            )
            .expect("feed should parse");

        assert_eq!(events.len(), 1);
        let event = &events[0];
        assert_eq!(event.source_name, "fed-news");
        assert_eq!(event.event_type, "rss_article");
        assert!(event.tags.contains(&"finance".to_owned()));
        assert!(event.tags.contains(&"source:fed-news".to_owned()));
        assert!(event.tags.contains(&"category:monetary-policy".to_owned()));
        assert_eq!(event.payload["title"], "Fed holds rates");
        assert_eq!(event.payload["url"], "https://example.com/fed-holds-rates");
        assert_eq!(event.payload["summary"], "Policy summary");
        assert_eq!(event.payload["author"], "press@example.com");
        assert_eq!(event.payload["categories"][0], "Monetary Policy");
    }

    #[test]
    fn same_guid_has_same_event_id_across_polls() {
        let source = rss_source();
        let body = br#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0">
  <channel>
    <title>Fed News</title>
    <item>
      <guid>stable-guid</guid>
      <title>Stable item</title>
      <link>https://example.com/stable</link>
    </item>
  </channel>
</rss>"#;

        let first = source.parse_feed_events(body).expect("first parse");
        let second = source.parse_feed_events(body).expect("second parse");

        assert_eq!(first[0].id, second[0].id);
    }

    #[test]
    fn missing_guid_uses_link_then_content_fallback() {
        let source = rss_source();
        let with_link = br#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0">
  <channel>
    <title>Fed News</title>
    <item>
      <title>Link fallback</title>
      <link>https://example.com/link-fallback</link>
    </item>
  </channel>
</rss>"#;
        let content_only = br#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0">
  <channel>
    <title>Fed News</title>
    <item>
      <title>Content fallback</title>
      <description>No link or guid</description>
    </item>
  </channel>
</rss>"#;

        let link_first = source.parse_feed_events(with_link).expect("link first");
        let link_second = source.parse_feed_events(with_link).expect("link second");
        let content_first = source
            .parse_feed_events(content_only)
            .expect("content first");
        let content_second = source
            .parse_feed_events(content_only)
            .expect("content second");

        assert_eq!(link_first[0].id, link_second[0].id);
        assert_eq!(content_first[0].id, content_second[0].id);
        assert_ne!(link_first[0].id, content_first[0].id);
    }
}
