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
pub(crate) mod task;

mod background_common;

use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use serde::de::DeserializeOwned;

/// Tool names that background/child agents must never have access to,
/// preventing recursive subagent spawning.
pub(crate) const RECURSIVE_TOOL_DENYLIST: &[&str] = &[
    crate::tool_names::TASK,
    crate::tool_names::SPAWN_BACKGROUND,
    crate::tool_names::CREATE_PLAN,
    crate::tool_names::ASK_USER,
];

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

/// Hint that a tool can attach to its output to influence agent loop behavior.
///
/// Tools return hints via [`ToolOutput::hints`]; the agent loop inspects them
/// after execution and acts accordingly. This keeps orchestration decisions
/// in the loop while letting tools signal intent declaratively.
///
/// **Current limitation**: The `ToolDef` derive macro calls
/// [`ToolOutput::from_serialize`] which always returns empty hints. Tools
/// using the macro cannot set hints via `ToolExecute::run()`. The agent loop
/// therefore uses tool-name detection as a pragmatic workaround for known
/// hint-worthy tools (e.g. `marketplace-install` → `SuggestFold`). Once the
/// macro supports hint propagation, tool-name checks should be replaced.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum ToolHint {
    /// Suggest that the agent loop should fold (compress) context after this
    /// iteration completes. Useful when a tool produces large output that is
    /// no longer needed verbatim in subsequent turns (e.g. plugin installation
    /// logs).
    SuggestFold {
        /// Optional human-readable reason for logging/diagnostics.
        reason: Option<String>,
    },
}

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
    /// Hints for the agent loop (e.g. suggest context folding).
    pub hints:     Vec<ToolHint>,
}

impl ToolOutput {
    /// Create a `ToolOutput` by serializing a typed result struct.
    ///
    /// The resulting `ToolOutput` has empty `resources` and `hints` lists.
    /// Tools that need to attach binary resources (e.g. screenshots) should
    /// use `execute_fn` and construct `ToolOutput` directly instead.
    pub fn from_serialize<T: serde::Serialize>(val: &T) -> anyhow::Result<Self> {
        Ok(Self {
            json:      serde_json::to_value(val)?,
            resources: vec![],
            hints:     vec![],
        })
    }

    /// Attach a hint to this output, returning `self` for chaining.
    #[must_use]
    pub fn with_hint(mut self, hint: ToolHint) -> Self {
        self.hints.push(hint);
        self
    }
}

impl From<serde_json::Value> for ToolOutput {
    fn from(json: serde_json::Value) -> Self {
        Self {
            json,
            resources: vec![],
            hints: vec![],
        }
    }
}

/// Reference-counted handle to an agent tool.
pub type AgentToolRef = Arc<dyn AgentTool>;

/// Status of a `discover-tools` invocation.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscoverToolsStatus {
    /// At least one deferred tool was activated.
    Activated,
    /// No tools or skills matched the query.
    NoMatches,
    /// Skills matched but no deferred tools were activated.
    SkillsOnly,
}

/// Typed result returned by the `discover-tools` tool.
///
/// Shared between the tool implementation (serializes) and the agent loop
/// (deserializes), so schema changes cause compile errors on both sides.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct DiscoverToolsResult {
    /// Outcome of the discovery query.
    pub status:  DiscoverToolsStatus,
    /// Tool entries that were discovered (empty on no_matches).
    #[serde(default)]
    pub tools:   Vec<DiscoveredToolEntry>,
    /// Skill entries matching the query (informational — read SKILL.md to use).
    #[serde(default)]
    pub skills:  Vec<DiscoveredSkillEntry>,
    /// Human-readable message for the LLM.
    pub message: String,
}

/// A single skill entry in a [`DiscoverToolsResult`].
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct DiscoveredSkillEntry {
    /// The skill name.
    pub name:        String,
    /// One-line description of the skill.
    #[serde(default)]
    pub description: String,
    /// Filesystem path to the skill directory (read SKILL.md inside).
    #[serde(default)]
    pub path:        String,
}

/// A single tool entry in a [`DiscoverToolsResult`].
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct DiscoveredToolEntry {
    /// The tool name used for activation.
    pub name:        String,
    /// One-line description of the tool.
    #[serde(default)]
    pub description: String,
    /// Compact parameter summary (e.g. `"query (string, required), source
    /// (string)"`). Empty when schema is unavailable.
    #[serde(default)]
    pub parameters:  String,
}

/// Convert a JSON Schema `parameters_schema` value into a compact one-line
/// summary suitable for LLM consumption.
///
/// Example output: `query (string, required), source (string), limit (integer)`
pub fn summarize_parameters(schema: &serde_json::Value) -> String {
    let properties = match schema.get("properties").and_then(|p| p.as_object()) {
        Some(props) => props,
        None => return String::new(),
    };
    let required: Vec<&str> = schema
        .get("required")
        .and_then(|r| r.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();

    let (mut req_parts, mut opt_parts): (Vec<_>, Vec<_>) = properties
        .iter()
        .map(|(name, prop)| {
            let ty = prop.get("type").and_then(|t| t.as_str()).unwrap_or("any");
            if required.contains(&name.as_str()) {
                (format!("{name} ({ty}) [required]"), true)
            } else {
                (format!("{name} ({ty})"), false)
            }
        })
        .partition(|(_, is_req)| *is_req);
    req_parts.sort();
    opt_parts.sort();
    let mut labels: Vec<&str> = req_parts.iter().map(|(l, _)| l.as_str()).collect();
    labels.extend(opt_parts.iter().map(|(l, _)| l.as_str()));
    labels.join(", ")
}

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
    /// Live tool registry for the current session (includes dynamic MCP tools).
    /// Used by `discover-tools` to query the deferred catalog at runtime.
    pub tool_registry:         Option<ToolRegistryRef>,
    /// Stream handle for emitting real-time output during execution.
    /// `None` when streaming is not available (e.g. background tasks).
    pub stream_handle:         Option<crate::io::StreamHandle>,
    /// The tool call ID assigned by the LLM for this invocation.
    /// Used to correlate streaming output with the tool call.
    pub tool_call_id:          Option<String>,
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
            .field("tool_registry", &self.tool_registry.as_ref().map(|_| "..."))
            .field("stream_handle", &self.stream_handle.as_ref().map(|_| "..."))
            .field("tool_call_id", &self.tool_call_id)
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

    /// Per-tool execution timeout override.
    ///
    /// Returns `None` to use the kernel's `default_tool_timeout`.
    /// Tools with internal timeout management (e.g. bash with its own 120s
    /// timeout) should return a value larger than their internal timeout
    /// so the internal mechanism fires first.
    fn execution_timeout(&self) -> Option<std::time::Duration> { None }

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

    /// Return sorted names of all Deferred tools (regardless of activation).
    /// Used to inject the tool name list into the agent system prompt.
    #[must_use]
    pub fn deferred_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .tools
            .values()
            .filter(|tool| tool.tier() == ToolTier::Deferred)
            .map(|tool| tool.name().to_string())
            .collect();
        names.sort_unstable();
        names
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

    /// Create a manifest-scoped registry.
    ///
    /// Behaves like [`Self::filtered`], with one deferred-tools exception:
    /// if the manifest allowlist includes `discover-tools`, all deferred tools
    /// are retained so they can be discovered and activated at runtime.
    #[must_use]
    pub fn filtered_for_manifest(&self, tool_names: &[String]) -> Self {
        if tool_names.is_empty() || tool_names.iter().any(|n| n == "*") {
            return self.clone();
        }
        let allow: std::collections::HashSet<&str> =
            tool_names.iter().map(String::as_str).collect();
        let keep_deferred = allow.contains("discover-tools");
        let mut new = Self::new();
        for (name, tool) in &self.tools {
            if allow.contains(name.as_str()) || (keep_deferred && tool.tier() == ToolTier::Deferred)
            {
                new.register(Arc::clone(tool));
            }
        }
        new
    }

    /// Create a new registry excluding the named tools.
    #[must_use]
    pub fn without(&self, excluded: &[String]) -> Self {
        let deny: std::collections::HashSet<&str> = excluded.iter().map(String::as_str).collect();
        let mut new = Self::new();
        for (name, tool) in &self.tools {
            if !deny.contains(name.as_str()) {
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
    fn without_excludes_named_tools() {
        let reg = build_registry();
        let filtered = reg.without(&["bash".to_string(), "http-fetch".to_string()]);
        assert_eq!(filtered.len(), 2);
        assert!(filtered.get("bash").is_none());
        assert!(filtered.get("http-fetch").is_none());
        assert!(filtered.get("read-file").is_some());
        assert!(filtered.get("write-file").is_some());
    }

    #[test]
    fn without_empty_returns_all() {
        let reg = build_registry();
        let filtered = reg.without(&[]);
        assert_eq!(filtered.len(), 4, "empty denylist should return all tools");
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

    #[test]
    fn deferred_names_returns_sorted_names() {
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
        reg.register(Arc::new(DeferredTool));
        reg.register(Arc::new(CoreTool));

        let names = reg.deferred_names();
        assert_eq!(names, vec!["deferred-tool"]);
    }

    #[test]
    fn discover_tools_result_deserializes_activated() {
        let json = serde_json::json!({
            "status": "activated",
            "tools": [
                {"name": "send-email", "description": "Send email"},
                {"name": "send-image", "description": "Send image"},
            ],
            "message": "Activated 2 tool(s)."
        });
        let result: DiscoverToolsResult = serde_json::from_value(json).unwrap();
        assert_eq!(result.status, DiscoverToolsStatus::Activated);
        assert_eq!(result.tools.len(), 2);
        assert_eq!(result.tools[0].name, "send-email");
        assert_eq!(result.tools[1].name, "send-image");
    }

    #[test]
    fn discover_tools_result_deserializes_no_matches() {
        let json = serde_json::json!({
            "status": "no_matches",
            "tools": [],
            "message": "No deferred tools match 'xyz'."
        });
        let result: DiscoverToolsResult = serde_json::from_value(json).unwrap();
        assert_eq!(result.status, DiscoverToolsStatus::NoMatches);
        assert!(result.tools.is_empty());
    }

    #[test]
    fn discover_tools_result_defaults_tools_when_missing() {
        let json = serde_json::json!({"status": "no_matches", "message": "nothing"});
        let result: DiscoverToolsResult = serde_json::from_value(json).unwrap();
        assert!(result.tools.is_empty());
    }

    #[test]
    fn summarize_parameters_extracts_compact_summary() {
        use super::summarize_parameters;

        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "query": {"type": "string"},
                "source": {"type": "string"},
                "limit": {"type": "integer"}
            },
            "required": ["query"]
        });
        let result = summarize_parameters(&schema);
        // Required params come first with [required] marker.
        assert!(
            result.contains("query (string) [required]"),
            "got: {result}"
        );
        assert!(result.contains("source (string)"), "got: {result}");
        assert!(result.contains("limit (integer)"), "got: {result}");
        // Required params should appear before optional ones.
        let req_pos = result.find("query").expect("missing query");
        let opt_pos = result.find("limit").expect("missing limit");
        assert!(
            req_pos < opt_pos,
            "required should come first, got: {result}"
        );
        // Optional params should not have [required] marker.
        assert!(
            !result.contains("source (string) [required]"),
            "got: {result}"
        );
    }

    #[test]
    fn summarize_parameters_handles_empty_schema() {
        use super::summarize_parameters;

        let empty = serde_json::json!({"type": "object"});
        assert_eq!(summarize_parameters(&empty), "");

        let null = serde_json::Value::Null;
        assert_eq!(summarize_parameters(&null), "");
    }
}
