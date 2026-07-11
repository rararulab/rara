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

use std::sync::Arc;

use async_trait::async_trait;
use jiff::Timestamp;
use rara_backend_admin::data_feeds::{DataFeedSvc, start_feed_task};
use rara_kernel::{
    data_feed::{DataFeedConfig, DataFeedRegistry, FeedStatus},
    tool::{ToolContext, ToolExecute},
};
use rara_tool_macro::ToolDef;
use rara_trading::feed::catalog::{DefaultFeedSource, default_finance_feed_sources};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const MAX_CATALOG_SOURCE_ID_LEN: usize = 128;

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct FinanceEnableFeedSourceParams {
    /// Built-in source id from `finance_list_feed_sources`.
    pub catalog_source_id: String,
    /// Whether to start the runtime feed task immediately. Defaults to true.
    #[serde(default)]
    pub start_now:         Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct FinanceEnableFeedSourceResult {
    pub catalog_source_id: String,
    pub feed_id:           String,
    pub source_name:       String,
    pub feed_type:         String,
    pub tags:              Vec<String>,
    pub created:           bool,
    pub enabled:           bool,
    pub started:           bool,
    pub running:           bool,
}

#[derive(ToolDef)]
#[tool(
    name = "finance_enable_feed_source",
    description = "Enable one built-in finance feed source from finance_list_feed_sources. This \
                   persists a DataFeedConfig and optionally starts the runtime feed task. It only \
                   works for catalog entries with can_enable=true; provider presets that require \
                   credentials or custom endpoints are rejected. This never places trades.",
    tier = "deferred"
)]
pub(super) struct FinanceEnableFeedSourceTool {
    svc:      DataFeedSvc,
    registry: Arc<DataFeedRegistry>,
}

impl FinanceEnableFeedSourceTool {
    pub(super) fn new(svc: DataFeedSvc, registry: Arc<DataFeedRegistry>) -> Self {
        Self { svc, registry }
    }
}

#[async_trait]
impl ToolExecute for FinanceEnableFeedSourceTool {
    type Output = FinanceEnableFeedSourceResult;
    type Params = FinanceEnableFeedSourceParams;

    async fn run(
        &self,
        params: FinanceEnableFeedSourceParams,
        _context: &ToolContext,
    ) -> anyhow::Result<FinanceEnableFeedSourceResult> {
        let catalog_source_id = normalize_catalog_source_id(params.catalog_source_id)?;
        let start_now = params.start_now.unwrap_or(true);
        let source = find_catalog_source(&catalog_source_id)?;
        anyhow::ensure!(
            source.can_enable(),
            "finance feed source {catalog_source_id} requires configuration: {}",
            source
                .setup_hint
                .as_deref()
                .unwrap_or("configure this provider before enabling it")
        );

        let source_name = source.feed_name();
        let existing = self
            .svc
            .list_feeds()
            .await?
            .into_iter()
            .find(|feed| feed.name == source_name);
        let already_running = self.registry.is_running(&source_name);
        let already_enabled = existing.as_ref().is_some_and(|feed| feed.enabled);

        let (config, created) = match existing {
            Some(feed) if already_enabled && already_running => (feed, false),
            Some(feed) => {
                let config = config_from_source(&source, Some(feed))?;
                anyhow::ensure!(
                    self.svc.update_feed(&config).await?,
                    "failed to update existing finance feed source: {source_name}"
                );
                replace_registry_config(&self.registry, config.clone())?;
                (config, false)
            }
            None => {
                let config = config_from_source(&source, None)?;
                self.svc.create_feed(&config).await?;
                self.registry.register(config.clone())?;
                (config, true)
            }
        };

        let started = if start_now && !already_running {
            start_feed_task(&config, &self.registry);
            true
        } else {
            false
        };

        Ok(FinanceEnableFeedSourceResult {
            catalog_source_id,
            feed_id: config.id,
            source_name: config.name,
            feed_type: config.feed_type.to_string(),
            tags: config.tags,
            created,
            enabled: config.enabled,
            started,
            running: self.registry.is_running(&source_name),
        })
    }
}

fn normalize_catalog_source_id(value: String) -> anyhow::Result<String> {
    let value = value.trim();
    anyhow::ensure!(!value.is_empty(), "catalog_source_id must not be empty");
    anyhow::ensure!(
        value.chars().count() <= MAX_CATALOG_SOURCE_ID_LEN,
        "catalog_source_id is too long"
    );
    Ok(value.to_owned())
}

fn find_catalog_source(catalog_source_id: &str) -> anyhow::Result<DefaultFeedSource> {
    default_finance_feed_sources()
        .into_iter()
        .find(|source| source.id == catalog_source_id)
        .ok_or_else(|| {
            anyhow::anyhow!("unknown finance feed catalog source id: {catalog_source_id}")
        })
}

fn config_from_source(
    source: &DefaultFeedSource,
    existing: Option<DataFeedConfig>,
) -> anyhow::Result<DataFeedConfig> {
    let now = Timestamp::now();
    let transport = source
        .transport
        .clone()
        .ok_or_else(|| anyhow::anyhow!("finance feed source {} has no transport", source.id))?;
    let id = existing
        .as_ref()
        .map(|feed| feed.id.clone())
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let created_at = existing.as_ref().map(|feed| feed.created_at).unwrap_or(now);

    Ok(DataFeedConfig::builder()
        .id(id)
        .name(source.feed_name())
        .feed_type(source.feed_type)
        .tags(source.tags.clone())
        .transport(transport)
        .maybe_auth(source.auth.clone())
        .enabled(true)
        .status(FeedStatus::Idle)
        .maybe_last_error(None)
        .created_at(created_at)
        .updated_at(now)
        .build())
}

fn replace_registry_config(
    registry: &Arc<DataFeedRegistry>,
    config: DataFeedConfig,
) -> anyhow::Result<()> {
    if registry.get(&config.name).is_some() {
        registry.remove(&config.name)?;
    }
    registry.register(config)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use diesel_async::RunQueryDsl;
    use rara_kernel::{
        io::MessageId,
        queue::{ShardedEventQueue, ShardedEventQueueConfig},
        session::SessionKey,
        tool::{AgentTool, ToolContext, ToolExecute},
    };

    use super::{FinanceEnableFeedSourceParams, FinanceEnableFeedSourceTool};

    fn context() -> ToolContext {
        ToolContext {
            user_id:               "alice".to_owned(),
            session_key:           SessionKey::new(),
            origin_endpoint:       None,
            origin_user_id:        None,
            event_queue:           Arc::new(ShardedEventQueue::new(ShardedEventQueueConfig {
                num_shards:      0,
                shard_capacity:  1,
                global_capacity: 16,
            })),
            rara_turn_id:          MessageId::new(),
            context_window_tokens: 0,
            tool_registry:         None,
            stream_handle:         None,
            tool_call_id:          None,
        }
    }

    async fn tool() -> (
        FinanceEnableFeedSourceTool,
        rara_backend_admin::data_feeds::DataFeedSvc,
        Arc<rara_kernel::data_feed::DataFeedRegistry>,
    ) {
        let pools = rara_kernel::testing::build_memory_diesel_pools().await;
        bootstrap_data_feed_schema(&pools).await;
        let svc = rara_backend_admin::data_feeds::DataFeedSvc::new(pools);
        let (event_tx, _event_rx) = tokio::sync::mpsc::channel(16);
        let registry = Arc::new(rara_kernel::data_feed::DataFeedRegistry::new(event_tx));
        (
            FinanceEnableFeedSourceTool::new(svc.clone(), registry.clone()),
            svc,
            registry,
        )
    }

    async fn bootstrap_data_feed_schema(pools: &yunara_store::diesel_pool::DieselSqlitePools) {
        let mut conn = pools.writer.get().await.expect("pool conn");
        for ddl in [
            "CREATE TABLE data_feeds (
                id TEXT PRIMARY KEY NOT NULL,
                name TEXT NOT NULL UNIQUE,
                feed_type TEXT NOT NULL,
                tags TEXT NOT NULL DEFAULT '[]',
                transport TEXT NOT NULL DEFAULT '{}',
                auth TEXT,
                enabled INTEGER NOT NULL DEFAULT 1,
                status TEXT NOT NULL DEFAULT 'idle',
                last_error TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
                updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
            )",
            "CREATE INDEX idx_data_feeds_name ON data_feeds(name)",
            "CREATE INDEX idx_data_feeds_type ON data_feeds(feed_type)",
        ] {
            diesel::sql_query(ddl)
                .execute(&mut *conn)
                .await
                .expect("bootstrap data feed schema");
        }
    }

    #[tokio::test]
    async fn enable_feed_source_persists_and_registers_config_without_starting() {
        let (tool, svc, registry) = tool().await;
        let result = tool
            .run(
                FinanceEnableFeedSourceParams {
                    catalog_source_id: "fed-press-releases".to_owned(),
                    start_now:         Some(false),
                },
                &context(),
            )
            .await
            .unwrap();

        assert!(result.created);
        assert!(result.enabled);
        assert!(!result.started);
        assert!(!result.running);
        assert_eq!(result.source_name, "finance-fed-press-releases");
        assert_eq!(result.feed_type, "rss");
        assert!(registry.get("finance-fed-press-releases").is_some());

        let feeds = svc.list_feeds().await.unwrap();
        assert_eq!(feeds.len(), 1);
        assert_eq!(feeds[0].name, "finance-fed-press-releases");
        assert!(feeds[0].enabled);
    }

    #[tokio::test]
    async fn enable_feed_source_is_idempotent_for_existing_enabled_source() {
        let (tool, svc, _registry) = tool().await;
        for expected_created in [true, false] {
            let result = tool
                .run(
                    FinanceEnableFeedSourceParams {
                        catalog_source_id: "fed-press-releases".to_owned(),
                        start_now:         Some(false),
                    },
                    &context(),
                )
                .await
                .unwrap();
            assert_eq!(result.created, expected_created);
        }

        assert_eq!(svc.list_feeds().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn enable_feed_source_rejects_sources_that_require_configuration() {
        let (tool, _svc, _registry) = tool().await;
        let err = tool
            .run(
                FinanceEnableFeedSourceParams {
                    catalog_source_id: "longbridge-market-candles".to_owned(),
                    start_now:         Some(false),
                },
                &context(),
            )
            .await
            .unwrap_err();

        assert!(err.to_string().contains("requires configuration"));
    }

    #[tokio::test]
    async fn enable_feed_source_is_mutating() {
        let (tool, _svc, _registry) = tool().await;

        assert!(!tool.is_read_only(&serde_json::json!({})));
    }
}
