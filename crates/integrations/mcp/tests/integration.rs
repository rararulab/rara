//! Integration tests for [`RmcpClient`].
//!
//! These tests spawn real MCP server processes and connect through the
//! production `new_stdio_client` / `new_streamable_http_client` code paths.

use std::{ffi::OsString, path::PathBuf, sync::LazyLock, time::Duration};

use anyhow::Result;
use rara_mcp::{
    client::RmcpClient, manager::log_buffer::McpLogBuffer, oauth::OAuthCredentialsStoreMode,
};
use rmcp::model::{ClientInfo, ReadResourceRequestParams};

mod test_server;
use test_server::TestServer;

// ── Test infrastructure ─────────────────────────────────────────────────

/// Path to the compiled `test_mcp_server` example binary.
///
/// Built once per test process via `LazyLock`.
static TEST_SERVER_BIN: LazyLock<PathBuf> = LazyLock::new(|| {
    let status = std::process::Command::new("cargo")
        .args(["build", "--example", "test_mcp_server", "-p", "rara-mcp"])
        .status()
        .expect("failed to run `cargo build --example test_mcp_server`");
    assert!(status.success(), "test server example failed to compile");

    // Walk up from the crate manifest dir to the workspace root's target dir.
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir
        .ancestors()
        .find(|p| p.join("Cargo.lock").exists())
        .expect("could not locate workspace root");
    let bin = workspace_root.join("target/debug/examples/test_mcp_server");
    assert!(
        bin.exists(),
        "test server binary not found at {}",
        bin.display()
    );
    bin
});

/// Timeout applied to every MCP request in tests.
const TIMEOUT: Option<Duration> = Some(Duration::from_secs(10));

/// Create a `SendElicitation` that always rejects — tests don't use it.
fn noop_elicitation() -> rara_mcp::logging_client_handler::SendElicitation {
    Box::new(|_id, _params| {
        Box::pin(async { Err(anyhow::anyhow!("elicitation not supported in tests")) })
    })
}

/// Spin up a new stdio-based MCP client connected to the test server.
async fn new_test_client() -> Result<RmcpClient> {
    let log_buffer = McpLogBuffer::default();
    let client = RmcpClient::new_stdio_client(
        OsString::from(TEST_SERVER_BIN.as_os_str()),
        vec![],
        None,
        &[],
        None,
    )
    .await?;

    let _init = client
        .initialize(
            ClientInfo::default(),
            TIMEOUT,
            noop_elicitation(),
            "test-stdio".to_string(),
            log_buffer,
        )
        .await?;

    Ok(client)
}

// ── Stdio integration tests ─────────────────────────────────────────────

#[tokio::test]
async fn test_initialize() -> Result<()> {
    let log_buffer = McpLogBuffer::default();
    let client = RmcpClient::new_stdio_client(
        OsString::from(TEST_SERVER_BIN.as_os_str()),
        vec![],
        None,
        &[],
        None,
    )
    .await?;

    let init = client
        .initialize(
            ClientInfo::default(),
            TIMEOUT,
            noop_elicitation(),
            "test-init".to_string(),
            log_buffer,
        )
        .await?;

    assert!(
        init.instructions
            .as_deref()
            .is_some_and(|s| s.contains("Test MCP server")),
        "expected test server instructions, got: {:?}",
        init.instructions,
    );
    Ok(())
}

#[tokio::test]
async fn test_list_tools() -> Result<()> {
    let client = new_test_client().await?;
    let result = client.list_tools(None, TIMEOUT).await?;

    let names: Vec<String> = result.tools.iter().map(|t| t.name.to_string()).collect();
    assert!(
        names.iter().any(|n| n == "echo"),
        "expected `echo` tool, got {names:?}"
    );
    assert!(
        names.iter().any(|n| n == "add"),
        "expected `add` tool, got {names:?}"
    );
    Ok(())
}

#[tokio::test]
async fn test_call_tool_echo() -> Result<()> {
    let client = new_test_client().await?;
    let result = client
        .call_tool(
            "echo".to_string(),
            Some(serde_json::json!({"message": "hello world"})),
            TIMEOUT,
        )
        .await?;

    let text = result
        .content
        .first()
        .and_then(|c| c.raw.as_text())
        .map(|t| t.text.as_ref());
    assert_eq!(text, Some("hello world"));
    Ok(())
}

#[tokio::test]
async fn test_call_tool_add() -> Result<()> {
    let client = new_test_client().await?;
    let result = client
        .call_tool(
            "add".to_string(),
            Some(serde_json::json!({"a": 17, "b": 25})),
            TIMEOUT,
        )
        .await?;

    let text = result
        .content
        .first()
        .and_then(|c| c.raw.as_text())
        .map(|t| t.text.as_ref());
    assert_eq!(text, Some("42"));
    Ok(())
}

#[tokio::test]
async fn test_call_tool_invalid_args() -> Result<()> {
    let client = new_test_client().await?;

    // Pass an array instead of an object — should be rejected by our client.
    let err = client
        .call_tool(
            "echo".to_string(),
            Some(serde_json::json!(["not", "an", "object"])),
            TIMEOUT,
        )
        .await;

    assert!(err.is_err(), "expected error for non-object arguments");
    let msg = err.unwrap_err().to_string();
    assert!(
        msg.contains("JSON object"),
        "expected 'JSON object' in error, got: {msg}",
    );
    Ok(())
}

#[tokio::test]
async fn test_list_resources() -> Result<()> {
    let client = new_test_client().await?;
    let result = client.list_resources(None, TIMEOUT).await?;

    assert_eq!(result.resources.len(), 1);
    assert_eq!(result.resources[0].raw.uri, "test://greeting");
    assert_eq!(result.resources[0].raw.name, "greeting");
    Ok(())
}

#[tokio::test]
async fn test_read_resource() -> Result<()> {
    let client = new_test_client().await?;
    let result = client
        .read_resource(
            ReadResourceRequestParams {
                meta: None,
                uri:  "test://greeting".to_string(),
            },
            TIMEOUT,
        )
        .await?;

    assert_eq!(result.contents.len(), 1);
    match &result.contents[0] {
        rmcp::model::ResourceContents::TextResourceContents { text, .. } => {
            assert_eq!(text, "Hello from test server!");
        }
        other => panic!("expected TextResourceContents, got {other:?}"),
    }
    Ok(())
}

// ── HTTP integration tests ──────────────────────────────────────────────

/// Start an in-process axum server running the `TestServer` MCP handler.
///
/// Returns `(url, CancellationToken)`. Drop the token to stop the server.
async fn start_http_server() -> Result<(String, tokio_util::sync::CancellationToken)> {
    use std::sync::Arc;

    use rmcp::transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
    };

    let ct = tokio_util::sync::CancellationToken::new();

    let service: StreamableHttpService<TestServer, LocalSessionManager> =
        StreamableHttpService::new(
            || Ok(TestServer::new()),
            Arc::new(LocalSessionManager::default()),
            StreamableHttpServerConfig {
                stateful_mode: false,
                sse_keep_alive: None,
                cancellation_token: ct.child_token(),
                ..Default::default()
            },
        );

    let router = axum::Router::new().nest_service("/mcp", service);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;

    tokio::spawn({
        let ct = ct.clone();
        async move {
            let _ = axum::serve(listener, router)
                .with_graceful_shutdown(async move { ct.cancelled_owned().await })
                .await;
        }
    });

    Ok((format!("http://{addr}/mcp"), ct))
}

/// Create an HTTP-based MCP client connected to the in-process test server.
async fn new_http_test_client() -> Result<(RmcpClient, tokio_util::sync::CancellationToken)> {
    let (url, ct) = start_http_server().await?;
    let log_buffer = McpLogBuffer::default();

    let client = RmcpClient::new_streamable_http_client(
        "test-server",
        &url,
        None,
        None,
        None,
        OAuthCredentialsStoreMode::default(),
    )
    .await?;

    let _init = client
        .initialize(
            ClientInfo::default(),
            TIMEOUT,
            noop_elicitation(),
            "test-server".to_string(),
            log_buffer,
        )
        .await?;

    Ok((client, ct))
}

#[tokio::test]
async fn test_http_initialize() -> Result<()> {
    let (url, ct) = start_http_server().await?;
    let log_buffer = McpLogBuffer::default();

    let client = RmcpClient::new_streamable_http_client(
        "test-server",
        &url,
        None,
        None,
        None,
        OAuthCredentialsStoreMode::default(),
    )
    .await?;

    let init = client
        .initialize(
            ClientInfo::default(),
            TIMEOUT,
            noop_elicitation(),
            "test-server".to_string(),
            log_buffer,
        )
        .await?;

    assert!(
        init.instructions
            .as_deref()
            .is_some_and(|s| s.contains("Test MCP server")),
    );
    ct.cancel();
    Ok(())
}

#[tokio::test]
async fn test_http_list_tools() -> Result<()> {
    let (client, ct) = new_http_test_client().await?;
    let result = client.list_tools(None, TIMEOUT).await?;

    let names: Vec<String> = result.tools.iter().map(|t| t.name.to_string()).collect();
    assert!(names.iter().any(|n| n == "echo"));
    assert!(names.iter().any(|n| n == "add"));
    ct.cancel();
    Ok(())
}

#[tokio::test]
async fn test_http_call_tool() -> Result<()> {
    let (client, ct) = new_http_test_client().await?;
    let result = client
        .call_tool(
            "add".to_string(),
            Some(serde_json::json!({"a": 100, "b": 200})),
            TIMEOUT,
        )
        .await?;

    let text = result
        .content
        .first()
        .and_then(|c| c.raw.as_text())
        .map(|t| t.text.as_ref());
    assert_eq!(text, Some("300"));
    ct.cancel();
    Ok(())
}

#[tokio::test]
async fn test_http_list_resources() -> Result<()> {
    let (client, ct) = new_http_test_client().await?;
    let result = client.list_resources(None, TIMEOUT).await?;

    assert_eq!(result.resources.len(), 1);
    assert_eq!(result.resources[0].raw.uri, "test://greeting");
    ct.cancel();
    Ok(())
}

#[tokio::test]
async fn test_http_read_resource() -> Result<()> {
    let (client, ct) = new_http_test_client().await?;
    let result = client
        .read_resource(
            ReadResourceRequestParams {
                meta: None,
                uri:  "test://greeting".to_string(),
            },
            TIMEOUT,
        )
        .await?;

    match &result.contents[0] {
        rmcp::model::ResourceContents::TextResourceContents { text, .. } => {
            assert_eq!(text, "Hello from test server!");
        }
        other => panic!("expected TextResourceContents, got {other:?}"),
    }
    ct.cancel();
    Ok(())
}
