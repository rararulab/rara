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

use super::registry::{
    FinanceDelivery, FinanceEventKind, FinanceSubscription, FinanceSubscriptionRegistry,
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
    pub delivery:               FinanceDelivery,
    pub cooldown_secs:          u64,
    pub max_immediate_per_hour: u16,
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
    name = "finance_subscribe",
    description = "Subscribe the current conversation to operator-configured finance information \
                   feeds. Identity and session are always taken from tool context; do not pass \
                   owner or session fields. Use for RSS/article watch terms and closed market \
                   candle symbol/timeframe updates. This never places trades.",
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
        validate_selectors(&params)?;
        let delivery = params.delivery.unwrap_or(FinanceDelivery::Silent);
        let cooldown_secs = params.cooldown_secs.unwrap_or(DEFAULT_COOLDOWN_SECS);
        let max_immediate_per_hour = params
            .max_immediate_per_hour
            .unwrap_or(DEFAULT_MAX_IMMEDIATE_PER_HOUR);

        let subscription = FinanceSubscription {
            id: Uuid::new_v4(),
            owner: UserId(context.user_id.clone()),
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

        Ok(FinanceSubscribeResult {
            subscription_id,
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
        FinanceListSubscriptionsTool, FinanceSubscribeParams, FinanceSubscribeTool,
        FinanceUnsubscribeParams, FinanceUnsubscribeTool,
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
}
