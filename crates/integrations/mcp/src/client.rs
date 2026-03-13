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

use std::{
    collections::HashMap, ffi::OsString, io, path::PathBuf, process::Stdio, sync::Arc,
    time::Duration,
};

use anyhow::{Result, anyhow};
use base::process_group::ProcessGroupGuard;
use futures::FutureExt;
use oauth2::TokenResponse;
use rara_keyring_store::KeyringStoreRef;
use reqwest::header::{AUTHORIZATION, HeaderMap};
use rmcp::{
    RoleClient,
    model::{
        CallToolRequestParams, CallToolResult, ClientNotification, ClientRequest,
        CustomNotification, CustomRequest, Extensions, InitializeRequestParams, InitializeResult,
        ListResourceTemplatesResult, ListResourcesResult, ListToolsResult, PaginatedRequestParams,
        ReadResourceRequestParams, ReadResourceResult, ServerResult, Tool,
    },
    service::{self, ClientInitializeError, RunningService},
    transport::{
        StreamableHttpClientTransport,
        auth::{AuthClient, AuthError, OAuthState},
        child_process::TokioChildProcess,
        streamable_http_client::StreamableHttpClientTransportConfig,
    },
};
use serde_json::Value;
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
    sync::Mutex,
};
use tracing::{info, warn};

use crate::{
    logging_client_handler::{LoggingClientHandler, SendElicitation},
    manager::log_buffer::McpLogBuffer,
    oauth::{OAuthCredentialsStoreMode, OAuthPersistor, StoredOAuthTokens},
    utils::{
        apply_default_headers, build_default_headers, create_env_for_mcp_server, run_with_timeout,
    },
};

// ── Transport types ─────────────────────────────────────────────────────

/// A transport that has been created but not yet connected to the MCP server.
enum PendingTransport {
    /// Subprocess communicating over stdin/stdout.
    ChildProcess {
        transport:           TokioChildProcess,
        process_group_guard: Option<ProcessGroupGuard>,
    },
    /// Streamable HTTP without OAuth.
    StreamableHttp {
        transport: StreamableHttpClientTransport<reqwest::Client>,
    },
    /// Streamable HTTP with OAuth token refresh.
    StreamableHttpWithOAuth {
        transport:       StreamableHttpClientTransport<AuthClient<reqwest::Client>>,
        oauth_persistor: OAuthPersistor,
    },
}

/// Internal state machine for a client's lifecycle.
enum ClientState {
    /// Transport created, waiting for the MCP handshake to complete.
    Connecting { transport: Option<PendingTransport> },
    /// Handshake done — the service is ready to accept requests.
    Ready {
        _process_group_guard: Option<ProcessGroupGuard>,
        service:              Arc<RunningService<RoleClient, LoggingClientHandler>>,
        oauth:                Option<OAuthPersistor>,
    },
}

// ── RmcpClient ──────────────────────────────────────────────────────────

/// MCP client built on top of the official [`rmcp`] SDK.
///
/// Supports two transport modes:
/// - **stdio** — launches a child process and communicates over stdin/stdout.
/// - **streamable HTTP** — connects to an HTTP endpoint, optionally with OAuth.
///
/// See <https://github.com/modelcontextprotocol/rust-sdk>.
pub struct RmcpClient {
    state: Mutex<ClientState>,
}

impl RmcpClient {
    // ── constructors ─────────────────────────────────────────────────────

    /// Create a client that communicates with a child process over stdio.
    ///
    /// The child is spawned in its own process group (on Unix) so that we
    /// can cleanly terminate the entire group via `ProcessGroupGuard`.
    ///
    /// # Arguments
    ///
    /// * `program`  — Executable path or name to spawn (e.g. `"npx"`,
    ///   `"/usr/bin/python3"`).
    /// * `args`     — Command-line arguments forwarded to the child process.
    /// * `env`      — Extra environment variables merged on top of the
    ///   defaults. Overrides any variable with the same key from `env_vars`.
    /// * `env_vars` — Names of host environment variables to forward to the
    ///   child (in addition to a platform-specific default set like `PATH`).
    /// * `cwd`      — Working directory for the child process. When `None`,
    ///   inherits the current process's working directory.
    #[tracing::instrument(skip(env))]
    pub async fn new_stdio_client(
        program: OsString,
        args: Vec<OsString>,
        env: Option<HashMap<String, String>>,
        env_vars: &[String],
        cwd: Option<PathBuf>,
    ) -> io::Result<Self> {
        let program_name = program.to_string_lossy().into_owned();
        let envs = create_env_for_mcp_server(env, env_vars);

        let mut command = Command::new(program);
        command
            .kill_on_drop(true)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .env_clear()
            .envs(envs)
            .args(&args);
        #[cfg(unix)]
        command.process_group(0);
        if let Some(cwd) = cwd {
            command.current_dir(cwd);
        }

        let (transport, stderr) = TokioChildProcess::builder(command)
            .stderr(Stdio::piped())
            .spawn()?;
        let process_group_guard = transport.id().map(ProcessGroupGuard::new);

        // Drain stderr on a background task so the child doesn't block.
        if let Some(stderr) = stderr {
            Self::drain_stderr(stderr, program_name);
        }

        Ok(Self::connecting(PendingTransport::ChildProcess {
            transport,
            process_group_guard,
        }))
    }

    /// Create a client that communicates over streamable HTTP.
    ///
    /// If stored OAuth tokens are available (and no explicit bearer token or
    /// Authorization header is provided), we attempt to set up an OAuth-aware
    /// transport. When the server doesn't support OAuth metadata discovery we
    /// fall back to plain bearer-token authentication.
    ///
    /// # Arguments
    ///
    /// * `server_name`      — Human-readable identifier for the MCP server
    ///   (used as part of the keyring/file store key).
    /// * `url`              — The HTTP(S) endpoint of the MCP server.
    /// * `bearer_token`     — Optional static bearer token placed in the
    ///   `Authorization` header. When set, OAuth is skipped.
    /// * `http_headers`     — Additional static HTTP headers to include in
    ///   every request (e.g. custom `X-Api-Key`).
    /// * `env_http_headers` — Map of `header-name → env-var-name`. The value of
    ///   each environment variable is read at construction time and sent as the
    ///   corresponding header.
    /// * `store_mode`       — Where to look for (and persist) OAuth
    ///   credentials: keyring, file, or auto.
    #[allow(clippy::too_many_arguments)]
    #[tracing::instrument(skip(bearer_token, http_headers, env_http_headers, store))]
    pub async fn new_streamable_http_client(
        server_name: &str,
        url: &str,
        bearer_token: Option<String>,
        http_headers: Option<HashMap<String, String>>,
        env_http_headers: Option<HashMap<String, String>>,
        store_mode: OAuthCredentialsStoreMode,
        store: KeyringStoreRef,
    ) -> Result<Self> {
        let default_headers = build_default_headers(http_headers, env_http_headers)?;

        // Try to load previously-stored OAuth tokens, unless the caller
        // already provided explicit credentials.
        let stored_tokens =
            if bearer_token.is_none() && !default_headers.contains_key(AUTHORIZATION) {
                StoredOAuthTokens::load(server_name, url, store_mode, &*store)
                    .await
                    .unwrap_or_else(|err| {
                        warn!("failed to read tokens for server `{server_name}`: {err}");
                        None
                    })
            } else {
                None
            };

        let transport = match stored_tokens {
            Some(tokens) => {
                Self::build_oauth_transport(
                    server_name,
                    url,
                    tokens,
                    store_mode,
                    store.clone(),
                    &default_headers,
                )
                .await?
            }
            None => Self::build_plain_transport(url, bearer_token.as_deref(), &default_headers)?,
        };

        Ok(Self::connecting(transport))
    }

    // ── lifecycle ────────────────────────────────────────────────────────

    /// Perform the MCP initialization handshake with the remote server.
    ///
    /// Transitions the client from `Connecting` → `Ready`. This method can
    /// only be called **once** — subsequent calls return an error.
    ///
    /// The handshake follows the [MCP lifecycle specification][spec]:
    /// 1. Takes the pending transport out of the `Connecting` state.
    /// 2. Starts the `rmcp` service and waits for the server's `initialize`
    ///    response.
    /// 3. Stores the running service in the `Ready` state for future requests.
    /// 4. Persists any refreshed OAuth tokens (if applicable).
    ///
    /// [spec]: https://modelcontextprotocol.io/specification/2025-06-18/basic/lifecycle#initialization
    ///
    /// # Arguments
    ///
    /// * `params`           — Client capabilities and metadata sent to the
    ///   server during the `initialize` request.
    /// * `timeout`          — Maximum duration to wait for the handshake to
    ///   complete. When `None`, waits indefinitely.
    /// * `send_elicitation` — Callback used by the client handler to forward
    ///   server-initiated elicitation requests back to the caller.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The client has already been initialized (or is currently
    ///   initializing).
    /// - The handshake times out or the underlying transport fails.
    /// - The server does not provide its info after a successful handshake.
    #[tracing::instrument(skip_all)]
    pub async fn initialize(
        &self,
        params: InitializeRequestParams,
        timeout: Option<Duration>,
        send_elicitation: SendElicitation,
        server_name: String,
        log_buffer: McpLogBuffer,
    ) -> Result<InitializeResult> {
        let handler =
            LoggingClientHandler::new(params.clone(), send_elicitation, server_name, log_buffer);

        // Phase 1: Take the pending transport out of the `Connecting` state.
        // The lock is released immediately so that the (potentially long-running)
        // handshake does not hold it.
        let (handshake_fut, oauth_persistor, process_group_guard) = {
            let mut guard = self.state.lock().await;
            match &mut *guard {
                ClientState::Connecting { transport } => {
                    Self::start_handshake(transport.take(), handler)?
                }
                ClientState::Ready { .. } => {
                    return Err(anyhow!("client already initialized"));
                }
            }
        };

        // Phase 2: Await the handshake (with optional timeout).
        let service: RunningService<RoleClient, LoggingClientHandler> =
            run_with_timeout(handshake_fut, timeout, "MCP handshake").await?;

        // Phase 3: Extract server info from the completed handshake.
        let initialize_result = service
            .peer()
            .peer_info()
            .cloned()
            .ok_or_else(|| anyhow!("handshake succeeded but server info was missing"))?;

        // Phase 4: Transition to the `Ready` state.
        {
            let mut guard = self.state.lock().await;
            *guard = ClientState::Ready {
                _process_group_guard: process_group_guard,
                service:              Arc::new(service),
                oauth:                oauth_persistor.clone(),
            };
        }

        // Phase 5: Persist any refreshed OAuth tokens. A failure here is not
        // fatal — the session is already established.
        if let Some(persistor) = oauth_persistor
            && let Err(error) = persistor.persist_if_needed().await
        {
            warn!("failed to persist OAuth tokens after initialize: {error}");
        }

        Ok(initialize_result)
    }

    // ── MCP requests ────────────────────────────────────────────────────

    /// List all tools exposed by the MCP server.
    ///
    /// Automatically refreshes OAuth tokens before the request and persists
    /// any rotated tokens afterwards.
    ///
    /// # Arguments
    ///
    /// * `params`  — Optional pagination cursor. Pass `None` to fetch the first
    ///   page.
    /// * `timeout` — Maximum wait duration for the server response. When
    ///   `None`, waits indefinitely.
    #[tracing::instrument(skip_all)]
    pub async fn list_tools(
        &self,
        params: Option<PaginatedRequestParams>,
        timeout: Option<Duration>,
    ) -> Result<ListToolsResult> {
        self.refresh_oauth_if_needed().await;
        let service = self.service().await?;
        let result = run_with_timeout(service.list_tools(params), timeout, "tools/list").await?;
        self.persist_oauth_tokens().await;
        Ok(result)
    }

    /// List all tools with their connector metadata (id and display name).
    ///
    /// This is a higher-level wrapper around [`list_tools`](Self::list_tools)
    /// that extracts `connector_id` and `connector_name` from each tool's
    /// `meta` object.
    ///
    /// # Arguments
    ///
    /// * `params`  — Optional pagination cursor. Pass `None` to fetch the first
    ///   page.
    /// * `timeout` — Maximum wait duration for the server response. When
    ///   `None`, waits indefinitely.
    #[tracing::instrument(skip_all)]
    pub async fn list_tools_with_connector_ids(
        &self,
        params: Option<PaginatedRequestParams>,
        timeout: Option<Duration>,
    ) -> Result<ListToolsWithConnectorIdResult> {
        self.refresh_oauth_if_needed().await;
        let service = self.service().await?;
        let result = run_with_timeout(service.list_tools(params), timeout, "tools/list").await?;

        let tools = result
            .tools
            .into_iter()
            .map(|tool| {
                let meta = tool.meta.as_ref();
                let connector_id = Self::meta_string(meta, "connector_id");
                let connector_name = Self::meta_string(meta, "connector_name")
                    .or_else(|| Self::meta_string(meta, "connector_display_name"));
                ToolWithConnectorId {
                    tool,
                    connector_id,
                    connector_name,
                }
            })
            .collect();

        self.persist_oauth_tokens().await;
        Ok(ListToolsWithConnectorIdResult {
            next_cursor: result.next_cursor,
            tools,
        })
    }

    /// List all resources exposed by the MCP server.
    ///
    /// # Arguments
    ///
    /// * `params`  — Optional pagination cursor.
    /// * `timeout` — Maximum wait duration for the server response.
    #[tracing::instrument(skip_all)]
    pub async fn list_resources(
        &self,
        params: Option<PaginatedRequestParams>,
        timeout: Option<Duration>,
    ) -> Result<ListResourcesResult> {
        self.refresh_oauth_if_needed().await;
        let service = self.service().await?;
        let result =
            run_with_timeout(service.list_resources(params), timeout, "resources/list").await?;
        self.persist_oauth_tokens().await;
        Ok(result)
    }

    /// List all resource templates exposed by the MCP server.
    ///
    /// # Arguments
    ///
    /// * `params`  — Optional pagination cursor.
    /// * `timeout` — Maximum wait duration for the server response.
    #[tracing::instrument(skip_all)]
    pub async fn list_resource_templates(
        &self,
        params: Option<PaginatedRequestParams>,
        timeout: Option<Duration>,
    ) -> Result<ListResourceTemplatesResult> {
        self.refresh_oauth_if_needed().await;
        let service = self.service().await?;
        let result = run_with_timeout(
            service.list_resource_templates(params),
            timeout,
            "resources/templates/list",
        )
        .await?;
        self.persist_oauth_tokens().await;
        Ok(result)
    }

    /// Read a specific resource from the MCP server.
    ///
    /// # Arguments
    ///
    /// * `params`  — Resource URI and any extra parameters required by the
    ///   server.
    /// * `timeout` — Maximum wait duration for the server response.
    #[tracing::instrument(skip_all)]
    pub async fn read_resource(
        &self,
        params: ReadResourceRequestParams,
        timeout: Option<Duration>,
    ) -> Result<ReadResourceResult> {
        self.refresh_oauth_if_needed().await;
        let service = self.service().await?;
        let result =
            run_with_timeout(service.read_resource(params), timeout, "resources/read").await?;
        self.persist_oauth_tokens().await;
        Ok(result)
    }

    /// Invoke a tool on the MCP server.
    ///
    /// # Arguments
    ///
    /// * `name`      — The tool name as advertised in `tools/list`.
    /// * `arguments` — Optional JSON object of tool arguments. Returns an error
    ///   if the value is not a JSON object (e.g. an array or scalar).
    /// * `timeout`   — Maximum wait duration for the server response.
    #[tracing::instrument(skip_all, fields(tool = %name))]
    pub async fn call_tool(
        &self,
        name: String,
        arguments: Option<Value>,
        timeout: Option<Duration>,
    ) -> Result<CallToolResult> {
        self.refresh_oauth_if_needed().await;
        let service = self.service().await?;

        let arguments = match arguments {
            Some(Value::Object(map)) => Some(map),
            Some(other) => {
                return Err(anyhow!(
                    "MCP tool arguments must be a JSON object, got {other}"
                ));
            }
            None => None,
        };

        let mut params = CallToolRequestParams::new(name);
        if let Some(args) = arguments {
            params = params.with_arguments(args);
        }

        let result =
            run_with_timeout(service.call_tool(params), timeout, "tools/call").await?;

        self.persist_oauth_tokens().await;
        Ok(result)
    }

    /// Send a custom notification to the MCP server.
    ///
    /// Notifications are fire-and-forget — the server does not send a
    /// response.
    ///
    /// # Arguments
    ///
    /// * `method` — The notification method name (e.g.
    ///   `"notifications/custom"`)
    /// * `params` — Optional JSON payload attached to the notification.
    #[tracing::instrument(skip_all, fields(%method))]
    pub async fn send_custom_notification(
        &self,
        method: &str,
        params: Option<Value>,
    ) -> Result<()> {
        let service = self.service().await?;
        service
            .send_notification(ClientNotification::CustomNotification(CustomNotification {
                method: method.to_string(),
                params,
                extensions: Extensions::new(),
            }))
            .await?;
        Ok(())
    }

    /// Send a custom request to the MCP server and await the response.
    ///
    /// # Arguments
    ///
    /// * `method` — The request method name.
    /// * `params` — Optional JSON payload attached to the request.
    #[tracing::instrument(skip_all, fields(%method))]
    pub async fn send_custom_request(
        &self,
        method: &str,
        params: Option<Value>,
    ) -> Result<ServerResult> {
        let service = self.service().await?;
        let response = service
            .send_request(ClientRequest::CustomRequest(CustomRequest::new(
                method, params,
            )))
            .await?;
        Ok(response)
    }
}

/// Result of [`RmcpClient::list_tools_with_connector_ids`], pairing each tool
/// with its connector metadata.
pub struct ListToolsWithConnectorIdResult {
    /// Opaque cursor for fetching the next page, or `None` if this is the last.
    pub next_cursor: Option<String>,
    /// Tools enriched with connector metadata.
    pub tools:       Vec<ToolWithConnectorId>,
}

/// A single MCP tool together with optional connector metadata extracted from
/// its `meta` object.
pub struct ToolWithConnectorId {
    /// The raw MCP tool definition.
    pub tool:           Tool,
    /// Value of `meta.connector_id`, if present.
    pub connector_id:   Option<String>,
    /// Value of `meta.connector_name` (or `meta.connector_display_name`).
    pub connector_name: Option<String>,
}

// ── RmcpClient: construction helpers ─────────────────────────────────

/// Private helpers used during client construction and the initialization
/// handshake.
impl RmcpClient {
    /// Wrap a `PendingTransport` into the initial `Connecting` state.
    fn connecting(transport: PendingTransport) -> Self {
        Self {
            state: Mutex::new(ClientState::Connecting {
                transport: Some(transport),
            }),
        }
    }

    /// Spawn a task that reads lines from the child's stderr and logs them.
    ///
    /// # Arguments
    ///
    /// * `stderr`       — The stderr handle taken from the spawned child
    ///   process.
    /// * `program_name` — Executable name included in each log line for
    ///   identification when multiple MCP servers run in parallel.
    fn drain_stderr(stderr: tokio::process::ChildStderr, program_name: String) {
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            loop {
                match lines.next_line().await {
                    Ok(Some(line)) => {
                        info!("MCP server stderr ({program_name}): {line}");
                    }
                    Ok(None) => break,
                    Err(error) => {
                        warn!("failed to read MCP server stderr ({program_name}): {error}");
                        break;
                    }
                }
            }
        });
    }

    /// Build an OAuth-aware transport. Falls back to a plain bearer-token
    /// transport if the server doesn't support OAuth metadata discovery.
    ///
    /// # Arguments
    ///
    /// * `server_name`     — Human-readable server identifier (forwarded to the
    ///   persistor for store-key computation).
    /// * `url`             — MCP server HTTP endpoint.
    /// * `tokens`          — Previously stored OAuth tokens to bootstrap the
    ///   session.
    /// * `store_mode`      — Credential storage strategy (keyring / file /
    ///   auto).
    /// * `default_headers` — Extra HTTP headers applied to every request.
    async fn build_oauth_transport(
        server_name: &str,
        url: &str,
        tokens: StoredOAuthTokens,
        store_mode: OAuthCredentialsStoreMode,
        store: KeyringStoreRef,
        default_headers: &HeaderMap,
    ) -> Result<PendingTransport> {
        match Self::try_oauth_transport(
            server_name,
            url,
            tokens.clone(),
            store_mode,
            store,
            default_headers,
        )
        .await
        {
            Ok((transport, persistor)) => Ok(PendingTransport::StreamableHttpWithOAuth {
                transport,
                oauth_persistor: persistor,
            }),
            // The server doesn't advertise OAuth metadata — use the stored
            // access token as a plain bearer token instead.
            Err(err) if is_no_auth_support(&err) => {
                warn!(
                    "OAuth metadata discovery unavailable for MCP server `{server_name}`; falling \
                     back to stored bearer token"
                );
                let access_token = tokens.token_response.0.access_token().secret();
                Self::build_plain_transport(url, Some(access_token), default_headers)
            }
            Err(err) => Err(err),
        }
    }

    /// Attempt to create a fully OAuth-managed transport + persistor.
    ///
    /// # Arguments
    ///
    /// * `server_name`     — Human-readable server identifier (used in
    ///   persistor).
    /// * `url`             — MCP server HTTP endpoint, also used for OAuth
    ///   metadata discovery.
    /// * `tokens`          — Stored tokens used to seed the OAuth state machine
    ///   (client ID + token response).
    /// * `store_mode`      — Where to persist refreshed tokens.
    /// * `default_headers` — Extra HTTP headers applied to every request.
    async fn try_oauth_transport(
        server_name: &str,
        url: &str,
        tokens: StoredOAuthTokens,
        store_mode: OAuthCredentialsStoreMode,
        store: KeyringStoreRef,
        default_headers: &HeaderMap,
    ) -> Result<(
        StreamableHttpClientTransport<AuthClient<reqwest::Client>>,
        OAuthPersistor,
    )> {
        let http_client =
            apply_default_headers(reqwest::Client::builder(), default_headers).build()?;
        let mut oauth_state = OAuthState::new(url, Some(http_client.clone())).await?;

        oauth_state
            .set_credentials(&tokens.client_id, tokens.token_response.0.clone())
            .await?;

        // Extract the authorization manager regardless of whether the state
        // ended up Authorized or Unauthorized.
        let manager = match oauth_state {
            OAuthState::Authorized(m) | OAuthState::Unauthorized(m) => m,
            _ => return Err(anyhow!("unexpected OAuth state during client setup")),
        };

        let auth_client = AuthClient::new(http_client, manager);
        let auth_manager = auth_client.auth_manager.clone();

        let transport = StreamableHttpClientTransport::with_client(
            auth_client,
            StreamableHttpClientTransportConfig::with_uri(url),
        );

        let persistor = OAuthPersistor::new(
            server_name,
            url,
            auth_manager,
            store_mode,
            store,
            Some(tokens),
        );

        Ok((transport, persistor))
    }

    /// Build a plain (non-OAuth) streamable HTTP transport, optionally with a
    /// bearer token in the `Authorization` header.
    ///
    /// # Arguments
    ///
    /// * `url`             — MCP server HTTP endpoint.
    /// * `bearer_token`    — Optional static bearer token. When present, it is
    ///   sent as `Authorization: Bearer <token>` on every request.
    /// * `default_headers` — Extra HTTP headers applied to every request.
    fn build_plain_transport(
        url: &str,
        bearer_token: Option<&str>,
        default_headers: &HeaderMap,
    ) -> Result<PendingTransport> {
        let mut config = StreamableHttpClientTransportConfig::with_uri(url.to_string());
        if let Some(token) = bearer_token {
            config = config.auth_header(token.to_string());
        }

        let http_client =
            apply_default_headers(reqwest::Client::builder(), default_headers).build()?;
        let transport = StreamableHttpClientTransport::with_client(http_client, config);

        Ok(PendingTransport::StreamableHttp { transport })
    }

    /// Take a [`PendingTransport`] and start the `rmcp` service handshake.
    ///
    /// Returns a type-erased future (via `.boxed()`) together with the optional
    /// OAuth persistor and process-group guard that belong to the transport.
    ///
    /// # Arguments
    ///
    /// * `transport` — The pending transport taken from the `Connecting` state.
    ///   `None` means another call already consumed it.
    /// * `handler`   — The client handler that processes server notifications
    ///   and elicitation requests during the session.
    fn start_handshake(
        transport: Option<PendingTransport>,
        handler: LoggingClientHandler,
    ) -> Result<(
        futures::future::BoxFuture<
            'static,
            Result<RunningService<RoleClient, LoggingClientHandler>, ClientInitializeError>,
        >,
        Option<OAuthPersistor>,
        Option<ProcessGroupGuard>,
    )> {
        match transport {
            Some(PendingTransport::ChildProcess {
                transport,
                process_group_guard,
            }) => Ok((
                service::serve_client(handler, transport).boxed(),
                None,
                process_group_guard,
            )),
            Some(PendingTransport::StreamableHttp { transport }) => Ok((
                service::serve_client(handler, transport).boxed(),
                None,
                None,
            )),
            Some(PendingTransport::StreamableHttpWithOAuth {
                transport,
                oauth_persistor,
            }) => Ok((
                service::serve_client(handler, transport).boxed(),
                Some(oauth_persistor),
                None,
            )),
            None => Err(anyhow!("client is already initializing")),
        }
    }
}

// ── RmcpClient: internal helpers ─────────────────────────────────────

/// Private helpers for state access and OAuth token lifecycle management.
impl RmcpClient {
    /// Persist OAuth tokens if they have been refreshed since the last save.
    /// Called after every server request to capture any token rotation that
    /// happened during the request.
    async fn persist_oauth_tokens(&self) {
        if let Some(persistor) = self.oauth_persistor().await
            && let Err(error) = persistor.persist_if_needed().await
        {
            warn!("failed to persist OAuth tokens: {error}");
        }
    }

    /// Pre-emptively refresh OAuth tokens when they are close to expiry.
    /// Called before every server request to avoid mid-request token
    /// expiration.
    async fn refresh_oauth_if_needed(&self) {
        if let Some(persistor) = self.oauth_persistor().await
            && let Err(error) = persistor.refresh_if_needed().await
        {
            warn!("failed to refresh OAuth tokens: {error}");
        }
    }

    /// Return the OAuth persistor if the client was set up with OAuth.
    /// Returns `None` for stdio transports and plain HTTP transports.
    async fn oauth_persistor(&self) -> Option<OAuthPersistor> {
        let guard = self.state.lock().await;
        match &*guard {
            ClientState::Ready {
                oauth: Some(persistor),
                ..
            } => Some(persistor.clone()),
            _ => None,
        }
    }

    /// Return the running service handle, or an error if the client has not
    /// been initialized yet.
    async fn service(&self) -> Result<Arc<RunningService<RoleClient, LoggingClientHandler>>> {
        let guard = self.state.lock().await;
        match &*guard {
            ClientState::Ready { service, .. } => Ok(Arc::clone(service)),
            ClientState::Connecting { .. } => Err(anyhow!("MCP client not initialized")),
        }
    }

    /// Check whether the underlying transport has closed.
    ///
    /// Returns `true` if the service is in `Ready` state and the transport
    /// channel is closed (process exited, HTTP connection dropped, etc.).
    /// Returns `false` if still connecting or the transport is alive.
    pub(crate) async fn is_transport_closed(&self) -> bool {
        let guard = self.state.lock().await;
        match &*guard {
            ClientState::Ready { service, .. } => service.is_closed(),
            ClientState::Connecting { .. } => false,
        }
    }

    /// Extract a non-empty string value from a tool's `meta` map.
    fn meta_string(meta: Option<&rmcp::model::Meta>, key: &str) -> Option<String> {
        meta.and_then(|meta| meta.get(key))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────

/// Check whether an error is `AuthError::NoAuthorizationSupport`.
fn is_no_auth_support(err: &anyhow::Error) -> bool {
    err.downcast_ref::<AuthError>()
        .is_some_and(|e| matches!(e, AuthError::NoAuthorizationSupport))
}
