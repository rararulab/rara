// Copyright 2025 Crrow
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

//! Layer 2 service tools for memory retrieval, writing, and recall strategy
//! management.

use std::sync::Arc;

use async_trait::async_trait;
use rara_kernel::tool::AgentTool;
use rara_memory::{
    MemoryManager,
    recall_engine::{
        InjectTarget, RecallAction, RecallRule, RecallRuleUpdate, RecallStrategyEngine, Trigger,
    },
};
use serde_json::json;

/// Search unified memory layer (mem0 + Hindsight, fused with RRF).
pub struct MemorySearchTool {
    manager: Arc<MemoryManager>,
}

impl MemorySearchTool {
    /// Create a `memory_search` tool.
    pub fn new(manager: Arc<MemoryManager>) -> Self { Self { manager } }
}

#[async_trait]
impl AgentTool for MemorySearchTool {
    fn name(&self) -> &str { "memory_search" }

    fn description(&self) -> &str {
        "Search long-term memory across mem0 and Hindsight. Returns relevant memories with source \
         and content."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Keyword query for searching memory"
                },
                "limit": {
                    "type": "number",
                    "description": "Maximum number of results (default 8, max 50)"
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let query = params
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: query"))?;

        let limit = params
            .get("limit")
            .and_then(|v| v.as_u64())
            .map_or(8_usize, |v| v as usize)
            .clamp(1, 50);

        let results = self
            .manager
            .search(query, limit)
            .await
            .map_err(|e| anyhow::anyhow!("memory search failed: {e}"))?;

        Ok(json!({
            "query": query,
            "count": results.len(),
            "results": results
                .iter()
                .map(|r| json!({
                    "id": r.id,
                    "source": format!("{:?}", r.source),
                    "content": r.content,
                    "score": r.score,
                }))
                .collect::<Vec<_>>()
        }))
    }
}

/// Deep recall from Hindsight memory network.
pub struct MemoryDeepRecallTool {
    manager: Arc<MemoryManager>,
}

impl MemoryDeepRecallTool {
    /// Create a `memory_deep_recall` tool.
    pub fn new(manager: Arc<MemoryManager>) -> Self { Self { manager } }
}

#[async_trait]
impl AgentTool for MemoryDeepRecallTool {
    fn name(&self) -> &str { "memory_deep_recall" }

    fn description(&self) -> &str {
        "Deep recall from Hindsight memory network. Triggers deep reasoning over the memory bank \
         for a given query."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Query for deep recall reasoning"
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let query = params
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: query"))?;

        let result = self
            .manager
            .deep_recall(query)
            .await
            .map_err(|e| anyhow::anyhow!("memory deep recall failed: {e}"))?;

        Ok(json!({
            "query": query,
            "result": result,
        }))
    }
}

/// Add a structured fact about the user to long-term memory (mem0).
pub struct MemoryAddFactTool {
    manager: Arc<MemoryManager>,
}

impl MemoryAddFactTool {
    /// Create a `memory_add_fact` tool.
    pub fn new(manager: Arc<MemoryManager>) -> Self { Self { manager } }
}

#[async_trait]
impl AgentTool for MemoryAddFactTool {
    fn name(&self) -> &str { "memory_add_fact" }

    fn description(&self) -> &str {
        "Add a structured fact about the user to long-term memory (mem0). Facts are \
         auto-deduplicated."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "content": {
                    "type": "string",
                    "description": "The fact or information to store about the user"
                }
            },
            "required": ["content"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let content = params
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: content"))?;

        self.manager
            .add_fact(content)
            .await
            .map_err(|e| anyhow::anyhow!("memory add fact failed: {e}"))?;

        Ok(json!({
            "status": "ok",
            "message": "Fact stored in long-term memory",
        }))
    }
}

/// Write a note to Memos (persistent Markdown note storage).
pub struct MemoryWriteTool {
    manager: Arc<MemoryManager>,
}

impl MemoryWriteTool {
    /// Create a `memory_write` tool.
    pub fn new(manager: Arc<MemoryManager>) -> Self { Self { manager } }
}

#[async_trait]
impl AgentTool for MemoryWriteTool {
    fn name(&self) -> &str { "memory_write" }

    fn description(&self) -> &str {
        "Write a Markdown note to Memos for long-term storage. Notes are searchable via \
         memory_search."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "content": {
                    "type": "string",
                    "description": "Markdown content to write as a note"
                },
                "tags": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional tags for the note (e.g. ['meeting', 'project-x'])"
                }
            },
            "required": ["content"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let content = params
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: content"))?;

        let tags: Vec<&str> = params
            .get("tags")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>())
            .unwrap_or_default();

        let name = self
            .manager
            .write_note(content, &tags)
            .await
            .map_err(|e| anyhow::anyhow!("memory write failed: {e}"))?;

        Ok(json!({
            "status": "ok",
            "name": name,
        }))
    }
}

// ---------------------------------------------------------------------------
// Recall Strategy Tools
// ---------------------------------------------------------------------------

/// Register a new recall strategy rule.
pub struct RecallStrategyAddTool {
    engine: Arc<RecallStrategyEngine>,
}

impl RecallStrategyAddTool {
    /// Create a `recall_strategy_add` tool.
    pub fn new(engine: Arc<RecallStrategyEngine>) -> Self { Self { engine } }
}

#[async_trait]
impl AgentTool for RecallStrategyAddTool {
    fn name(&self) -> &str { "recall_strategy_add" }

    fn description(&self) -> &str {
        "Register a new recall strategy rule. Rules control when and how memory is queried and \
         injected into the system prompt."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Human-readable name for the rule"
                },
                "trigger": {
                    "type": "object",
                    "description": "Trigger condition (JSON). Examples: {\"type\":\"Always\"}, {\"type\":\"KeywordMatch\",\"keywords\":[\"rust\"]}, {\"type\":\"Event\",\"kind\":\"Compaction\"}, {\"type\":\"EveryNTurns\",\"n\":3}"
                },
                "action": {
                    "type": "object",
                    "description": "Recall action (JSON). Examples: {\"type\":\"Search\",\"query_template\":\"{user_text}\",\"limit\":5}, {\"type\":\"GetProfile\"}, {\"type\":\"DeepRecall\",\"query_template\":\"{user_text}\"}"
                },
                "inject": {
                    "type": "string",
                    "description": "Where to inject results: 'SystemPrompt' or 'ContextMessage'",
                    "enum": ["SystemPrompt", "ContextMessage"]
                },
                "priority": {
                    "type": "number",
                    "description": "Priority (lower = higher priority, default 100)"
                }
            },
            "required": ["name", "trigger", "action"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: name"))?;

        let trigger: Trigger = serde_json::from_value(
            params
                .get("trigger")
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: trigger"))?,
        )
        .map_err(|e| anyhow::anyhow!("invalid trigger: {e}"))?;

        let action: RecallAction = serde_json::from_value(
            params
                .get("action")
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: action"))?,
        )
        .map_err(|e| anyhow::anyhow!("invalid action: {e}"))?;

        let inject = match params.get("inject").and_then(|v| v.as_str()) {
            Some("ContextMessage") => InjectTarget::ContextMessage,
            _ => InjectTarget::SystemPrompt,
        };

        let priority = params
            .get("priority")
            .and_then(|v| v.as_u64())
            .map_or(100_u16, |v| v as u16);

        let id = uuid::Uuid::new_v4().to_string();

        let rule = RecallRule {
            id: id.clone(),
            name: name.to_owned(),
            trigger,
            action,
            inject,
            priority,
            enabled: true,
        };

        self.engine.add_rule(rule).await;

        Ok(json!({
            "id": id,
            "status": "ok",
        }))
    }
}

/// List all recall strategy rules.
pub struct RecallStrategyListTool {
    engine: Arc<RecallStrategyEngine>,
}

impl RecallStrategyListTool {
    /// Create a `recall_strategy_list` tool.
    pub fn new(engine: Arc<RecallStrategyEngine>) -> Self { Self { engine } }
}

#[async_trait]
impl AgentTool for RecallStrategyListTool {
    fn name(&self) -> &str { "recall_strategy_list" }

    fn description(&self) -> &str {
        "List all recall strategy rules with their triggers, actions, and status."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {}
        })
    }

    async fn execute(&self, _params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let rules = self.engine.list_rules().await;

        let rules_json: Vec<serde_json::Value> = rules
            .iter()
            .map(|r| {
                json!({
                    "id": r.id,
                    "name": r.name,
                    "trigger": serde_json::to_value(&r.trigger).unwrap_or_default(),
                    "action": serde_json::to_value(&r.action).unwrap_or_default(),
                    "inject": serde_json::to_value(&r.inject).unwrap_or_default(),
                    "priority": r.priority,
                    "enabled": r.enabled,
                })
            })
            .collect();

        Ok(json!({
            "count": rules_json.len(),
            "rules": rules_json,
        }))
    }
}

/// Update an existing recall strategy rule.
pub struct RecallStrategyUpdateTool {
    engine: Arc<RecallStrategyEngine>,
}

impl RecallStrategyUpdateTool {
    /// Create a `recall_strategy_update` tool.
    pub fn new(engine: Arc<RecallStrategyEngine>) -> Self { Self { engine } }
}

#[async_trait]
impl AgentTool for RecallStrategyUpdateTool {
    fn name(&self) -> &str { "recall_strategy_update" }

    fn description(&self) -> &str {
        "Update an existing recall strategy rule. Only the provided fields are changed."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "ID of the rule to update"
                },
                "enabled": {
                    "type": "boolean",
                    "description": "Enable or disable the rule"
                },
                "priority": {
                    "type": "number",
                    "description": "New priority value"
                },
                "trigger": {
                    "type": "object",
                    "description": "New trigger condition (JSON)"
                },
                "action": {
                    "type": "object",
                    "description": "New recall action (JSON)"
                }
            },
            "required": ["id"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let id = params
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: id"))?;

        let trigger: Option<Trigger> = params
            .get("trigger")
            .map(|v| serde_json::from_value(v.clone()))
            .transpose()
            .map_err(|e| anyhow::anyhow!("invalid trigger: {e}"))?;

        let action: Option<RecallAction> = params
            .get("action")
            .map(|v| serde_json::from_value(v.clone()))
            .transpose()
            .map_err(|e| anyhow::anyhow!("invalid action: {e}"))?;

        let enabled = params.get("enabled").and_then(|v| v.as_bool());
        let priority = params
            .get("priority")
            .and_then(|v| v.as_u64())
            .map(|v| v as u16);

        let update = RecallRuleUpdate {
            trigger,
            action,
            inject: None,
            priority,
            enabled,
        };

        let found = self.engine.update_rule(id, update).await;

        Ok(json!({
            "status": if found { "ok" } else { "not_found" },
        }))
    }
}

/// Remove a recall strategy rule.
pub struct RecallStrategyRemoveTool {
    engine: Arc<RecallStrategyEngine>,
}

impl RecallStrategyRemoveTool {
    /// Create a `recall_strategy_remove` tool.
    pub fn new(engine: Arc<RecallStrategyEngine>) -> Self { Self { engine } }
}

#[async_trait]
impl AgentTool for RecallStrategyRemoveTool {
    fn name(&self) -> &str { "recall_strategy_remove" }

    fn description(&self) -> &str { "Remove a recall strategy rule by ID." }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "ID of the rule to remove"
                }
            },
            "required": ["id"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let id = params
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: id"))?;

        let found = self.engine.remove_rule(id).await;

        Ok(json!({
            "status": if found { "ok" } else { "not_found" },
        }))
    }
}
