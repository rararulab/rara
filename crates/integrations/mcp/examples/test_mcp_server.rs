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

//! Minimal MCP server for integration testing.
//!
//! Communicates over **stdio** and exposes:
//! - Two tools: `echo` (returns arguments) and `add` (sums two numbers).
//! - One resource: `test://greeting` (returns a greeting string).

use rmcp::{
    RoleServer, ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{
        Annotated, ListResourcesResult, RawResource, ReadResourceRequestParams, ReadResourceResult,
        ResourceContents, ServerCapabilities, ServerInfo,
    },
    schemars,
    service::{RequestContext, RunningService},
    tool, tool_handler, tool_router,
};

// ── Tool parameter types ────────────────────────────────────────────────

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct EchoRequest {
    /// The message to echo back.
    pub message: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AddRequest {
    /// Left operand.
    pub a: i32,
    /// Right operand.
    pub b: i32,
}

// ── Server ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct TestServer {
    tool_router: ToolRouter<Self>,
}

impl TestServer {
    pub fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }
}

#[tool_router]
impl TestServer {
    /// Echo the given message back to the caller.
    #[tool(name = "echo", description = "Echo a message back")]
    fn echo(&self, Parameters(req): Parameters<EchoRequest>) -> String { req.message }

    /// Add two integers and return the sum as a string.
    #[tool(name = "add", description = "Add two numbers")]
    fn add(&self, Parameters(req): Parameters<AddRequest>) -> String { (req.a + req.b).to_string() }
}

#[tool_handler]
impl ServerHandler for TestServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some("Test MCP server for integration tests".into()),
            capabilities: ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
            ..Default::default()
        }
    }

    async fn list_resources(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, rmcp::ErrorData> {
        Ok(ListResourcesResult {
            resources:   vec![Annotated {
                raw:         RawResource::new("test://greeting", "greeting"),
                annotations: None,
            }],
            next_cursor: None,
            meta:        None,
        })
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResult, rmcp::ErrorData> {
        if request.uri == "test://greeting" {
            Ok(ReadResourceResult {
                contents: vec![ResourceContents::text(
                    "Hello from test server!",
                    request.uri,
                )],
            })
        } else {
            Err(rmcp::ErrorData::resource_not_found(
                format!("unknown resource: {}", request.uri),
                None,
            ))
        }
    }
}

// ── main ────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let server = TestServer::new();
    let (rx, tx) = (tokio::io::stdin(), tokio::io::stdout());
    let service: RunningService<RoleServer, TestServer> = server.serve((rx, tx)).await?;
    service.waiting().await?;
    Ok(())
}
