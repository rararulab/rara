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

use std::collections::{HashMap, HashSet};

use rara_mcp::manager::registry::McpServerConfig;
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
pub struct McpServerInfo {
    pub name:   String,
    pub config: McpServerConfigView,
    pub status: McpServerStatus,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum McpServerStatus {
    Connected,
    Connecting,
    Disconnected,
    Error { message: String },
}

#[derive(Serialize)]
pub struct McpServerConfigView {
    pub command:              String,
    pub args:                 Vec<String>,
    pub env:                  HashMap<String, String>,
    pub enabled:              bool,
    pub transport:            String,
    pub url:                  Option<String>,
    pub startup_timeout_secs: Option<u64>,
    pub tool_timeout_secs:    Option<u64>,
    pub tools_enabled:        Option<HashSet<String>>,
    pub tools_disabled:       HashSet<String>,
}

impl From<McpServerConfig> for McpServerConfigView {
    fn from(c: McpServerConfig) -> Self {
        Self {
            command:              c.command,
            args:                 c.args,
            env:                  c.env,
            enabled:              c.enabled,
            transport:            format!("{:?}", c.transport).to_lowercase(),
            url:                  c.url,
            startup_timeout_secs: c.startup_timeout_secs,
            tool_timeout_secs:    c.tool_timeout_secs,
            tools_enabled:        c.tools_enabled,
            tools_disabled:       c.tools_disabled,
        }
    }
}

#[derive(Deserialize)]
pub struct CreateServerRequest {
    pub name:   String,
    #[serde(flatten)]
    pub config: McpServerConfig,
}

#[derive(Deserialize)]
pub struct UpdateServerRequest {
    #[serde(flatten)]
    pub config: McpServerConfig,
}

#[derive(Serialize)]
pub struct McpToolView {
    pub name:         String,
    pub description:  Option<String>,
    pub input_schema: serde_json::Value,
}

#[derive(Serialize)]
pub struct McpResourceView {
    pub uri:         String,
    pub name:        Option<String>,
    pub description: Option<String>,
    pub mime_type:   Option<String>,
}
