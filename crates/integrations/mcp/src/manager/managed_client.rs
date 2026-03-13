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

//! Async-managed MCP client with lazy startup via [`Shared`] futures.
//!
//! # Architecture
//!
//! ```text
//!   McpManager
//!       │
//!       ▼
//!   AsyncManagedClient          ◄── stored in McpManagerInner.clients
//!       │
//!       │  Shared<BoxFuture<Result<ManagedClient>>>
//!       │
//!       ▼
//!   ┌─────────────────────────────────────────────────────────────┐
//!   │  Startup pipeline (runs once, result shared to all callers) │
//!   │                                                             │
//!   │  1. validate_mcp_server_name   — reject bad names early     │
//!   │  2. make_rmcp_client           — create transport (stdio    │
//!   │                                  or streamable HTTP)        │
//!   │  3. ManagedClient::start       — MCP initialize handshake   │
//!   │                                  + tools/list               │
//!   │                                                             │
//!   │  All racing against CancellationToken (stop_server)         │
//!   └─────────────────────────────────────────────────────────────┘
//!       │
//!       ▼
//!   ManagedClient { client, server_name, tool_filter, tool_timeout, tools_cache }
//! ```
//!
//! The [`Shared`] wrapper on the startup future means:
//! - The future is **lazy** — it only starts when first polled.
//! - Multiple `.clone().await` callers get the **same** result.
//! - Once resolved, subsequent awaits return instantly.
//!
//! This lets `McpManager` store the `AsyncManagedClient` immediately
//! in its clients map, before the handshake completes, so concurrent
//! callers (e.g. tool calls arriving during startup) can simply await
//! the same future rather than racing to create duplicate connections.

use std::{
    collections::HashSet,
    ffi::OsString,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::Result;
use futures::{
    FutureExt,
    future::{BoxFuture, Shared},
};
use rara_keyring_store::KeyringStoreRef;
use rmcp::model::{ClientCapabilities, Implementation, InitializeRequestParams, ProtocolVersion, Tool};
use tokio_util::sync::CancellationToken;
use tracing::info;

use crate::{
    client::RmcpClient,
    manager::{
        erm::ElicitationRequestManager,
        log_buffer::McpLogBuffer,
        registry::{McpServerConfig, TransportType},
    },
    oauth::OAuthCredentialsStoreMode,
};

/// Default timeout for the MCP initialize handshake (seconds).
const DEFAULT_STARTUP_TIMEOUT_SECS: u64 = 30;
/// Default timeout for individual tool calls (seconds).
const DEFAULT_TOOL_TIMEOUT_SECS: u64 = 60;
/// TTL for the per-server tool cache.
const TOOLS_CACHE_TTL: Duration = Duration::from_secs(300);

// ── AsyncManagedClient ──────────────────────────────────────────────

/// A cloneable handle to a lazily-started MCP server connection.
///
/// Internally wraps a `Shared<BoxFuture<Result<ManagedClient>>>`:
/// - **Lazy**: the startup pipeline only begins on the first `.await`.
/// - **Shared**: every clone resolves to the same `ManagedClient`.
/// - **Cancellable**: calling [`cancel`](Self::cancel) signals the
///   [`CancellationToken`], which races against the startup pipeline inside
///   `tokio::select!`.
#[derive(Clone)]
pub(crate) struct AsyncManagedClient {
    client:       Shared<BoxFuture<'static, Result<ManagedClient, StartupOutcomeError>>>,
    cancel_token: CancellationToken,
}

impl AsyncManagedClient {
    /// Build a new managed client that will connect on first poll.
    ///
    /// **Intentionally not `async`** — this method only *captures* the
    /// startup pipeline as a [`Shared`] future without starting it.
    /// The actual async work (transport creation, MCP handshake,
    /// `tools/list`) begins only when someone first calls
    /// [`.client().await`](Self::client).
    ///
    /// This lets [`McpManager`] store the handle in its clients map
    /// *immediately* (no `.await` needed), so concurrent callers that
    /// arrive during startup can share the same in-flight future
    /// instead of racing to create duplicate connections.
    ///
    /// The startup pipeline captured in the future is:
    /// 1. [`validate_mcp_server_name`] — reject names with special chars.
    /// 2. [`make_rmcp_client`] — create the transport (stdio / HTTP).
    /// 3. [`ManagedClient::start`] — MCP `initialize` handshake + `tools/list`.
    ///
    /// The whole pipeline races against `cancel_token.cancelled()`.
    pub(crate) fn new<S: Into<String>>(
        server_name: S,
        config: McpServerConfig,
        store_mode: OAuthCredentialsStoreMode,
        store: KeyringStoreRef,
        elicitation_requests: ElicitationRequestManager,
        log_buffer: McpLogBuffer,
    ) -> Self {
        let server_name = server_name.into();
        let cancel_token = CancellationToken::new();
        let ct = cancel_token.clone();
        let tool_filter = ToolFilter::from_config(&config);

        let fut = async move {
            validate_mcp_server_name(&server_name)?;

            let client =
                Arc::new(make_rmcp_client(&server_name, &config, store_mode, store).await?);

            let startup_timeout = config
                .startup_timeout_secs
                .unwrap_or(DEFAULT_STARTUP_TIMEOUT_SECS);
            let tool_timeout = config
                .tool_timeout_secs
                .unwrap_or(DEFAULT_TOOL_TIMEOUT_SECS);

            // Race the actual handshake against the cancellation signal.
            // If McpManager::stop_server is called before startup completes,
            // the cancel branch fires and we return Cancelled immediately.
            let result = tokio::select! {
                result = ManagedClient::start(
                    server_name.clone(),
                    client,
                    Some(Duration::from_secs(startup_timeout)),
                    Duration::from_secs(tool_timeout),
                    tool_filter,
                    elicitation_requests,
                    log_buffer.clone(),
                ) => result,
                _ = ct.cancelled() => Err(StartupOutcomeError::Cancelled),
            };

            if let Ok(ref mc) = result {
                let tool_count = mc.tools_cache.lock().await.tools.len();
                log_buffer
                    .push(
                        &server_name,
                        "info",
                        format!("connected, {tool_count} tools available"),
                    )
                    .await;
            }

            result
        };

        Self {
            client: fut.boxed().shared(),
            cancel_token,
        }
    }

    /// Await the underlying startup future.
    ///
    /// Multiple callers can call this concurrently — the [`Shared`]
    /// wrapper ensures the startup pipeline runs exactly once.
    pub(crate) async fn client(&self) -> Result<ManagedClient, StartupOutcomeError> {
        self.client.clone().await
    }

    /// Returns `true` if the startup pipeline has completed successfully.
    ///
    /// Uses [`Shared::peek`] to inspect the future without polling it:
    /// - `None` → the future hasn't resolved yet (still connecting).
    /// - `Some(Ok(_))` → startup succeeded (connected).
    /// - `Some(Err(_))` → startup failed.
    pub(crate) fn is_ready(&self) -> bool {
        self.client.peek().is_some_and(|result| result.is_ok())
    }

    /// Returns `true` if the client started successfully AND the underlying
    /// transport is still alive. Returns `false` if still connecting, startup
    /// failed, or the transport has closed (process exited, connection
    /// dropped).
    pub(crate) async fn is_alive(&self) -> bool {
        match self.client.peek() {
            Some(Ok(mc)) => !mc.client.is_transport_closed().await,
            _ => false,
        }
    }

    /// Signal the startup pipeline to abort.
    ///
    /// If the handshake is still in progress the `tokio::select!` will
    /// resolve to [`StartupOutcomeError::Cancelled`]. If startup already
    /// finished this is a no-op.
    pub(crate) fn cancel(&self) { self.cancel_token.cancel(); }
}

// ── ManagedClient ───────────────────────────────────────────────────

/// A fully initialized MCP server connection.
///
/// Produced by [`start_server_task`] after a successful handshake.
/// Holds the underlying [`RmcpClient`], the tool catalogue, and the
/// per-server tool filter / timeout configuration.
#[derive(Clone)]
pub(crate) struct ManagedClient {
    /// The low-level rmcp SDK client (wrapped in Arc for cheap cloning).
    pub(crate) client:       Arc<RmcpClient>,
    /// Which MCP server this client is connected to.
    pub(crate) server_name:  String,
    /// Allowlist / denylist filter applied before exposing tools.
    pub(crate) tool_filter:  ToolFilter,
    /// Per-tool-call timeout for this server.
    pub(crate) tool_timeout: Option<Duration>,
    /// Cached tool catalogue with TTL-based expiration.
    tools_cache:             Arc<tokio::sync::Mutex<CachedTools>>,
}

/// Cached tool catalogue with expiration time.
struct CachedTools {
    tools:      Vec<ToolInfo>,
    expires_at: Instant,
}

// ── ToolInfo ────────────────────────────────────────────────────────

/// Metadata for a single tool returned by an MCP server.
///
/// Enriched with `connector_id` / `connector_name` extracted from the
/// tool's `meta` object (see [`RmcpClient::list_tools_with_connector_ids`]).
#[derive(Clone)]
pub(crate) struct ToolInfo {
    /// Which MCP server this tool belongs to.
    pub(crate) server_name:    String,
    /// Tool name as advertised in `tools/list`.
    pub(crate) tool_name:      String,
    /// The raw MCP tool definition (schema, description, etc.).
    pub(crate) tool:           Tool,
    /// Optional connector identifier (from `meta.connector_id`).
    pub(crate) connector_id:   Option<String>,
    /// Optional human-readable connector name (from `meta.connector_name`).
    pub(crate) connector_name: Option<String>,
}

// ── ToolFilter ──────────────────────────────────────────────────────

/// Allowlist + denylist filter for MCP tools.
///
/// Evaluation rules:
/// 1. If the tool is in the **denylist** → blocked (always wins).
/// 2. If an **allowlist** is set and the tool is **not** in it → blocked.
/// 3. Otherwise → allowed.
#[derive(Default, Clone)]
pub(crate) struct ToolFilter {
    /// When `Some`, only tools in this set are allowed.
    enabled:  Option<HashSet<String>>,
    /// Tools in this set are always blocked.
    disabled: HashSet<String>,
}

impl ToolFilter {
    /// Build a filter from the registry config's
    /// `tools_enabled` / `tools_disabled` fields.
    fn from_config(config: &McpServerConfig) -> Self {
        Self {
            enabled:  config.tools_enabled.clone(),
            disabled: config.tools_disabled.clone(),
        }
    }

    /// Check whether `tool_name` passes the filter.
    pub(crate) fn allowed(&self, tool_name: &str) -> bool {
        if self.disabled.contains(tool_name) {
            return false;
        }
        match &self.enabled {
            Some(allowed) => allowed.contains(tool_name),
            None => true,
        }
    }
}

// ── StartupOutcomeError ─────────────────────────────────────────────

/// Error produced by the startup pipeline.
///
/// Must be [`Clone`] because [`Shared`] requires `Output: Clone`.
/// We can't store `anyhow::Error` (not Clone), so we stringify it.
#[derive(Debug, Clone, snafu::Snafu)]
pub(crate) enum StartupOutcomeError {
    /// The startup was explicitly cancelled via [`AsyncManagedClient::cancel`].
    #[snafu(display("MCP startup cancelled"))]
    Cancelled,
    /// Transport creation, handshake, or tool listing failed.
    #[snafu(display("MCP startup failed: {error}"))]
    Failed { error: String },
}

impl From<anyhow::Error> for StartupOutcomeError {
    fn from(err: anyhow::Error) -> Self {
        Self::Failed {
            error: err.to_string(),
        }
    }
}

// ── Private helpers ─────────────────────────────────────────────────

/// Reject server names that contain characters outside `[a-zA-Z0-9_-]`.
///
/// This prevents path-traversal or shell-injection issues when the name
/// is used in file paths, keyring entries, or log messages.
fn validate_mcp_server_name(name: &str) -> Result<(), StartupOutcomeError> {
    let valid = !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    if !valid {
        return Err(StartupOutcomeError::Failed {
            error: format!("invalid MCP server name '{name}': must match [a-zA-Z0-9_-]+"),
        });
    }
    Ok(())
}

/// Create a not-yet-initialized [`RmcpClient`] for the configured transport.
///
/// - **Stdio**: spawns a child process, communicating over stdin/stdout.
/// - **SSE (Streamable HTTP)**: opens an HTTP connection, optionally with a
///   bearer token resolved from an environment variable.
///
/// The returned client is in the `Connecting` state — call
/// [`RmcpClient::initialize`] to perform the MCP handshake.
async fn make_rmcp_client(
    server_name: &str,
    config: &McpServerConfig,
    store_mode: OAuthCredentialsStoreMode,
    store: KeyringStoreRef,
) -> Result<RmcpClient, StartupOutcomeError> {
    match config.transport {
        TransportType::Stdio => {
            let command: OsString = config.command.clone().into();
            let args: Vec<OsString> = config.args.iter().map(OsString::from).collect();
            let env = if config.env.is_empty() {
                None
            } else {
                Some(config.env.clone())
            };

            RmcpClient::new_stdio_client(command, args, env, &config.env_vars, config.cwd.clone())
                .await
                .map_err(|err| StartupOutcomeError::Failed {
                    error: err.to_string(),
                })
        }
        TransportType::Sse => {
            let url = config
                .url
                .as_deref()
                .ok_or_else(|| StartupOutcomeError::Failed {
                    error: format!("SSE transport for '{server_name}' requires a url"),
                })?;

            let bearer_token = resolve_bearer_token(config.bearer_token_env_var.as_deref())?;

            RmcpClient::new_streamable_http_client(
                server_name,
                url,
                bearer_token,
                config.http_headers.clone(),
                config.env_http_headers.clone(),
                store_mode,
                store.clone(),
            )
            .await
            .map_err(StartupOutcomeError::from)
        }
    }
}

/// Read a bearer token from the environment variable named `env_var`.
///
/// Returns:
/// - `Ok(None)` if `env_var` is `None` (no token configured).
/// - `Ok(Some(token))` if the variable is set and non-empty.
/// - `Err` if the variable is set but empty, or not set at all.
fn resolve_bearer_token(env_var: Option<&str>) -> Result<Option<String>, StartupOutcomeError> {
    let Some(var_name) = env_var else {
        return Ok(None);
    };
    match std::env::var(var_name) {
        Ok(val) if !val.is_empty() => Ok(Some(val)),
        Ok(_) => Err(StartupOutcomeError::Failed {
            error: format!("bearer token env var '{var_name}' is empty"),
        }),
        Err(_) => Err(StartupOutcomeError::Failed {
            error: format!("bearer token env var '{var_name}' is not set"),
        }),
    }
}

impl ManagedClient {
    /// Perform the MCP `initialize` handshake and fetch the tool catalogue.
    ///
    /// This is the core of the startup pipeline:
    /// 1. Build [`InitializeRequestParams`] with our client capabilities.
    /// 2. Call [`RmcpClient::initialize`] for the MCP handshake.
    /// 3. Fetch the tool catalogue via `tools/list`.
    /// 4. Return a fully initialized [`ManagedClient`].
    async fn start(
        server_name: String,
        client: Arc<RmcpClient>,
        startup_timeout: Option<Duration>,
        tool_timeout: Duration,
        tool_filter: ToolFilter,
        elicitation_requests: ElicitationRequestManager,
        log_buffer: McpLogBuffer,
    ) -> Result<Self, StartupOutcomeError> {
        // Declare our client capabilities per the MCP specification.
        // https://modelcontextprotocol.io/specification/2025-06-18/basic/lifecycle
        // Elicitation: server can ask the user for input via forms.
        // https://modelcontextprotocol.io/specification/2025-06-18/client/elicitation
        let capabilities = ClientCapabilities::builder()
            .enable_elicitation()
            .build();

        let client_info = Implementation::new("rara-mcp-client", env!("CARGO_PKG_VERSION"))
            .with_title("rara");

        let params = InitializeRequestParams::new(capabilities, client_info)
            .with_protocol_version(ProtocolVersion::LATEST);

        // Build the elicitation callback that bridges server-initiated
        // elicitation requests back to the UI via ElicitationRequestManager.
        let send_elicitation = elicitation_requests.make_sender(server_name.clone());

        // Phase 1: MCP initialize handshake.
        let _init_result = client
            .initialize(
                params,
                startup_timeout,
                send_elicitation,
                server_name.clone(),
                log_buffer,
            )
            .await
            .map_err(StartupOutcomeError::from)?;

        // Phase 2: Fetch tool catalogue (reuses the startup timeout for the
        // tools/list request as well).
        let tools = Self::fetch_tools(&server_name, &client, startup_timeout)
            .await
            .map_err(StartupOutcomeError::from)?;

        info!(
            server = %server_name,
            tools = tools.len(),
            "MCP server initialized"
        );

        Ok(Self {
            client: Arc::clone(&client),
            server_name,
            tool_filter,
            tool_timeout: Some(tool_timeout),
            tools_cache: Arc::new(tokio::sync::Mutex::new(CachedTools {
                tools,
                expires_at: Instant::now() + TOOLS_CACHE_TTL,
            })),
        })
    }

    /// Return cached tools, re-fetching from the server if the cache has
    /// expired.
    pub(crate) async fn list_tools(&self) -> Result<Vec<ToolInfo>> {
        let mut cache = self.tools_cache.lock().await;
        if Instant::now() < cache.expires_at {
            return Ok(cache.tools.clone());
        }
        let tools = Self::fetch_tools(&self.server_name, &self.client, self.tool_timeout).await?;
        cache.tools = tools.clone();
        cache.expires_at = Instant::now() + TOOLS_CACHE_TTL;
        Ok(tools)
    }

    /// Fetch all tools from the MCP server, enriched with connector metadata.
    ///
    /// Currently fetches the first page only. Most MCP servers return all
    /// tools in a single response; pagination can be added later if needed.
    async fn fetch_tools(
        server_name: &str,
        client: &Arc<RmcpClient>,
        timeout: Option<Duration>,
    ) -> Result<Vec<ToolInfo>> {
        let result = client.list_tools_with_connector_ids(None, timeout).await?;

        Ok(result
            .tools
            .into_iter()
            .map(|t| ToolInfo {
                server_name:    server_name.to_string(),
                tool_name:      t.tool.name.to_string(),
                tool:           t.tool,
                connector_id:   t.connector_id,
                connector_name: t.connector_name,
            })
            .collect())
    }
}
