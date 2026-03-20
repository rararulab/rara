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

pub mod browser;
pub(crate) mod cancel_background;
pub(crate) mod create_plan;
pub(crate) mod fold_branch;
pub(crate) mod schedule;
pub(crate) mod spawn_background;
pub(crate) mod tape;

use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use serde::de::DeserializeOwned;

/// Typed tool execution trait.
///
/// Bridges strongly-typed parameter structs and the untyped
/// `AgentTool::execute` interface. The `ToolDef` derive macro generates an
/// `AgentTool` impl that deserializes `serde_json::Value` into `Self::Params`
/// and delegates to [`ToolExecute::run`].
#[async_trait]
pub trait ToolExecute: Send + Sync {
    /// The parameter struct for this tool.
    /// Must derive both `serde::Deserialize` and `schemars::JsonSchema`.
    type Params: DeserializeOwned + schemars::JsonSchema;

    /// The result struct for this tool. Must derive `serde::Serialize`.
    type Output: serde::Serialize;

    /// Execute the tool with typed parameters.
    async fn run(
        &self,
        params: Self::Params,
        context: &ToolContext,
    ) -> anyhow::Result<Self::Output>;
}

/// Empty parameter struct for tools that accept no parameters.
#[derive(serde::Deserialize, schemars::JsonSchema)]
pub struct EmptyParams {}

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

impl ToolOutput {
    /// Create a `ToolOutput` by serializing a typed result struct.
    ///
    /// The resulting `ToolOutput` has an empty `resources` list. Tools that
    /// need to attach binary resources (e.g. screenshots) should use
    /// `execute_fn` and construct `ToolOutput` directly instead.
    pub fn from_serialize<T: serde::Serialize>(val: &T) -> anyhow::Result<Self> {
        Ok(Self {
            json:      serde_json::to_value(val)?,
            resources: vec![],
        })
    }
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

    /// Optional system prompt fragment injected when this interceptor is
    /// active.
    ///
    /// Returns guidance text that teaches the LLM how to interact with
    /// intercepted (indexed) tool outputs.
    fn system_prompt_fragment(&self) -> Option<&str> { None }
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
#[derive(Clone)]
pub struct ToolContext {
    /// The authenticated user identifier for the current session.
    pub user_id:               String,
    /// The session key for the current conversation turn.
    pub session_key:           crate::session::SessionKey,
    /// The originating endpoint (e.g. Telegram chat) for routing replies.
    pub origin_endpoint:       Option<crate::io::Endpoint>,
    /// Event queue for pushing outbound events.
    pub event_queue:           crate::queue::EventQueueRef,
    /// The inbound message ID that triggered the current turn.
    pub rara_message_id:       crate::io::MessageId,
    /// Context window size in tokens for the current model.
    pub context_window_tokens: usize,
}

impl std::fmt::Debug for ToolContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolContext")
            .field("user_id", &self.user_id)
            .field("session_key", &self.session_key)
            .field("origin_endpoint", &self.origin_endpoint)
            .field("event_queue", &"...")
            .field("rara_message_id", &self.rara_message_id)
            .field("context_window_tokens", &self.context_window_tokens)
            .finish()
    }
}

/// Tool loading tier for deferred tool discovery.
///
/// `Core` tools are always included in LLM requests. `Deferred` tools are
/// only included after the LLM explicitly activates them via `discover-tools`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToolTier {
    /// Always sent in tool definitions.
    Core,
    /// Only sent after activation via `discover-tools`.
    Deferred,
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

    /// Whether this tool's output should bypass the output interceptor
    /// (e.g. context-mode indexing). Tools with binary, always-small, or
    /// write-only output should override this to return `true`.
    fn bypass_output_interceptor(&self) -> bool { false }

    /// The loading tier for this tool. Defaults to [`ToolTier::Core`].
    fn tier(&self) -> ToolTier { ToolTier::Core }
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

    /// Return tool definitions for only Core tools plus any tools in the
    /// `activated` set. This is the primary method used by the agent loop.
    #[must_use]
    pub fn to_llm_tool_definitions_active(
        &self,
        activated: &std::collections::HashSet<String>,
    ) -> Vec<crate::llm::ToolDefinition> {
        self.tools
            .values()
            .filter(|tool| tool.tier() == ToolTier::Core || activated.contains(tool.name()))
            .map(|tool| crate::llm::ToolDefinition {
                name:        tool.name().to_string(),
                description: tool.description().to_string(),
                parameters:  tool.parameters_schema(),
            })
            .collect()
    }

    /// Return a catalog of all Deferred tools that are NOT yet activated.
    /// Each entry is `(name, description)` for the discover-tools search.
    #[must_use]
    pub fn deferred_catalog(
        &self,
        activated: &std::collections::HashSet<String>,
    ) -> Vec<(String, String)> {
        self.tools
            .values()
            .filter(|tool| tool.tier() == ToolTier::Deferred && !activated.contains(tool.name()))
            .map(|tool| (tool.name().to_string(), tool.description().to_string()))
            .collect()
    }

    /// Return the names of all registered tools.
    #[must_use]
    pub fn tool_names(&self) -> Vec<String> { self.tools.keys().cloned().collect() }

    /// Create a new registry containing only tools the user is authorized to
    /// use (based on `KernelUser::can_use_tool`).
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

// Re-export the derive macro so tools can `use crate::tool::ToolDef`.
pub use rara_tool_macro::ToolDef;

/// Recursively clean a JSON Schema produced by `schemars` for LLM consumption.
///
/// - Removes: `$schema`, `title`, `definitions`/`$defs`
/// - Preserves: `type`, `properties`, `required`, `description`, `enum`,
///   `items`, `default`, `format`
/// - Inline-resolves all `$ref` pointers and then drops the definitions block.
pub fn clean_schema(schema: schemars::Schema) -> serde_json::Value {
    let mut value = serde_json::to_value(schema).unwrap_or(serde_json::Value::Null);

    // Extract definitions for $ref resolution before removing them.
    let definitions = extract_definitions(&value);

    // Resolve all $ref pointers inline.
    resolve_refs(&mut value, &definitions);

    // Strip noise fields.
    clean_value(&mut value);

    value
}

/// Extract the definitions/$defs map from the root schema.
fn extract_definitions(
    value: &serde_json::Value,
) -> std::collections::HashMap<String, serde_json::Value> {
    let mut defs = std::collections::HashMap::new();
    for key in &["definitions", "$defs"] {
        if let Some(serde_json::Value::Object(map)) = value.get(*key) {
            for (name, schema) in map {
                defs.insert(name.clone(), schema.clone());
            }
        }
    }
    defs
}

/// Recursively resolve `$ref` pointers by inlining the referenced definition.
fn resolve_refs(
    value: &mut serde_json::Value,
    definitions: &std::collections::HashMap<String, serde_json::Value>,
) {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(serde_json::Value::String(ref_path)) = map.get("$ref") {
                let def_name = ref_path.rsplit('/').next().unwrap_or("").to_string();
                if let Some(resolved) = definitions.get(&def_name) {
                    let mut resolved = resolved.clone();
                    resolve_refs(&mut resolved, definitions);
                    clean_value(&mut resolved);
                    *value = resolved;
                    return;
                }
            }
            for v in map.values_mut() {
                resolve_refs(v, definitions);
            }
        }
        serde_json::Value::Array(arr) => {
            for v in arr {
                resolve_refs(v, definitions);
            }
        }
        _ => {}
    }
}

/// Remove noise fields from a JSON Schema value.
fn clean_value(value: &mut serde_json::Value) {
    if let serde_json::Value::Object(map) = value {
        map.remove("$schema");
        map.remove("title");
        map.remove("definitions");
        map.remove("$defs");

        for v in map.values_mut() {
            clean_value(v);
        }
    } else if let serde_json::Value::Array(arr) = value {
        for v in arr {
            clean_value(v);
        }
    }
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
        assert!(
            filtered.is_empty(),
            "unknown tool names should result in empty registry"
        );
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

    #[test]
    fn clean_schema_removes_noise_fields() {
        #[derive(serde::Deserialize, schemars::JsonSchema)]
        struct TestParams {
            /// The name field
            name:  String,
            /// Optional count
            count: Option<u32>,
        }

        let cleaned = super::clean_schema(schemars::schema_for!(TestParams));

        // Noise fields must be gone.
        assert!(cleaned.get("$schema").is_none());
        assert!(cleaned.get("title").is_none());
        assert!(cleaned.get("definitions").is_none());
        assert!(cleaned.get("$defs").is_none());

        // Structure must be preserved.
        assert_eq!(cleaned["type"], "object");
        assert!(cleaned["properties"]["name"].is_object());
        assert!(cleaned["properties"]["count"].is_object());
        assert_eq!(
            cleaned["properties"]["name"]["description"],
            "The name field"
        );
    }

    #[test]
    fn clean_schema_resolves_refs() {
        #[derive(serde::Deserialize, schemars::JsonSchema)]
        enum Mode {
            Fast,
            Slow,
        }

        #[derive(serde::Deserialize, schemars::JsonSchema)]
        struct RefParams {
            /// The mode
            mode: Mode,
        }

        let cleaned = super::clean_schema(schemars::schema_for!(RefParams));

        // $ref should be resolved inline.
        let mode = &cleaned["properties"]["mode"];
        assert!(mode.get("$ref").is_none(), "refs should be inlined");
    }

    #[test]
    fn clean_schema_empty_params() {
        let cleaned = super::clean_schema(schemars::schema_for!(super::EmptyParams));
        assert_eq!(cleaned["type"], "object");
    }

    #[test]
    fn to_llm_tool_definitions_active_filters_by_tier() {
        use std::collections::HashSet;

        struct CoreTool;
        #[async_trait]
        impl AgentTool for CoreTool {
            fn name(&self) -> &str { "core-tool" }

            fn description(&self) -> &str { "A core tool" }

            fn parameters_schema(&self) -> serde_json::Value { serde_json::json!({}) }

            async fn execute(
                &self,
                _: serde_json::Value,
                _: &ToolContext,
            ) -> anyhow::Result<ToolOutput> {
                unimplemented!()
            }
        }

        struct DeferredTool;
        #[async_trait]
        impl AgentTool for DeferredTool {
            fn name(&self) -> &str { "deferred-tool" }

            fn description(&self) -> &str { "A deferred tool" }

            fn parameters_schema(&self) -> serde_json::Value { serde_json::json!({}) }

            async fn execute(
                &self,
                _: serde_json::Value,
                _: &ToolContext,
            ) -> anyhow::Result<ToolOutput> {
                unimplemented!()
            }

            fn tier(&self) -> ToolTier { ToolTier::Deferred }
        }

        let mut reg = ToolRegistry::new();
        reg.register(Arc::new(CoreTool));
        reg.register(Arc::new(DeferredTool));

        // Without activation: only core tool
        let empty = HashSet::new();
        let defs = reg.to_llm_tool_definitions_active(&empty);
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].name, "core-tool");

        // With activation: both tools
        let mut activated = HashSet::new();
        activated.insert("deferred-tool".to_string());
        let defs = reg.to_llm_tool_definitions_active(&activated);
        assert_eq!(defs.len(), 2);

        // Deferred catalog shows unactivated tools
        let catalog = reg.deferred_catalog(&empty);
        assert_eq!(catalog.len(), 1);
        assert_eq!(catalog[0].0, "deferred-tool");

        // Deferred catalog excludes activated tools
        let catalog = reg.deferred_catalog(&activated);
        assert!(catalog.is_empty());
    }
}
