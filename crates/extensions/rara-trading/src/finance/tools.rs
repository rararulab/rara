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
use rara_kernel::{
    identity::UserId,
    tool::{ToolContext, ToolExecute},
};
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    feed::catalog::{DefaultFeedSource, default_finance_feed_sources},
    finance::registry::{
        FinanceDelivery, FinanceEventKind, FinanceSubscription, FinanceSubscriptionRegistry,
    },
    market_data::Timeframe,
};

const MAX_SELECTOR_VALUES: usize = 64;
const MAX_SELECTOR_LEN: usize = 128;
const DEFAULT_COOLDOWN_SECS: u64 = 900;
const DEFAULT_MAX_IMMEDIATE_PER_HOUR: u16 = 6;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FinanceSubscribeParams {
    #[serde(default)]
    pub event_kinds:            Option<Vec<FinanceEventKind>>,
    #[serde(default)]
    pub catalog_source_ids:     Vec<String>,
    #[serde(default)]
    pub source_names:           Vec<String>,
    #[serde(default)]
    pub category_tags:          Vec<String>,
    #[serde(default)]
    pub watch_terms:            Vec<String>,
    #[serde(default)]
    pub venues:                 Vec<String>,
    #[serde(default)]
    pub symbols:                Vec<String>,
    #[serde(default)]
    pub timeframes:             Vec<String>,
    #[serde(default)]
    pub delivery:               Option<FinanceDelivery>,
    #[serde(default)]
    pub cooldown_secs:          Option<u64>,
    #[serde(default)]
    pub max_immediate_per_hour: Option<u16>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FinanceSubscribeResult {
    pub subscription_id:        Uuid,
    pub source_names:           Vec<String>,
    pub catalog_source_ids:     Vec<String>,
    pub delivery:               FinanceDelivery,
    pub cooldown_secs:          u64,
    pub max_immediate_per_hour: u16,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FinanceListFeedSourcesParams {}

#[derive(Debug, Clone, Serialize)]
pub struct FinanceFeedSourceEntry {
    pub id:                     String,
    pub name:                   String,
    pub description:            String,
    pub feed_type:              String,
    pub tags:                   Vec<String>,
    pub source_name:            String,
    pub requires_configuration: bool,
    pub can_enable:             bool,
    pub setup_hint:             Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FinanceListFeedSourcesResult {
    pub sources: Vec<FinanceFeedSourceEntry>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FinanceUnsubscribeParams {
    pub subscription_id: Uuid,
}

#[derive(Debug, Clone, Serialize)]
pub struct FinanceUnsubscribeResult {
    pub subscription_id: Uuid,
    pub removed:         bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FinanceListSubscriptionsParams {}

#[derive(Debug, Clone, Serialize)]
pub struct FinanceListSubscriptionsResult {
    pub subscriptions: Vec<FinanceSubscription>,
}

#[derive(ToolDef)]
#[tool(
    name = "finance_list_feed_sources",
    description = "List built-in finance data feed source catalog entries and their subscription \
                   source names. Use this before finance_subscribe when the user asks to watch a \
                   default feed such as Fed, SEC, Binance, or Longbridge. This is read-only and \
                   never places trades.",
    tier = "deferred",
    read_only,
    concurrency_safe
)]
pub struct FinanceListFeedSourcesTool;

#[async_trait]
impl ToolExecute for FinanceListFeedSourcesTool {
    type Output = FinanceListFeedSourcesResult;
    type Params = FinanceListFeedSourcesParams;

    async fn run(
        &self,
        _params: FinanceListFeedSourcesParams,
        _context: &ToolContext,
    ) -> anyhow::Result<FinanceListFeedSourcesResult> {
        Ok(FinanceListFeedSourcesResult {
            sources: default_finance_feed_sources()
                .into_iter()
                .map(feed_source_entry)
                .collect(),
        })
    }
}

#[derive(ToolDef)]
#[tool(
    name = "finance_subscribe",
    description = "Subscribe the current conversation to operator-configured finance information \
                   feeds. Identity and session are always taken from tool context; do not pass \
                   owner or session fields. Pass catalog_source_ids for built-in sources from \
                   finance_list_feed_sources, or source_names for custom feeds. Use for \
                   RSS/article watch terms and closed market candle symbol/timeframe updates. \
                   This never places trades.",
    tier = "deferred"
)]
pub struct FinanceSubscribeTool {
    registry: Arc<FinanceSubscriptionRegistry>,
}

impl FinanceSubscribeTool {
    pub fn new(registry: Arc<FinanceSubscriptionRegistry>) -> Self { Self { registry } }
}

#[async_trait]
impl ToolExecute for FinanceSubscribeTool {
    type Output = FinanceSubscribeResult;
    type Params = FinanceSubscribeParams;

    async fn run(
        &self,
        params: FinanceSubscribeParams,
        context: &ToolContext,
    ) -> anyhow::Result<FinanceSubscribeResult> {
        let mut params = params;
        expand_catalog_source_ids(&mut params)?;
        infer_event_kinds_from_scoped_selectors(&mut params);
        let catalog_source_ids = params.catalog_source_ids.clone();
        validate_selectors(&params)?;
        let delivery = params.delivery.unwrap_or(FinanceDelivery::Silent);
        let cooldown_secs = params.cooldown_secs.unwrap_or(DEFAULT_COOLDOWN_SECS);
        let max_immediate_per_hour = params
            .max_immediate_per_hour
            .unwrap_or(DEFAULT_MAX_IMMEDIATE_PER_HOUR);
        let owner = UserId(context.user_id.clone());

        let subscription = FinanceSubscription {
            id: Uuid::new_v4(),
            owner: owner.clone(),
            session_key: context.session_key,
            event_kinds: params.event_kinds.unwrap_or_default(),
            source_names: params.source_names,
            category_tags: params.category_tags,
            watch_terms: params.watch_terms,
            venues: params.venues,
            symbols: params.symbols,
            timeframes: params.timeframes,
            delivery,
            cooldown_secs,
            max_immediate_per_hour,
        };
        let subscription_id = self.registry.upsert(subscription).await?;
        let source_names = self
            .registry
            .list_for_owner(&owner)
            .await
            .into_iter()
            .find(|subscription| subscription.id == subscription_id)
            .map_or_else(Vec::new, |subscription| subscription.source_names);

        Ok(FinanceSubscribeResult {
            subscription_id,
            source_names,
            catalog_source_ids,
            delivery,
            cooldown_secs,
            max_immediate_per_hour,
        })
    }
}

#[derive(ToolDef)]
#[tool(
    name = "finance_unsubscribe",
    description = "Remove one finance information subscription owned by the current user. The \
                   current user is taken from tool context; this cannot remove another user's \
                   subscription.",
    tier = "deferred"
)]
pub struct FinanceUnsubscribeTool {
    registry: Arc<FinanceSubscriptionRegistry>,
}

impl FinanceUnsubscribeTool {
    pub fn new(registry: Arc<FinanceSubscriptionRegistry>) -> Self { Self { registry } }
}

#[async_trait]
impl ToolExecute for FinanceUnsubscribeTool {
    type Output = FinanceUnsubscribeResult;
    type Params = FinanceUnsubscribeParams;

    async fn run(
        &self,
        params: FinanceUnsubscribeParams,
        context: &ToolContext,
    ) -> anyhow::Result<FinanceUnsubscribeResult> {
        let owner = UserId(context.user_id.clone());
        let removed = self.registry.remove(&owner, params.subscription_id).await?;
        Ok(FinanceUnsubscribeResult {
            subscription_id: params.subscription_id,
            removed,
        })
    }
}

#[derive(ToolDef)]
#[tool(
    name = "finance_list_subscriptions",
    description = "List finance information subscriptions owned by the current user. Identity is \
                   taken from tool context; there is no owner or session parameter.",
    tier = "deferred",
    read_only,
    concurrency_safe
)]
pub struct FinanceListSubscriptionsTool {
    registry: Arc<FinanceSubscriptionRegistry>,
}

impl FinanceListSubscriptionsTool {
    pub fn new(registry: Arc<FinanceSubscriptionRegistry>) -> Self { Self { registry } }
}

#[async_trait]
impl ToolExecute for FinanceListSubscriptionsTool {
    type Output = FinanceListSubscriptionsResult;
    type Params = FinanceListSubscriptionsParams;

    async fn run(
        &self,
        _params: FinanceListSubscriptionsParams,
        context: &ToolContext,
    ) -> anyhow::Result<FinanceListSubscriptionsResult> {
        let owner = UserId(context.user_id.clone());
        Ok(FinanceListSubscriptionsResult {
            subscriptions: self.registry.list_for_owner(&owner).await,
        })
    }
}

fn validate_selectors(params: &FinanceSubscribeParams) -> anyhow::Result<()> {
    if params.source_names.is_empty()
        && params.category_tags.is_empty()
        && params.watch_terms.is_empty()
        && params.venues.is_empty()
        && params.symbols.is_empty()
        && params.timeframes.is_empty()
    {
        anyhow::bail!(
            "at least one source/category/watch-term/venue/symbol/timeframe selector is required"
        );
    }

    validate_string_group("source_names", &params.source_names)?;
    validate_string_group("category_tags", &params.category_tags)?;
    validate_string_group("watch_terms", &params.watch_terms)?;
    validate_string_group("venues", &params.venues)?;
    validate_string_group("symbols", &params.symbols)?;
    validate_string_group("timeframes", &params.timeframes)?;
    validate_timeframes(&params.timeframes)?;
    validate_selector_scope(params)?;

    if let Some(event_kinds) = &params.event_kinds {
        anyhow::ensure!(
            event_kinds.len() <= MAX_SELECTOR_VALUES,
            "event_kinds has too many values"
        );
    }
    if let Some(max) = params.max_immediate_per_hour {
        anyhow::ensure!(max <= 60, "max_immediate_per_hour must be <= 60");
    }
    if let Some(cooldown) = params.cooldown_secs {
        anyhow::ensure!(cooldown <= 86_400, "cooldown_secs must be <= 86400");
    }

    Ok(())
}

fn infer_event_kinds_from_scoped_selectors(params: &mut FinanceSubscribeParams) {
    if params.event_kinds.is_some() {
        return;
    }
    match (
        has_article_only_selectors(params),
        has_candle_only_selectors(params),
    ) {
        (true, false) => params.event_kinds = Some(vec![FinanceEventKind::RssArticle]),
        (false, true) => params.event_kinds = Some(vec![FinanceEventKind::MarketCandleClosed]),
        _ => {}
    }
}

fn validate_selector_scope(params: &FinanceSubscribeParams) -> anyhow::Result<()> {
    let Some(event_kinds) = &params.event_kinds else {
        return Ok(());
    };
    let includes_rss = event_kinds.contains(&FinanceEventKind::RssArticle);
    let includes_candle = event_kinds.contains(&FinanceEventKind::MarketCandleClosed);
    let has_article_only = has_article_only_selectors(params);
    let has_candle_only = has_candle_only_selectors(params);

    anyhow::ensure!(
        includes_rss || !has_article_only,
        "watch_terms selectors require rss_article event kind"
    );
    anyhow::ensure!(
        includes_candle || !has_candle_only,
        "venue/symbol/timeframe selectors require market_candle_closed event kind"
    );
    anyhow::ensure!(
        !(includes_rss && includes_candle && has_article_only && !has_candle_only),
        "subscriptions that include market_candle_closed and watch_terms must also include a \
         venue, symbol, or timeframe selector"
    );
    anyhow::ensure!(
        !(includes_rss && includes_candle && has_candle_only && !has_article_only),
        "subscriptions that include rss_article and market candle selectors must also include a \
         watch_terms selector"
    );

    Ok(())
}

fn has_article_only_selectors(params: &FinanceSubscribeParams) -> bool {
    !params.watch_terms.is_empty()
}

fn has_candle_only_selectors(params: &FinanceSubscribeParams) -> bool {
    !params.venues.is_empty() || !params.symbols.is_empty() || !params.timeframes.is_empty()
}

fn validate_timeframes(values: &[String]) -> anyhow::Result<()> {
    for value in values {
        let normalized = value.trim().to_ascii_lowercase();
        Timeframe::parse(&normalized)
            .map_err(|err| anyhow::anyhow!("invalid timeframe selector {value:?}: {err}"))?;
    }
    Ok(())
}

fn validate_string_group(name: &str, values: &[String]) -> anyhow::Result<()> {
    anyhow::ensure!(
        values.len() <= MAX_SELECTOR_VALUES,
        "{name} has too many values"
    );
    for value in values {
        let trimmed = value.trim();
        anyhow::ensure!(!trimmed.is_empty(), "{name} contains an empty value");
        anyhow::ensure!(
            trimmed.chars().count() <= MAX_SELECTOR_LEN,
            "{name} value is too long"
        );
    }
    Ok(())
}

fn expand_catalog_source_ids(params: &mut FinanceSubscribeParams) -> anyhow::Result<()> {
    if params.catalog_source_ids.is_empty() {
        return Ok(());
    }

    validate_string_group("catalog_source_ids", &params.catalog_source_ids)?;
    params.catalog_source_ids = dedupe(
        params
            .catalog_source_ids
            .iter()
            .map(|id| id.trim().to_owned())
            .filter(|id| !id.is_empty())
            .collect(),
    );
    let sources = default_finance_feed_sources();
    let mut expanded = Vec::with_capacity(params.catalog_source_ids.len());
    for id in &params.catalog_source_ids {
        let source = sources
            .iter()
            .find(|source| source.id == *id)
            .ok_or_else(|| anyhow::anyhow!("unknown finance feed catalog source id: {id}"))?;
        expanded.push(source.feed_name());
    }
    params.source_names.extend(expanded);
    params.source_names = dedupe(std::mem::take(&mut params.source_names));
    Ok(())
}

fn feed_source_entry(source: DefaultFeedSource) -> FinanceFeedSourceEntry {
    let source_name = source.feed_name();
    let can_enable = source.can_enable();
    FinanceFeedSourceEntry {
        id: source.id,
        name: source.name,
        description: source.description,
        feed_type: source.feed_type.to_string(),
        tags: source.tags,
        source_name,
        requires_configuration: source.requires_configuration,
        can_enable,
        setup_hint: source.setup_hint,
    }
}

fn dedupe(values: Vec<String>) -> Vec<String> {
    let mut out = Vec::with_capacity(values.len());
    for value in values {
        if !out.contains(&value) {
            out.push(value);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use rara_kernel::{
        io::MessageId,
        queue::{ShardedEventQueue, ShardedEventQueueConfig},
        session::SessionKey,
        tool::{AgentTool, ToolContext, ToolExecute},
    };

    use super::{
        FinanceListFeedSourcesParams, FinanceListFeedSourcesTool, FinanceListSubscriptionsTool,
        FinanceSubscribeParams, FinanceSubscribeTool, FinanceUnsubscribeParams,
        FinanceUnsubscribeTool,
    };
    use crate::finance::registry::{
        FinanceDelivery, FinanceEventKind, FinanceSubscriptionRegistry,
    };

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

    #[tokio::test]
    async fn subscribe_uses_context_owner_and_session_not_llm_fields() {
        let tmp = tempfile::tempdir().expect("tempdir should be created");
        let registry = Arc::new(FinanceSubscriptionRegistry::load(
            tmp.path().join("subs.json"),
        ));
        let tool = FinanceSubscribeTool::new(registry.clone());
        let ctx = context();

        let result = tool
            .run(
                FinanceSubscribeParams {
                    event_kinds:            Some(vec![FinanceEventKind::RssArticle]),
                    catalog_source_ids:     Vec::new(),
                    source_names:           vec!["fed-news".to_owned()],
                    category_tags:          Vec::new(),
                    watch_terms:            vec!["BTC".to_owned()],
                    venues:                 Vec::new(),
                    symbols:                Vec::new(),
                    timeframes:             Vec::new(),
                    delivery:               Some(FinanceDelivery::Immediate),
                    cooldown_secs:          None,
                    max_immediate_per_hour: None,
                },
                &ctx,
            )
            .await
            .unwrap();

        let subs = registry
            .list_for_owner(&rara_kernel::identity::UserId("alice".to_owned()))
            .await;
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0].owner.0, "alice");
        assert_eq!(subs[0].session_key, ctx.session_key);
        assert_eq!(subs[0].id, result.subscription_id);
    }

    #[tokio::test]
    async fn unsubscribe_cannot_remove_another_users_subscription() {
        let tmp = tempfile::tempdir().expect("tempdir should be created");
        let registry = Arc::new(FinanceSubscriptionRegistry::load(
            tmp.path().join("subs.json"),
        ));
        let subscribe = FinanceSubscribeTool::new(registry.clone());
        let unsubscribe = FinanceUnsubscribeTool::new(registry.clone());
        let alice = context();
        let mut bob = context();
        bob.user_id = "bob".to_owned();

        let result = subscribe
            .run(
                FinanceSubscribeParams {
                    event_kinds:            Some(vec![FinanceEventKind::RssArticle]),
                    catalog_source_ids:     Vec::new(),
                    source_names:           vec!["fed-news".to_owned()],
                    category_tags:          Vec::new(),
                    watch_terms:            vec!["BTC".to_owned()],
                    venues:                 Vec::new(),
                    symbols:                Vec::new(),
                    timeframes:             Vec::new(),
                    delivery:               None,
                    cooldown_secs:          None,
                    max_immediate_per_hour: None,
                },
                &alice,
            )
            .await
            .unwrap();

        let removed = unsubscribe
            .run(
                FinanceUnsubscribeParams {
                    subscription_id: result.subscription_id,
                },
                &bob,
            )
            .await
            .unwrap();
        assert!(!removed.removed);
        assert_eq!(
            registry
                .list_for_owner(&rara_kernel::identity::UserId("alice".to_owned()))
                .await
                .len(),
            1
        );
    }

    #[test]
    fn list_schema_has_no_identity_or_session_parameter() {
        let tmp = tempfile::tempdir().expect("tempdir should be created");
        let registry = Arc::new(FinanceSubscriptionRegistry::load(
            tmp.path().join("subs.json"),
        ));
        let tool = FinanceListSubscriptionsTool::new(registry);
        let schema = tool.parameters_schema();
        let schema_json = schema.to_string();

        assert!(!schema_json.contains("user_id"));
        assert!(!schema_json.contains("owner"));
        assert!(!schema_json.contains("session"));
        assert!(tool.is_read_only(&serde_json::json!({})));
    }

    #[tokio::test]
    async fn list_feed_sources_exposes_subscription_source_names() {
        let tool = FinanceListFeedSourcesTool;
        let result = tool
            .run(FinanceListFeedSourcesParams {}, &context())
            .await
            .unwrap();

        let fed = result
            .sources
            .iter()
            .find(|source| source.id == "fed-press-releases")
            .expect("fed source should be listed");
        assert_eq!(fed.source_name, "finance-fed-press-releases");
        assert!(fed.tags.contains(&"finance".to_owned()));
        assert!(!fed.requires_configuration);
        assert!(fed.can_enable);

        let binance = result
            .sources
            .iter()
            .find(|source| source.id == "binance-market-candles")
            .expect("binance source should be listed");
        assert_eq!(binance.source_name, "finance-binance-market-candles");
        assert_eq!(binance.feed_type, "market_candle");
    }

    #[tokio::test]
    async fn subscribe_expands_catalog_source_ids_to_source_names() {
        let tmp = tempfile::tempdir().expect("tempdir should be created");
        let registry = Arc::new(FinanceSubscriptionRegistry::load(
            tmp.path().join("subs.json"),
        ));
        let tool = FinanceSubscribeTool::new(registry.clone());
        let ctx = context();

        let result = tool
            .run(
                FinanceSubscribeParams {
                    event_kinds:            Some(vec![FinanceEventKind::RssArticle]),
                    catalog_source_ids:     vec![
                        " fed-press-releases ".to_owned(),
                        "fed-press-releases".to_owned(),
                    ],
                    source_names:           Vec::new(),
                    category_tags:          Vec::new(),
                    watch_terms:            vec!["rate cut".to_owned()],
                    venues:                 Vec::new(),
                    symbols:                Vec::new(),
                    timeframes:             Vec::new(),
                    delivery:               None,
                    cooldown_secs:          None,
                    max_immediate_per_hour: None,
                },
                &ctx,
            )
            .await
            .unwrap();

        assert_eq!(result.catalog_source_ids, ["fed-press-releases"]);
        assert_eq!(result.source_names, ["finance-fed-press-releases"]);

        let subs = registry
            .list_for_owner(&rara_kernel::identity::UserId("alice".to_owned()))
            .await;
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0].source_names, ["finance-fed-press-releases"]);
        assert_eq!(subs[0].watch_terms, ["rate cut"]);
    }

    #[tokio::test]
    async fn subscribe_persists_normalized_market_selectors() {
        let tmp = tempfile::tempdir().expect("tempdir should be created");
        let registry = Arc::new(FinanceSubscriptionRegistry::load(
            tmp.path().join("subs.json"),
        ));
        let tool = FinanceSubscribeTool::new(registry.clone());
        let ctx = context();

        let result = tool
            .run(
                FinanceSubscribeParams {
                    event_kinds:            Some(vec![FinanceEventKind::MarketCandleClosed]),
                    catalog_source_ids:     Vec::new(),
                    source_names:           vec![" finance-binance-market-candles ".to_owned()],
                    category_tags:          vec![" Category:Market Data ".to_owned()],
                    watch_terms:            Vec::new(),
                    venues:                 vec![" Binance ".to_owned()],
                    symbols:                vec![" btcusdt ".to_owned()],
                    timeframes:             vec![" 15M ".to_owned()],
                    delivery:               None,
                    cooldown_secs:          None,
                    max_immediate_per_hour: None,
                },
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(result.source_names, ["finance-binance-market-candles"]);

        let subs = registry
            .list_for_owner(&rara_kernel::identity::UserId("alice".to_owned()))
            .await;
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0].source_names, ["finance-binance-market-candles"]);
        assert_eq!(subs[0].category_tags, ["category:market-data"]);
        assert_eq!(subs[0].venues, ["binance"]);
        assert_eq!(subs[0].symbols, ["BTCUSDT"]);
        assert_eq!(subs[0].timeframes, ["15m"]);
    }

    #[tokio::test]
    async fn subscribe_rejects_invalid_timeframe_selectors() {
        let tmp = tempfile::tempdir().expect("tempdir should be created");
        let registry = Arc::new(FinanceSubscriptionRegistry::load(
            tmp.path().join("subs.json"),
        ));
        let tool = FinanceSubscribeTool::new(registry);

        let err = tool
            .run(
                FinanceSubscribeParams {
                    event_kinds:            Some(vec![FinanceEventKind::MarketCandleClosed]),
                    catalog_source_ids:     Vec::new(),
                    source_names:           vec!["finance-binance-market-candles".to_owned()],
                    category_tags:          Vec::new(),
                    watch_terms:            Vec::new(),
                    venues:                 vec!["binance".to_owned()],
                    symbols:                vec!["BTCUSDT".to_owned()],
                    timeframes:             vec!["15min".to_owned()],
                    delivery:               None,
                    cooldown_secs:          None,
                    max_immediate_per_hour: None,
                },
                &context(),
            )
            .await
            .unwrap_err();

        assert!(
            err.to_string()
                .contains("invalid timeframe selector \"15min\""),
            "{err}"
        );
    }

    #[tokio::test]
    async fn subscribe_infers_rss_event_kind_for_watch_terms() {
        let tmp = tempfile::tempdir().expect("tempdir should be created");
        let registry = Arc::new(FinanceSubscriptionRegistry::load(
            tmp.path().join("subs.json"),
        ));
        let tool = FinanceSubscribeTool::new(registry.clone());

        tool.run(
            FinanceSubscribeParams {
                event_kinds:            None,
                catalog_source_ids:     Vec::new(),
                source_names:           vec!["finance-fed-press-releases".to_owned()],
                category_tags:          Vec::new(),
                watch_terms:            vec!["rate cut".to_owned()],
                venues:                 Vec::new(),
                symbols:                Vec::new(),
                timeframes:             Vec::new(),
                delivery:               None,
                cooldown_secs:          None,
                max_immediate_per_hour: None,
            },
            &context(),
        )
        .await
        .unwrap();

        let subs = registry
            .list_for_owner(&rara_kernel::identity::UserId("alice".to_owned()))
            .await;
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0].event_kinds, [FinanceEventKind::RssArticle]);
    }

    #[tokio::test]
    async fn subscribe_infers_candle_event_kind_for_market_selectors() {
        let tmp = tempfile::tempdir().expect("tempdir should be created");
        let registry = Arc::new(FinanceSubscriptionRegistry::load(
            tmp.path().join("subs.json"),
        ));
        let tool = FinanceSubscribeTool::new(registry.clone());

        tool.run(
            FinanceSubscribeParams {
                event_kinds:            None,
                catalog_source_ids:     Vec::new(),
                source_names:           vec!["finance-binance-market-candles".to_owned()],
                category_tags:          Vec::new(),
                watch_terms:            Vec::new(),
                venues:                 vec!["binance".to_owned()],
                symbols:                vec!["BTCUSDT".to_owned()],
                timeframes:             vec!["15m".to_owned()],
                delivery:               None,
                cooldown_secs:          None,
                max_immediate_per_hour: None,
            },
            &context(),
        )
        .await
        .unwrap();

        let subs = registry
            .list_for_owner(&rara_kernel::identity::UserId("alice".to_owned()))
            .await;
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0].event_kinds, [FinanceEventKind::MarketCandleClosed]);
    }

    #[tokio::test]
    async fn subscribe_rejects_mismatched_event_specific_selectors() {
        let tmp = tempfile::tempdir().expect("tempdir should be created");
        let registry = Arc::new(FinanceSubscriptionRegistry::load(
            tmp.path().join("subs.json"),
        ));
        let tool = FinanceSubscribeTool::new(registry);

        let rss_with_symbol = tool
            .run(
                FinanceSubscribeParams {
                    event_kinds:            Some(vec![FinanceEventKind::RssArticle]),
                    catalog_source_ids:     Vec::new(),
                    source_names:           vec!["finance-fed-press-releases".to_owned()],
                    category_tags:          Vec::new(),
                    watch_terms:            Vec::new(),
                    venues:                 Vec::new(),
                    symbols:                vec!["BTCUSDT".to_owned()],
                    timeframes:             Vec::new(),
                    delivery:               None,
                    cooldown_secs:          None,
                    max_immediate_per_hour: None,
                },
                &context(),
            )
            .await
            .unwrap_err();
        assert!(
            rss_with_symbol
                .to_string()
                .contains("market_candle_closed event kind"),
            "{rss_with_symbol}"
        );

        let candle_with_watch_term = tool
            .run(
                FinanceSubscribeParams {
                    event_kinds:            Some(vec![FinanceEventKind::MarketCandleClosed]),
                    catalog_source_ids:     Vec::new(),
                    source_names:           vec!["finance-binance-market-candles".to_owned()],
                    category_tags:          Vec::new(),
                    watch_terms:            vec!["BTC".to_owned()],
                    venues:                 Vec::new(),
                    symbols:                Vec::new(),
                    timeframes:             Vec::new(),
                    delivery:               None,
                    cooldown_secs:          None,
                    max_immediate_per_hour: None,
                },
                &context(),
            )
            .await
            .unwrap_err();
        assert!(
            candle_with_watch_term
                .to_string()
                .contains("rss_article event kind"),
            "{candle_with_watch_term}"
        );
    }
}
