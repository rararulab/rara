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

pub(crate) mod create_plan;
pub(crate) mod schedule;
pub(crate) mod tape;

use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;

/// A binary resource produced by a tool (e.g. a compressed screenshot).
#[derive(Debug, Clone)]
pub struct ResourceAttachment {
    /// MIME type of the resource (e.g. `"image/jpeg"`).
    pub media_type: String,
    /// Raw bytes of the resource (already compressed if applicable).
    pub data:       Vec<u8>,
}

/// Output of a tool execution — a JSON result plus optional resource
/// attachments (images, files) that should be persisted separately and
/// fed to the LLM as multimodal content blocks.
#[derive(Debug, Clone)]
pub struct ToolOutput {
    /// JSON payload visible to the LLM as text.
    pub json:      serde_json::Value,
    /// Binary resources to persist and inject as multimodal content.
    pub resources: Vec<ResourceAttachment>,
}

impl From<serde_json::Value> for ToolOutput {
    fn from(json: serde_json::Value) -> Self {
        Self {
            json,
            resources: vec![],
        }
    }
}

/// Reference-counted handle to an agent tool.
pub type AgentToolRef = Arc<dyn AgentTool>;

/// Provider of tools that are discovered at runtime (e.g. MCP servers).
/// Implementors are called on every `GetToolRegistry` syscall to inject
/// dynamic tools into the registry.
#[async_trait]
pub trait DynamicToolProvider: Send + Sync {
    /// Return all currently available dynamic tools.
    async fn tools(&self) -> Vec<AgentToolRef>;
}

/// Shared reference to a dynamic tool provider.
pub type DynamicToolProviderRef = Arc<dyn DynamicToolProvider>;

/// Intercepts tool output before it is returned to the LLM.
///
/// Used for transparent output compression (e.g. indexing large outputs into
/// a knowledge base and returning a compact reference).
#[async_trait]
pub trait OutputInterceptor: Send + Sync {
    /// Optionally transform a tool's output. Receives the tool name and the
    /// original output; returns the (possibly replaced) output.
    async fn intercept(&self, tool_name: &str, output: ToolOutput) -> ToolOutput;
}

/// Shared reference to an output interceptor.
pub type OutputInterceptorRef = Arc<dyn OutputInterceptor>;

/// A dynamically-swappable output interceptor.
/// Wraps `Option<OutputInterceptorRef>` behind a lock so it can be updated
/// at runtime (e.g. when context-mode MCP reconnects).
pub type DynamicOutputInterceptor = Arc<tokio::sync::RwLock<Option<OutputInterceptorRef>>>;

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
    pub user_id:         Option<String>,
    /// The session key for the current conversation turn.
    pub session_key:     Option<crate::session::SessionKey>,
    /// The originating endpoint (e.g. Telegram chat) for routing replies.
    pub origin_endpoint: Option<crate::io::Endpoint>,
    /// Event queue for pushing outbound events.
    pub event_queue:     Option<crate::queue::EventQueueRef>,
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
    ) -> anyhow::Result<ToolOutput>;
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

    /// Create a new registry containing only tools the user is authorized to
    /// use (based on [`KernelUser::can_use_tool`]).
    #[must_use]
    pub fn filtered_by_user(&self, user: &crate::identity::KernelUser) -> Self {
        let mut new = Self::new();
        for (name, tool) in &self.tools {
            if user.can_use_tool(name) {
                new.register(Arc::clone(tool));
            }
        }
        new
    }

    /// Create a new registry containing only the named tools.
    ///
    /// - If `tool_names` is empty or contains `"*"`, returns a clone of all
    ///   tools (no filtering).
    /// - Otherwise only tools whose name appears in `tool_names` are kept.
    #[must_use]
    pub fn filtered(&self, tool_names: &[String]) -> Self {
        if tool_names.is_empty() || tool_names.iter().any(|n| n == "*") {
            return self.clone();
        }
        let allow: std::collections::HashSet<&str> =
            tool_names.iter().map(String::as_str).collect();
        let mut new = Self::new();
        for (name, tool) in &self.tools {
            if allow.contains(name.as_str()) {
                new.register(Arc::clone(tool));
            }
        }
        new
    }
}

impl Default for ToolRegistry {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;
    use crate::identity::{KernelUser, Permission, Role};

    struct DummyTool {
        name: String,
    }

    #[async_trait]
    impl AgentTool for DummyTool {
        fn name(&self) -> &str { &self.name }

        fn description(&self) -> &str { "test tool" }

        fn parameters_schema(&self) -> serde_json::Value { serde_json::json!({"type": "object"}) }

        async fn execute(
            &self,
            _params: serde_json::Value,
            _context: &ToolContext,
        ) -> anyhow::Result<ToolOutput> {
            Ok(serde_json::json!({"ok": true}).into())
        }
    }

    fn build_registry() -> ToolRegistry {
        let mut reg = ToolRegistry::new();
        for name in ["bash", "http-fetch", "read-file", "write-file"] {
            reg.register(Arc::new(DummyTool { name: name.into() }));
        }
        reg
    }

    #[test]
    fn filtered_by_user_removes_unauthorized_tools() {
        let reg = build_registry();
        let user = KernelUser {
            name:        "regular".into(),
            role:        Role::User,
            permissions: vec![],
            enabled:     true,
        };
        let filtered = reg.filtered_by_user(&user);
        assert!(
            filtered.is_empty(),
            "Role::User with no permissions should have no tools"
        );
    }

    #[test]
    fn filtered_by_user_keeps_all_for_admin() {
        let reg = build_registry();
        let user = KernelUser {
            name:        "admin".into(),
            role:        Role::Admin,
            permissions: vec![Permission::All],
            enabled:     true,
        };
        let filtered = reg.filtered_by_user(&user);
        assert_eq!(filtered.len(), 4);
    }

    #[test]
    fn filtered_by_user_keeps_specific_tools() {
        let reg = build_registry();
        let user = KernelUser {
            name:        "limited".into(),
            role:        Role::User,
            permissions: vec![
                Permission::Spawn,
                Permission::UseTool("http-fetch".into()),
                Permission::UseTool("read-file".into()),
            ],
            enabled:     true,
        };
        let filtered = reg.filtered_by_user(&user);
        assert_eq!(filtered.len(), 2);
        assert!(filtered.get("http-fetch").is_some());
        assert!(filtered.get("read-file").is_some());
        assert!(filtered.get("bash").is_none());
        assert!(filtered.get("write-file").is_none());
    }

    #[test]
    fn filtered_empty_returns_all() {
        let reg = build_registry();
        let filtered = reg.filtered(&[]);
        assert_eq!(filtered.len(), 4, "empty allowlist should return all tools");
    }

    #[test]
    fn filtered_wildcard_returns_all() {
        let reg = build_registry();
        let filtered = reg.filtered(&["*".to_string()]);
        assert_eq!(filtered.len(), 4, "wildcard '*' should return all tools");
    }

    #[test]
    fn filtered_specific_names() {
        let reg = build_registry();
        let filtered = reg.filtered(&["bash".to_string(), "read-file".to_string()]);
        assert_eq!(filtered.len(), 2);
        assert!(filtered.get("bash").is_some());
        assert!(filtered.get("read-file").is_some());
        assert!(filtered.get("http-fetch").is_none());
    }

    #[test]
    fn filtered_unknown_names_ignored() {
        let reg = build_registry();
        let filtered = reg.filtered(&["nonexistent".to_string()]);
        assert!(filtered.is_empty(), "unknown tool names should result in empty registry");
    }

    struct TestInterceptor {
        called: AtomicBool,
    }

    #[async_trait]
    impl OutputInterceptor for TestInterceptor {
        async fn intercept(&self, _tool_name: &str, _output: ToolOutput) -> ToolOutput {
            self.called.store(true, Ordering::SeqCst);
            ToolOutput::from(serde_json::json!({ "intercepted": true }))
        }
    }

    #[tokio::test]
    async fn output_interceptor_is_called() {
        let interceptor = TestInterceptor {
            called: AtomicBool::new(false),
        };

        let output = ToolOutput::from(serde_json::json!({ "data": "original" }));
        let result = interceptor.intercept("test-tool", output).await;

        assert!(interceptor.called.load(Ordering::SeqCst));
        assert_eq!(result.json["intercepted"], true);
    }
}
