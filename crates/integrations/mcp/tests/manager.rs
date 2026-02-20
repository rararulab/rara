//! Integration tests for [`McpManager`].
//!
//! Uses [`FSMcpRegistry`] with a temp file and the real `test_mcp_server`
//! example binary (stdio transport) to verify MCP protocol operations
//! end-to-end: tool listing, tool calling, resource reading.

use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{Arc, LazyLock},
};

use anyhow::Result;
use rara_mcp::{
    manager::{
        mgr::McpManager,
        registry::{FSMcpRegistry, McpServerConfig, TransportType},
    },
    oauth::OAuthCredentialsStoreMode,
};
use serde_json::json;

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

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir
        .ancestors()
        .find(|p| p.join("Cargo.lock").exists())
        .expect("could not locate workspace root");
    let bin = workspace_root.join("target/debug/examples/test_mcp_server");
    assert!(bin.exists(), "test server binary not found at {bin:?}");
    bin
});

/// Create a stdio-based McpServerConfig pointing at the test server binary.
fn test_server_config() -> McpServerConfig {
    McpServerConfig {
        command: TEST_SERVER_BIN.to_string_lossy().into_owned(),
        transport: TransportType::Stdio,
        ..Default::default()
    }
}

/// Create a new McpManager backed by a temp-file FSMcpRegistry.
async fn new_test_manager() -> Result<(McpManager, tempfile::TempDir)> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("mcp-registry.json");
    let registry = FSMcpRegistry::load(&path).await?;
    let manager = McpManager::new(Arc::new(registry), OAuthCredentialsStoreMode::default());
    Ok((manager, dir))
}

/// Start a server named "test" and return the manager.
async fn manager_with_server() -> Result<(McpManager, tempfile::TempDir)> {
    let (manager, dir) = new_test_manager().await?;
    manager.start_server("test", &test_server_config()).await?;
    Ok((manager, dir))
}

// ── Tool discovery ──────────────────────────────────────────────────────

#[tokio::test]
async fn list_tools_returns_echo_and_add() -> Result<()> {
    let (manager, _dir) = manager_with_server().await?;

    let tools = manager.list_tools("test").await?;
    let names: HashSet<_> = tools.iter().map(|t| &*t.name).collect();
    assert!(
        names.contains("echo"),
        "expected 'echo' tool, got {names:?}"
    );
    assert!(names.contains("add"), "expected 'add' tool, got {names:?}");

    manager.shutdown_all().await;
    Ok(())
}

#[tokio::test]
async fn list_tools_on_disconnected_server_fails() -> Result<()> {
    let (manager, _dir) = new_test_manager().await?;

    let result = manager.list_tools("nonexistent").await;
    assert!(result.is_err());
    assert!(
        result.unwrap_err().to_string().contains("not connected"),
        "expected 'not connected' error",
    );
    Ok(())
}

// ── Tool calling ────────────────────────────────────────────────────────

#[tokio::test]
async fn call_echo_tool() -> Result<()> {
    let (manager, _dir) = manager_with_server().await?;

    let result = manager
        .call_tool("test", "echo", Some(json!({"message": "hello world"})))
        .await?;

    let text = result
        .content
        .first()
        .and_then(|c| c.raw.as_text())
        .map(|t| &*t.text)
        .unwrap_or("");
    assert_eq!(text, "hello world");

    manager.shutdown_all().await;
    Ok(())
}

#[tokio::test]
async fn call_add_tool() -> Result<()> {
    let (manager, _dir) = manager_with_server().await?;

    let result = manager
        .call_tool("test", "add", Some(json!({"a": 17, "b": 25})))
        .await?;

    let text = result
        .content
        .first()
        .and_then(|c| c.raw.as_text())
        .map(|t| &*t.text)
        .unwrap_or("");
    assert_eq!(text, "42");

    manager.shutdown_all().await;
    Ok(())
}

#[tokio::test]
async fn call_tool_on_disconnected_server_fails() -> Result<()> {
    let (manager, _dir) = new_test_manager().await?;

    let result = manager.call_tool("ghost", "echo", None).await;
    assert!(result.is_err());
    Ok(())
}

// ── Resources ───────────────────────────────────────────────────────────

#[tokio::test]
async fn list_resources_returns_greeting() -> Result<()> {
    let (manager, _dir) = manager_with_server().await?;

    let result = manager.list_resources("test").await?;
    let uris: Vec<_> = result.resources.iter().map(|r| &*r.raw.uri).collect();
    assert!(
        uris.contains(&"test://greeting"),
        "expected 'test://greeting' resource, got {uris:?}",
    );

    manager.shutdown_all().await;
    Ok(())
}

#[tokio::test]
async fn read_resource_greeting() -> Result<()> {
    let (manager, _dir) = manager_with_server().await?;

    let result = manager
        .read_resource(
            "test",
            rmcp::model::ReadResourceRequestParams {
                uri:  "test://greeting".into(),
                meta: None,
            },
        )
        .await?;

    let text = match result.contents.first() {
        Some(rmcp::model::ResourceContents::TextResourceContents { text, .. }) => text.as_str(),
        _ => panic!("expected text resource content"),
    };
    assert_eq!(text, "Hello from test server!");

    manager.shutdown_all().await;
    Ok(())
}

// ── Lifecycle with verification ─────────────────────────────────────────

#[tokio::test]
async fn restart_server_preserves_functionality() -> Result<()> {
    let (manager, _dir) = new_test_manager().await?;
    let config = test_server_config();

    manager.add_server("test".into(), config, true).await?;

    // Call a tool before restart.
    let before = manager
        .call_tool("test", "add", Some(json!({"a": 1, "b": 2})))
        .await?;
    let text_before = before
        .content
        .first()
        .and_then(|c| c.raw.as_text())
        .map(|t| &*t.text)
        .unwrap_or("");
    assert_eq!(text_before, "3");

    // Restart.
    manager.restart_server("test").await?;

    // Call the same tool after restart — should still work.
    let after = manager
        .call_tool("test", "add", Some(json!({"a": 10, "b": 20})))
        .await?;
    let text_after = after
        .content
        .first()
        .and_then(|c| c.raw.as_text())
        .map(|t| &*t.text)
        .unwrap_or("");
    assert_eq!(text_after, "30");

    manager.shutdown_all().await;
    Ok(())
}

#[tokio::test]
async fn start_enabled_concurrent_all_functional() -> Result<()> {
    let (manager, _dir) = new_test_manager().await?;

    // Add three servers to the registry (without starting).
    for name in ["s1", "s2", "s3"] {
        manager
            .add_server(name.into(), test_server_config(), false)
            .await?;
    }

    // start_enabled should start all three concurrently.
    let mut started = manager.start_enabled().await;
    started.sort();
    assert_eq!(started, vec!["s1", "s2", "s3"]);

    // Verify each server is actually functional.
    for name in &started {
        let result = manager
            .call_tool(name, "echo", Some(json!({"message": name})))
            .await?;
        let text = result
            .content
            .first()
            .and_then(|c| c.raw.as_text())
            .map(|t| &*t.text)
            .unwrap_or("");
        assert_eq!(text, name.as_str());
    }

    manager.shutdown_all().await;
    Ok(())
}

#[tokio::test]
async fn stop_server_then_call_fails() -> Result<()> {
    let (manager, _dir) = manager_with_server().await?;

    // Verify it works.
    manager
        .call_tool("test", "echo", Some(json!({"message": "ok"})))
        .await?;

    // Stop.
    manager.stop_server("test").await;

    // Should fail now.
    let result = manager
        .call_tool("test", "echo", Some(json!({"message": "nope"})))
        .await;
    assert!(result.is_err());
    Ok(())
}

// ── Tool filtering ──────────────────────────────────────────────────────

#[tokio::test]
async fn tools_disabled_filters_tools() -> Result<()> {
    let (manager, _dir) = new_test_manager().await?;

    let mut config = test_server_config();
    config.tools_disabled = HashSet::from(["add".into()]);

    manager.start_server("test", &config).await?;

    let tools = manager.list_tools("test").await?;
    let names: Vec<_> = tools.iter().map(|t| &*t.name).collect();
    assert!(names.contains(&"echo"), "echo should be visible");
    assert!(!names.contains(&"add"), "add should be filtered out");

    manager.shutdown_all().await;
    Ok(())
}

#[tokio::test]
async fn tools_enabled_allowlist() -> Result<()> {
    let (manager, _dir) = new_test_manager().await?;

    let mut config = test_server_config();
    config.tools_enabled = Some(HashSet::from(["echo".into()]));

    manager.start_server("test", &config).await?;

    let tools = manager.list_tools("test").await?;
    let names: Vec<_> = tools.iter().map(|t| &*t.name).collect();
    assert_eq!(names, vec!["echo"], "only echo should be visible");

    manager.shutdown_all().await;
    Ok(())
}

// ── Error cases ─────────────────────────────────────────────────────────

#[tokio::test]
async fn start_server_bad_command_fails() -> Result<()> {
    let (manager, _dir) = new_test_manager().await?;
    let config = McpServerConfig {
        command: "/nonexistent/binary".into(),
        ..Default::default()
    };

    let result = manager.start_server("bad-cmd", &config).await;
    assert!(result.is_err());
    Ok(())
}

#[tokio::test]
async fn start_server_invalid_name_fails() -> Result<()> {
    let (manager, _dir) = new_test_manager().await?;
    let config = test_server_config();

    let result = manager.start_server("bad name!", &config).await;
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("invalid MCP server name"),
        "expected invalid name error",
    );
    Ok(())
}
