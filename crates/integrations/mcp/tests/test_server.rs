//! Shared `TestServer` implementation used by both stdio and HTTP integration
//! tests.
//!
//! This is the same server as `examples/test_mcp_server.rs` but factored into
//! a module so integration tests can construct it in-process for HTTP testing.

#![allow(dead_code)]

use rmcp::{
    RoleServer, ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{
        Annotated, ListResourcesResult, PaginatedRequestParams, RawResource,
        ReadResourceRequestParams, ReadResourceResult, ResourceContents, ServerCapabilities,
        ServerInfo,
    },
    schemars,
    service::RequestContext,
    tool, tool_handler, tool_router,
};

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct EchoRequest {
    pub message: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AddRequest {
    pub a: i32,
    pub b: i32,
}

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
    #[tool(name = "echo", description = "Echo a message back")]
    fn echo(&self, Parameters(req): Parameters<EchoRequest>) -> String {
        req.message
    }

    #[tool(name = "add", description = "Add two numbers")]
    fn add(&self, Parameters(req): Parameters<AddRequest>) -> String {
        (req.a + req.b).to_string()
    }
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
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, rmcp::ErrorData> {
        Ok(ListResourcesResult {
            resources: vec![Annotated {
                raw: RawResource::new("test://greeting", "greeting"),
                annotations: None,
            }],
            next_cursor: None,
            meta: None,
        })
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResult, rmcp::ErrorData> {
        if request.uri == "test://greeting" {
            Ok(ReadResourceResult {
                contents: vec![ResourceContents::text("Hello from test server!", request.uri)],
            })
        } else {
            Err(rmcp::ErrorData::resource_not_found(
                format!("unknown resource: {}", request.uri),
                None,
            ))
        }
    }
}
