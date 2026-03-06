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

use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;

/// Reference-counted handle to an agent tool.
pub type AgentToolRef = Arc<dyn AgentTool>;

/// Shared reference to the [`ToolRegistry`].
pub type ToolRegistryRef = Arc<ToolRegistry>;

/// Execution context passed to every tool invocation.
///
/// Provides ambient session metadata (e.g. the authenticated user) so tools
/// do not need to rely on LLM-supplied identity parameters.
#[derive(Clone, Default)]
pub struct ToolContext {
    /// The authenticated user identifier for the current session.
    /// `None` when the session has no resolved principal (e.g. anonymous).
    pub user_id: Option<String>,
    /// The session key for the current conversation turn.
    pub session_key: Option<crate::session::SessionKey>,
    /// The originating endpoint (e.g. Telegram chat) for routing replies.
    pub origin_endpoint: Option<crate::io::Endpoint>,
    /// Event queue for pushing outbound events.
    pub event_queue: Option<crate::queue::EventQueueRef>,
}

impl std::fmt::Debug for ToolContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolContext")
            .field("user_id", &self.user_id)
            .field("session_key", &self.session_key)
            .field("origin_endpoint", &self.origin_endpoint)
            .field("event_queue", &self.event_queue.as_ref().map(|_| "..."))
            .finish()
    }
}

/// Agent-callable tool.
#[async_trait]
pub trait AgentTool: Send + Sync {
    /// Unique name of the tool.
    fn name(&self) -> &str;

    /// Human-readable description of the tool's purpose.
    fn description(&self) -> &str;

    /// JSON Schema describing the accepted parameters.
    fn parameters_schema(&self) -> serde_json::Value;

    /// Execute the tool with the given parameters and execution context.
    async fn execute(
        &self,
        params: serde_json::Value,
        context: &ToolContext,
    ) -> anyhow::Result<serde_json::Value>;
}

/// Registry of available tools for an agent run.
#[derive(Clone)]
pub struct ToolRegistry {
    tools: HashMap<String, AgentToolRef>,
}

impl ToolRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    /// Register a tool. Returns the previously registered tool with the same
    /// name, if any.
    pub fn register(&mut self, tool: AgentToolRef) -> Option<AgentToolRef> {
        let name = tool.name().to_owned();
        self.tools.insert(name, tool)
    }

    pub fn get(&self, name: &str) -> Option<&AgentToolRef> { self.tools.get(name) }

    #[must_use]
    pub fn is_empty(&self) -> bool { self.tools.is_empty() }

    #[must_use]
    pub fn len(&self) -> usize { self.tools.len() }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &AgentToolRef)> {
        self.tools.iter().map(|(name, tool)| (name.as_str(), tool))
    }

    /// Convert all tools to [`llm::ToolDefinition`] format for the
    /// [`LlmDriver`](crate::llm::LlmDriver) path.
    #[must_use]
    pub fn to_llm_tool_definitions(&self) -> Vec<crate::llm::ToolDefinition> {
        self.tools
            .values()
            .map(|tool| crate::llm::ToolDefinition {
                name:        tool.name().to_string(),
                description: tool.description().to_string(),
                parameters:  tool.parameters_schema(),
            })
            .collect()
    }

    /// Return the names of all registered tools.
    #[must_use]
    pub fn tool_names(&self) -> Vec<String> { self.tools.keys().cloned().collect() }

    /// Create a new registry containing only the named tools.
    /// If `tool_names` is empty, returns a clone of all tools.
    #[must_use]
    pub fn filtered(&self, tool_names: &[String]) -> Self {
        if tool_names.is_empty() {
            return self.clone();
        }
        let mut new = Self::new();
        for (name, tool) in &self.tools {
            if tool_names.iter().any(|n| n == name) {
                new.register(Arc::clone(tool));
            }
        }
        new
    }
}

impl Default for ToolRegistry {
    fn default() -> Self { Self::new() }
}
