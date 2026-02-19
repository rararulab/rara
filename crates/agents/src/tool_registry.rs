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

use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use snafu::ResultExt;

use crate::err::prelude::*;

pub type AgentToolRef = Arc<dyn AgentTool>;

/// Agent-callable tool.
#[async_trait]
pub trait AgentTool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters_schema(&self) -> serde_json::Value;
    async fn execute(&self, params: serde_json::Value) -> Result<serde_json::Value>;
}

/// Where a tool originates from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolSource {
    /// Built-in tool shipped with the binary.
    Builtin,
    /// Tool provided by an MCP server.
    Mcp { server: String },
}

/// Architectural layer a tool belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolLayer {
    /// Atomic primitive operation (db, http, notify, storage).
    Primitive,
    /// Complex business workflow (MCP service).
    Service,
}

/// Internal entry pairing a tool with its source and layer metadata.
struct ToolEntry {
    tool:   AgentToolRef,
    source: ToolSource,
    layer:  ToolLayer,
}

/// Registry of available tools for an agent run.
pub struct ToolRegistry {
    tools: HashMap<String, ToolEntry>,
}

impl ToolRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    /// Register a built-in primitive tool (Layer 1).
    pub fn register_primitive(&mut self, tool: AgentToolRef) -> Option<AgentToolRef> {
        self.register(tool, ToolSource::Builtin, ToolLayer::Primitive)
    }

    /// Register a built-in service tool (Layer 2).
    pub fn register_service(&mut self, tool: AgentToolRef) -> Option<AgentToolRef> {
        self.register(tool, ToolSource::Builtin, ToolLayer::Service)
    }

    /// Register a built-in tool. Defaults to [`ToolLayer::Primitive`].
    pub fn register_builtin(&mut self, tool: AgentToolRef) -> Option<AgentToolRef> {
        self.register(tool, ToolSource::Builtin, ToolLayer::Primitive)
    }

    /// Register an MCP-provided tool. Defaults to [`ToolLayer::Service`].
    pub fn register_mcp(
        &mut self,
        tool: AgentToolRef,
        server: impl Into<String>,
    ) -> Option<AgentToolRef> {
        self.register(
            tool,
            ToolSource::Mcp {
                server: server.into(),
            },
            ToolLayer::Service,
        )
    }

    pub fn get(&self, name: &str) -> Option<&AgentToolRef> {
        self.tools.get(name).map(|entry| &entry.tool)
    }

    pub fn source_of(&self, name: &str) -> Option<&ToolSource> {
        self.tools.get(name).map(|entry| &entry.source)
    }

    pub fn layer_of(&self, name: &str) -> Option<ToolLayer> {
        self.tools.get(name).map(|entry| entry.layer)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool { self.tools.is_empty() }

    #[must_use]
    pub fn len(&self) -> usize { self.tools.len() }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &AgentToolRef, &ToolSource, ToolLayer)> {
        self.tools
            .iter()
            .map(|(name, entry)| (name.as_str(), &entry.tool, &entry.source, entry.layer))
    }

    pub fn to_openrouter_tools(&self) -> Result<Vec<openrouter_rs::types::Tool>> {
        self.tools
            .values()
            .map(|entry| {
                openrouter_rs::types::Tool::builder()
                    .name(entry.tool.name())
                    .description(entry.tool.description())
                    .parameters(entry.tool.parameters_schema())
                    .build()
                    .context(OpenRouterSnafu)
            })
            .collect()
    }

    /// Convert only the named tools to OpenRouter format.
    /// If `tool_names` is empty, include ALL tools (no filtering).
    pub fn to_openrouter_tools_filtered(
        &self,
        tool_names: &[String],
    ) -> Result<Vec<openrouter_rs::types::Tool>> {
        if tool_names.is_empty() {
            return self.to_openrouter_tools();
        }
        self.tools
            .values()
            .filter(|entry| tool_names.iter().any(|n| n == entry.tool.name()))
            .map(|entry| {
                openrouter_rs::types::Tool::builder()
                    .name(entry.tool.name())
                    .description(entry.tool.description())
                    .parameters(entry.tool.parameters_schema())
                    .build()
                    .context(OpenRouterSnafu)
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
            let mut new = Self::new();
            for entry in self.tools.values() {
                new.register(Arc::clone(&entry.tool), entry.source.clone(), entry.layer);
            }
            return new;
        }
        let mut new = Self::new();
        for (name, entry) in &self.tools {
            if tool_names.iter().any(|n| n == name) {
                new.register(Arc::clone(&entry.tool), entry.source.clone(), entry.layer);
            }
        }
        new
    }

    fn register(
        &mut self,
        tool: AgentToolRef,
        source: ToolSource,
        layer: ToolLayer,
    ) -> Option<AgentToolRef> {
        let name = tool.name().to_owned();
        self.tools
            .insert(
                name,
                ToolEntry {
                    tool,
                    source,
                    layer,
                },
            )
            .map(|entry| entry.tool)
    }
}

impl ToolRegistry {
    /// Create a registry pre-loaded with all built-in generic primitive tools.
    pub fn with_defaults() -> Self {
        use crate::tools::primitives::*;
        let mut registry = Self::new();
        registry.register_primitive(Arc::new(BashTool::new()));
        registry.register_primitive(Arc::new(ReadFileTool::new()));
        registry.register_primitive(Arc::new(WriteFileTool::new()));
        registry.register_primitive(Arc::new(EditFileTool::new()));
        registry.register_primitive(Arc::new(FindFilesTool::new()));
        registry.register_primitive(Arc::new(GrepTool::new()));
        registry.register_primitive(Arc::new(ListDirectoryTool::new()));
        registry.register_primitive(Arc::new(HttpFetchTool::new()));
        registry
    }
}

impl Default for ToolRegistry {
    fn default() -> Self { Self::new() }
}
