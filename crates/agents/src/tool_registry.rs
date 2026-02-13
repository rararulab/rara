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

/// Internal entry pairing a tool with its source metadata.
struct ToolEntry {
    tool: AgentToolRef,
    source: ToolSource,
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

    pub fn register_builtin(&mut self, tool: AgentToolRef) -> Option<AgentToolRef> {
        self.register(tool, ToolSource::Builtin)
    }

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
        )
    }

    pub fn get(&self, name: &str) -> Option<&AgentToolRef> {
        self.tools.get(name).map(|entry| &entry.tool)
    }

    pub fn source_of(&self, name: &str) -> Option<&ToolSource> {
        self.tools.get(name).map(|entry| &entry.source)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.tools.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &AgentToolRef, &ToolSource)> {
        self.tools
            .iter()
            .map(|(name, entry)| (name.as_str(), &entry.tool, &entry.source))
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

    fn register(&mut self, tool: AgentToolRef, source: ToolSource) -> Option<AgentToolRef> {
        let name = tool.name().to_owned();
        self.tools
            .insert(name, ToolEntry { tool, source })
            .map(|entry| entry.tool)
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}
