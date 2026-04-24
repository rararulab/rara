// Copyright 2025 Rararulab
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

//! `query-feed` tool — query historical events from registered data feeds.

use async_trait::async_trait;
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::{
    data_feed::{FeedFilter, FeedStoreRef, parse_duration_ago},
    tool::{ToolContext, ToolExecute},
};

// ============================================================================
// QueryFeedTool
// ============================================================================

/// Tool for querying historical events from registered data feeds.
#[derive(ToolDef)]
#[tool(
    name = "query-feed",
    description = "Query historical events from registered data feeds. Filter by source name, \
                   tags, and time range. Returns matching events in chronological order.",
    tier = "deferred",
    read_only,
    concurrency_safe
)]
pub struct QueryFeedTool {
    feed_store: FeedStoreRef,
}

impl QueryFeedTool {
    /// Create a new `QueryFeedTool` backed by the given feed store.
    pub fn new(feed_store: FeedStoreRef) -> Self { Self { feed_store } }
}

/// Parameters for the `query-feed` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct QueryFeedParams {
    /// Filter by feed source name (e.g. "github-rara", "crypto-binance").
    /// Omit to query all sources.
    source: Option<String>,

    /// Filter by tags — only events carrying ALL specified tags are returned.
    #[serde(default)]
    tags: Vec<String>,

    /// Time range filter. Supports human-friendly durations: "1h", "24h",
    /// "7d", "30m". Omit to return the most recent events regardless of time.
    since: Option<String>,

    /// Maximum number of events to return. Defaults to 20 if omitted.
    limit: Option<usize>,
}

/// A single event in the query result.
#[derive(Debug, Serialize)]
struct QueryFeedEvent {
    id:          String,
    source:      String,
    event_type:  String,
    tags:        Vec<String>,
    payload:     serde_json::Value,
    received_at: String,
}

/// Result of a `query-feed` invocation.
#[derive(Debug, Serialize)]
pub struct QueryFeedResult {
    events: Vec<QueryFeedEvent>,
    count:  usize,
}

#[async_trait]
impl ToolExecute for QueryFeedTool {
    type Output = QueryFeedResult;
    type Params = QueryFeedParams;

    async fn run(
        &self,
        params: QueryFeedParams,
        _context: &ToolContext,
    ) -> anyhow::Result<QueryFeedResult> {
        let since = params
            .since
            .as_deref()
            .map(parse_duration_ago)
            .transpose()?;
        let limit = params.limit.unwrap_or(20).min(100);

        let filter = FeedFilter {
            source_name: params.source,
            tags: params.tags,
            since,
            limit,
        };

        debug!(?filter, "query-feed: executing query");

        let events = self.feed_store.query(filter).await?;
        let count = events.len();

        let events: Vec<QueryFeedEvent> = events
            .into_iter()
            .map(|e| QueryFeedEvent {
                id:          e.id.to_string(),
                source:      e.source_name,
                event_type:  e.event_type,
                tags:        e.tags,
                payload:     e.payload,
                received_at: e.received_at.to_string(),
            })
            .collect();

        Ok(QueryFeedResult { events, count })
    }
}
