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

//! OpenAI-compatible LLM driver.
//!
//! Uses `reqwest` directly for HTTP + SSE parsing, supporting fields
//! like `reasoning_content` that `async-openai` doesn't expose.

use std::{collections::HashMap, net::IpAddr, sync::Arc, time::Duration};

use async_trait::async_trait;
use eventsource_stream::Eventsource;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::mpsc;

use super::{
    driver::{LlmCredentialResolverRef, LlmDriver, LlmEmbedder, LlmModelLister},
    stream::StreamDelta,
    types::{
        CompletionRequest, CompletionResponse, ContentBlock, EmbeddingRequest, EmbeddingResponse,
        LlmProviderFamily, Message, MessageContent, ModelInfo, ReasoningEffort, Role, StopReason,
        ThinkingConfig, ToolCallRequest, ToolChoice, Usage, detect_provider_family,
    },
};
use crate::error::{KernelError, Result};

// ---------------------------------------------------------------------------
// OpenAiDriver
// ---------------------------------------------------------------------------

/// Cached metadata for a model from the provider's `/models` endpoint.
#[derive(Debug, Clone)]
struct ModelMeta {
    context_length:  Option<usize>,
    supports_vision: bool,
}

/// OpenAI-compatible LLM driver.
///
/// Uses `reqwest` directly for HTTP + SSE parsing, supporting fields
/// like `reasoning_content` that `async-openai` doesn't expose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiFormat {
    ChatCompletions,
    Responses,
}

impl ApiFormat {
    fn request_path(self) -> &'static str {
        match self {
            Self::ChatCompletions => "/chat/completions",
            Self::Responses => "/v1/responses",
        }
    }
}

pub struct OpenAiDriver {
    /// Client for non-streaming requests (with total timeout).
    client:                reqwest::Client,
    /// Client for streaming requests (no total timeout — SSE idle timeout
    /// handles stall detection instead).
    stream_client:         reqwest::Client,
    config_source:         OpenAiDriverConfigSource,
    /// Per-event idle timeout for SSE streaming. Defaults to
    /// [`Self::DEFAULT_SSE_IDLE_TIMEOUT`].
    sse_idle_timeout:      Duration,
    /// Lazily populated cache of model metadata from the provider's
    /// `/models` endpoint.  Initialised at most once via
    /// [`tokio::sync::OnceCell`] to avoid duplicate fetches under
    /// concurrent access.
    models_cache:          tokio::sync::OnceCell<HashMap<String, ModelMeta>>,
    api_format:            ApiFormat,
    request_path_override: Option<String>,
    base_url_override:     Option<String>,
}

enum OpenAiDriverConfigSource {
    Static {
        base_url: String,
        api_key:  String,
    },
    SettingsBacked {
        settings:      Arc<dyn rara_domain_shared::settings::SettingsProvider>,
        provider_name: String,
    },
    Dynamic {
        resolver: LlmCredentialResolverRef,
    },
}

#[derive(Debug)]
struct ResolvedConfig {
    base_url:      String,
    api_key:       String,
    extra_headers: Vec<(String, String)>,
}

/// Check whether a URL points to a local/private-network address.
///
/// Returns `true` for loopback (`127.x.x.x`, `::1`), link-local, and
/// RFC 1918 private ranges (`10.x`, `172.16-31.x`, `192.168.x`) as well as
/// `localhost`.  Used to decide whether the reqwest client should bypass
/// system proxy settings — proxies typically cannot route to these addresses.
pub fn is_local_url(url: &str) -> bool {
    let host = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
        .unwrap_or(url)
        // Remove path
        .split('/')
        .next()
        .unwrap_or("")
        // Remove port
        .rsplit_once(':')
        .map_or(url, |(host, _)| host);

    if host == "localhost" {
        return true;
    }

    host.parse::<IpAddr>().is_ok_and(|ip| match ip {
        IpAddr::V4(v4) => v4.is_loopback() || v4.is_private() || v4.is_link_local(),
        IpAddr::V6(v6) => v6.is_loopback(),
    })
}

/// Maximum number of retries for rate-limited (429) requests.
const RATE_LIMIT_MAX_RETRIES: u32 = 2;
/// Initial backoff delay for rate-limited retries.
const RATE_LIMIT_INITIAL_DELAY: Duration = Duration::from_secs(5);
/// Maximum backoff delay for rate-limited retries.
const RATE_LIMIT_MAX_DELAY: Duration = Duration::from_mins(1);

impl OpenAiDriver {
    /// Default SSE idle timeout (90 s). If no SSE event arrives within this
    /// duration the stream is aborted and a retryable error returned.
    /// Set to 90 s to accommodate reasoning models (o1, o3, deepseek-r1) that
    /// may take 60+ seconds before emitting the first token.
    pub const DEFAULT_SSE_IDLE_TIMEOUT: Duration = Duration::from_secs(90);
    /// Timeout for the metadata-only `/models` request.  Kept short so a
    /// slow or unreachable provider does not block agent loop startup.
    const MODELS_FETCH_TIMEOUT: Duration = Duration::from_secs(5);

    /// Build a reqwest client for non-streaming requests (5-minute total
    /// timeout).
    ///
    /// When `no_proxy` is true, system proxy settings are bypassed entirely.
    /// This is needed for local/private-network providers where a configured
    /// HTTP proxy would incorrectly intercept the request.
    fn build_http_client(no_proxy: bool) -> reqwest::Client {
        let mut builder = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_mins(5));
        if no_proxy {
            builder = builder.no_proxy();
        }
        builder.build().expect("failed to build HTTP client")
    }

    /// Build a reqwest client for streaming requests.
    ///
    /// No total timeout — the per-event SSE idle timeout handles stall
    /// detection. A global timeout would incorrectly kill long-running
    /// streams with extended thinking or large context.
    ///
    /// When `no_proxy` is true, system proxy settings are bypassed entirely.
    fn build_stream_client(no_proxy: bool) -> reqwest::Client {
        let mut builder = reqwest::Client::builder().connect_timeout(Duration::from_secs(10));
        if no_proxy {
            builder = builder.no_proxy();
        }
        builder
            .build()
            .expect("failed to build streaming HTTP client")
    }

    /// Create a new driver targeting the given API base URL.
    pub fn new(base_url: impl Into<String>, api_key: impl Into<String>) -> Self {
        let base_url = base_url.into();
        let no_proxy = is_local_url(&base_url);
        Self::with_idle_timeout_inner(base_url, api_key, Self::DEFAULT_SSE_IDLE_TIMEOUT, no_proxy)
    }

    /// Create a new driver with an explicit SSE idle timeout.
    pub fn with_idle_timeout(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        sse_idle_timeout: Duration,
    ) -> Self {
        let base_url = base_url.into();
        let no_proxy = is_local_url(&base_url);
        Self::with_idle_timeout_inner(base_url, api_key, sse_idle_timeout, no_proxy)
    }

    fn with_idle_timeout_inner(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        sse_idle_timeout: Duration,
        no_proxy: bool,
    ) -> Self {
        Self {
            client: Self::build_http_client(no_proxy),
            stream_client: Self::build_stream_client(no_proxy),
            config_source: OpenAiDriverConfigSource::Static {
                base_url: base_url.into(),
                api_key:  api_key.into(),
            },
            sse_idle_timeout,
            models_cache: tokio::sync::OnceCell::new(),
            api_format: ApiFormat::ChatCompletions,
            request_path_override: None,
            base_url_override: None,
        }
    }

    /// Create a driver that resolves its base URL and API key from runtime
    /// settings on every request.
    ///
    /// Looks up `llm.providers.{provider_name}.base_url` and
    /// `llm.providers.{provider_name}.api_key` from the settings provider.
    ///
    /// `no_proxy` bypasses system proxy for local/private-network providers.
    /// Use [`is_local_url`] on the provider's base URL to determine this.
    pub fn from_settings(
        settings: Arc<dyn rara_domain_shared::settings::SettingsProvider>,
        provider_name: impl Into<String>,
        sse_idle_timeout: Duration,
        no_proxy: bool,
    ) -> Self {
        Self {
            client: Self::build_http_client(no_proxy),
            stream_client: Self::build_stream_client(no_proxy),
            config_source: OpenAiDriverConfigSource::SettingsBacked {
                settings,
                provider_name: provider_name.into(),
            },
            sse_idle_timeout,
            models_cache: tokio::sync::OnceCell::new(),
            api_format: ApiFormat::ChatCompletions,
            request_path_override: None,
            base_url_override: None,
        }
    }

    /// Create a driver that resolves credentials dynamically via a
    /// [`LlmCredentialResolver`](super::driver::LlmCredentialResolver) on every
    /// request.
    ///
    /// Used for providers with expiring tokens (e.g. OAuth).
    ///
    /// **Note:** `models_cache` is shared across all requests and never
    /// invalidated. This assumes the resolver always returns the same
    /// `base_url` — if a future resolver can switch providers, the cache
    /// must be reconsidered.
    pub fn with_credential_resolver(
        resolver: LlmCredentialResolverRef,
        sse_idle_timeout: Duration,
    ) -> Self {
        // Dynamic resolvers typically point to cloud providers, so proxy is fine.
        Self {
            client: Self::build_http_client(false),
            stream_client: Self::build_stream_client(false),
            config_source: OpenAiDriverConfigSource::Dynamic { resolver },
            sse_idle_timeout,
            models_cache: tokio::sync::OnceCell::new(),
            api_format: ApiFormat::ChatCompletions,
            request_path_override: None,
            base_url_override: None,
        }
    }

    #[must_use]
    pub fn with_api_format(mut self, api_format: ApiFormat) -> Self {
        self.api_format = api_format;
        self
    }

    #[must_use]
    pub fn with_request_path_override(mut self, request_path: impl Into<String>) -> Self {
        self.request_path_override = Some(request_path.into());
        self
    }

    /// Override the base URL resolved from credentials.
    ///
    /// Useful when the credential resolver returns a generic URL (e.g.
    /// `api.openai.com`) but the driver must target a different endpoint
    /// (e.g. `chatgpt.com/backend-api`).
    #[must_use]
    pub fn with_base_url_override(mut self, base_url: impl Into<String>) -> Self {
        self.base_url_override = Some(base_url.into());
        self
    }

    async fn resolve_config(&self) -> Result<ResolvedConfig> {
        match &self.config_source {
            OpenAiDriverConfigSource::Static { base_url, api_key } => Ok(ResolvedConfig {
                base_url:      base_url.clone(),
                api_key:       api_key.clone(),
                extra_headers: Vec::new(),
            }),
            OpenAiDriverConfigSource::SettingsBacked {
                settings,
                provider_name,
            } => {
                let base_url_key = format!("llm.providers.{provider_name}.base_url");
                let api_key_key = format!("llm.providers.{provider_name}.api_key");

                let base_url =
                    settings
                        .get(&base_url_key)
                        .await
                        .ok_or_else(|| KernelError::Provider {
                            message: format!(
                                "LLM provider base URL is not configured (checked: {base_url_key})"
                            )
                            .into(),
                        })?;
                let api_key =
                    settings
                        .get(&api_key_key)
                        .await
                        .ok_or_else(|| KernelError::Provider {
                            message: format!(
                                "LLM provider API key is not configured (checked: {api_key_key})"
                            )
                            .into(),
                        })?;

                Ok(ResolvedConfig {
                    base_url,
                    api_key,
                    extra_headers: Vec::new(),
                })
            }
            OpenAiDriverConfigSource::Dynamic { resolver } => {
                let cred = resolver.resolve().await?;
                Ok(ResolvedConfig {
                    base_url:      cred.base_url().to_owned(),
                    api_key:       cred.api_key().to_owned(),
                    extra_headers: cred.extra_headers().to_vec(),
                })
            }
        }
    }

    /// Send an authenticated HTTP request to the provider and return the
    /// successful response.
    ///
    /// Handles bearer auth and error classification. Used by models/embeddings
    /// endpoints that don't need the chat-specific retry logic.
    async fn send_authenticated_request(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> Result<reqwest::Response> {
        let config = self.resolve_config().await?;
        let url = format!("{}{}", config.base_url, path);

        let mut builder = self
            .client
            .request(method, &url)
            .bearer_auth(&config.api_key);
        for (name, value) in &config.extra_headers {
            builder = builder.header(name, value);
        }
        if let Some(b) = body {
            builder = builder.json(&b);
        }

        let response = builder.send().await.map_err(|e| KernelError::Provider {
            message: format!(
                "HTTP request to {path} failed: {}",
                crate::error::format_error_chain(&e)
            )
            .into(),
        })?;

        if response.status().is_success() {
            return Ok(response);
        }

        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        Err(crate::error::classify_provider_error(
            &format!("HTTP {status}: {text}"),
            Some(status.as_u16()),
        ))
    }

    /// Send a request and return the successful HTTP response.
    ///
    /// Automatically retries on HTTP 429 (rate limited) with exponential
    /// backoff, respecting the `Retry-After` header when present.
    async fn send_request(
        &self,
        request: &CompletionRequest,
        stream: bool,
    ) -> Result<reqwest::Response> {
        let config = self.resolve_config().await?;
        let base_url = self
            .base_url_override
            .as_deref()
            .unwrap_or(&config.base_url);
        let body = build_request_body(request, stream, self.api_format)?;
        let path = self
            .request_path_override
            .as_deref()
            .unwrap_or_else(|| self.api_format.request_path());

        tracing::debug!(
            model = request.model.as_str(),
            messages = request.messages.len(),
            tools = request.tools.len(),
            stream,
            api_format = ?self.api_format,
            path,
            "sending LLM request"
        );

        let mut attempt = 0u32;
        // Use stream_client for streaming (no total timeout; SSE idle timeout
        // handles stalls) and client for non-streaming (5-min hard cap).
        let http = if stream {
            &self.stream_client
        } else {
            &self.client
        };

        loop {
            let mut req = http
                .post(format!("{}{}", base_url, path))
                .bearer_auth(&config.api_key);
            for (name, value) in &config.extra_headers {
                req = req.header(name, value);
            }
            let response = req
                .json(&body)
                .send()
                .await
                .map_err(|e| KernelError::Provider {
                    message: format!(
                        "LLM provider request failed: {}",
                        crate::error::format_error_chain(&e)
                    )
                    .into(),
                })?;

            if response.status().is_success() {
                return Ok(response);
            }

            let status = response.status();

            // Rate limited — retry with backoff
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS && attempt < RATE_LIMIT_MAX_RETRIES
            {
                let retry_after = response
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<u64>().ok())
                    .map(Duration::from_secs);

                let backoff = retry_after.unwrap_or_else(|| {
                    (RATE_LIMIT_INITIAL_DELAY * 2u32.saturating_pow(attempt))
                        .min(RATE_LIMIT_MAX_DELAY)
                });

                tracing::warn!(
                    attempt = attempt + 1,
                    max_retries = RATE_LIMIT_MAX_RETRIES,
                    backoff_secs = backoff.as_secs(),
                    model = request.model.as_str(),
                    "rate limited (429), backing off"
                );

                tokio::time::sleep(backoff).await;
                attempt += 1;
                continue;
            }

            // Non-429 error — fail immediately
            let text = response.text().await.unwrap_or_default();

            if let Ok(request_body) = serde_json::to_string(&body) {
                tracing::warn!(
                    %status,
                    response_body = %text,
                    request_body = %request_body,
                    "LLM provider returned error"
                );
            }

            return Err(crate::error::classify_provider_error(
                &format!("HTTP {status}: {text}"),
                Some(status.as_u16()),
            ));
        }
    }

    /// Fetch metadata for all models from the provider's `/models` endpoint.
    /// Returns a map of `model_id → ModelMeta`.
    ///
    /// Errors are logged and swallowed — callers fall back to the default.
    async fn fetch_model_metadata(&self) -> HashMap<String, ModelMeta> {
        let config = match self.resolve_config().await {
            Ok(c) => c,
            Err(e) => {
                tracing::debug!(error = %e, "failed to resolve config for /models fetch");
                return HashMap::new();
            }
        };
        let url = format!("{}/models", config.base_url);

        let result = tokio::time::timeout(
            Self::MODELS_FETCH_TIMEOUT,
            self.client.get(&url).bearer_auth(&config.api_key).send(),
        )
        .await;

        let response = match result {
            Ok(Ok(resp)) => resp,
            Ok(Err(e)) => {
                tracing::debug!(error = %e, "failed to fetch /models for context length cache");
                return HashMap::new();
            }
            Err(_) => {
                tracing::debug!(
                    timeout_secs = Self::MODELS_FETCH_TIMEOUT.as_secs(),
                    "timed out fetching /models for context length cache"
                );
                return HashMap::new();
            }
        };

        if !response.status().is_success() {
            tracing::debug!(
                status = %response.status(),
                "non-success response from /models endpoint"
            );
            return HashMap::new();
        }

        let raw: RawModelsResponse = match response.json().await {
            Ok(r) => r,
            Err(e) => {
                tracing::debug!(error = %e, "failed to parse /models response for context lengths");
                return HashMap::new();
            }
        };

        let cache: HashMap<String, ModelMeta> = raw
            .data
            .into_iter()
            .map(|e| {
                let supports_vision = e
                    .architecture
                    .as_ref()
                    .map(|a| a.input_modalities.iter().any(|m| m == "image"))
                    .unwrap_or(false);
                (
                    e.id,
                    ModelMeta {
                        context_length: e.context_length,
                        supports_vision,
                    },
                )
            })
            .collect();

        tracing::debug!(
            cached_models = cache.len(),
            "populated model metadata cache from /models"
        );

        cache
    }
}

// ---------------------------------------------------------------------------
// Request body construction — dispatches by ApiFormat
// ---------------------------------------------------------------------------

/// Build the JSON request body for the given API format.
/// Sanitize a tool name to comply with OpenAI's `^[a-zA-Z0-9-]+$` restriction.
///
/// Returns `Some(sanitized)` only when the name needed modification.
fn sanitize_tool_name(name: &str) -> Option<String> {
    if name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
        return None;
    }
    Some(
        name.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' {
                    c
                } else {
                    '-'
                }
            })
            .collect(),
    )
}

/// Rewrite non-compliant tool names inside a [`CompletionRequest`] and return
/// a reverse map (sanitized → original) so responses can be mapped back.
fn sanitize_request_tool_names(request: &mut CompletionRequest) -> HashMap<String, String> {
    let mut reverse_map = HashMap::new();

    for tool in &mut request.tools {
        if let Some(sanitized) = sanitize_tool_name(&tool.name) {
            let original = std::mem::replace(&mut tool.name, sanitized.clone());
            reverse_map.insert(sanitized, original);
        }
    }

    // Rewrite tool-call names in conversation history so the provider sees
    // consistent names across the entire request.
    for msg in &mut request.messages {
        for tc in &mut msg.tool_calls {
            if let Some(sanitized) = sanitize_tool_name(&tc.name) {
                tc.name = sanitized;
            }
        }
    }

    reverse_map
}

/// Look up the original tool name from a potentially-sanitized name.
fn unsanitize_tool_name(name: String, reverse_map: &HashMap<String, String>) -> String {
    reverse_map.get(&name).cloned().unwrap_or(name)
}

fn build_request_body(
    request: &CompletionRequest,
    stream: bool,
    format: ApiFormat,
) -> Result<Value> {
    match format {
        ApiFormat::ChatCompletions => {
            let chat = ChatRequest::from_completion(request, stream);
            serde_json::to_value(chat).map_err(|e| KernelError::Provider {
                message: format!("failed to serialize chat request: {e}").into(),
            })
        }
        ApiFormat::Responses => {
            let mut body = build_responses_request(request, format);
            body["stream"] = json!(stream);
            Ok(body)
        }
    }
}

/// Build a Responses API request body from our internal `CompletionRequest`.
///
/// The Responses API uses `input[]` items with explicit type tags instead of
/// the Chat Completions `messages[]` format.
pub(crate) fn build_responses_request(request: &CompletionRequest, _format: ApiFormat) -> Value {
    let mut input = Vec::new();
    let mut instructions_parts: Vec<String> = Vec::new();

    for msg in &request.messages {
        match msg.role {
            Role::System | Role::Developer => {
                let text = msg.content.as_text();
                if !text.is_empty() {
                    instructions_parts.push(text.to_string());
                }
            }
            Role::User => {
                input.push(json!({
                    "type": "message",
                    "role": "user",
                    "content": [{
                        "type": "input_text",
                        "text": msg.content.as_text(),
                    }],
                }));
            }
            Role::Assistant => {
                if msg.tool_calls.is_empty() {
                    input.push(json!({
                        "type": "message",
                        "role": "assistant",
                        "content": [{
                            "type": "output_text",
                            "text": msg.content.as_text(),
                        }],
                    }));
                } else {
                    let text = msg.content.as_text();
                    if !text.is_empty() {
                        input.push(json!({
                            "type": "message",
                            "role": "assistant",
                            "content": [{
                                "type": "output_text",
                                "text": text,
                            }],
                        }));
                    }
                    for tc in &msg.tool_calls {
                        input.push(json!({
                            "type": "function_call",
                            "name": tc.name,
                            "arguments": tc.arguments,
                            "call_id": tc.id,
                        }));
                    }
                }
            }
            Role::Tool => {
                input.push(json!({
                    "type": "function_call_output",
                    "call_id": msg.tool_call_id.as_deref().unwrap_or(""),
                    "output": msg.content.as_text(),
                }));
            }
        }
    }

    let tools: Vec<Value> = request
        .tools
        .iter()
        .map(|t| {
            json!({
                "type": "function",
                "name": t.name,
                "description": t.description,
                "parameters": t.parameters,
            })
        })
        .collect();

    let instructions = instructions_parts.join("\n\n");

    let mut body = json!({
        "model": request.model,
        "instructions": instructions,
        "input": input,
        "store": false,
    });

    if !tools.is_empty() {
        body["tools"] = json!(tools);
    }

    let reasoning_effort = resolve_reasoning_effort(&request.model, request.thinking.as_ref());

    body["reasoning"] = json!({
        "effort": reasoning_effort.as_wire(),
        "summary": "auto",
    });

    body
}

/// Resolve the [`ReasoningEffort`] for an OpenAI Responses API call.
///
/// Preference order:
///
/// 1. `ThinkingConfig { enabled: false, .. }` → [`ReasoningEffort::Off`].
///    Dominates any `effort` hint the caller happens to carry alongside, so we
///    cannot ask the API for reasoning on a disabled turn.
/// 2. Typed [`ThinkingConfig::effort`] — used verbatim, clamped to the concrete
///    model's accepted set.
/// 3. Legacy [`ThinkingConfig::budget_tokens`] fallback — the pre-`effort` code
///    path, kept so callers that still set only a budget keep working.
/// 4. No config → [`ReasoningEffort::Medium`], so reasoning-family models still
///    reason by default.
fn resolve_reasoning_effort(model: &str, thinking: Option<&ThinkingConfig>) -> ReasoningEffort {
    let effort = match thinking {
        None => return ReasoningEffort::Medium,
        Some(t) if !t.enabled => ReasoningEffort::Off,
        Some(t) => t
            .effort
            .unwrap_or_else(|| effort_from_budget(t.budget_tokens)),
    };
    effort.clamp_for_model(model)
}

/// Map a legacy Anthropic-style token budget to the closest
/// [`ReasoningEffort`] bucket. Kept so pre-`effort` callers still route to
/// sensible values.
fn effort_from_budget(budget: Option<u32>) -> ReasoningEffort {
    match budget {
        Some(b) if b >= 10_000 => ReasoningEffort::High,
        Some(b) if b >= 3_000 => ReasoningEffort::Medium,
        Some(_) => ReasoningEffort::Low,
        None => ReasoningEffort::Medium,
    }
}
// ---------------------------------------------------------------------------
// Non-streaming response parsing
// ---------------------------------------------------------------------------

/// Parse a Chat Completions API response into our internal type.
fn parse_chat_completion_response(
    raw: RawCompletionResponse,
    request: &CompletionRequest,
    reverse_map: &HashMap<String, String>,
) -> CompletionResponse {
    let choice = raw.choices.into_iter().next();
    let (stop_reason, tool_calls, content, reasoning_content) = match choice {
        Some(choice) => {
            let stop_reason = parse_stop_reason(choice.finish_reason.as_deref());
            let tool_calls = choice
                .message
                .tool_calls
                .unwrap_or_default()
                .into_iter()
                .map(|tc| ToolCallRequest {
                    id:        tc.id,
                    name:      unsanitize_tool_name(tc.function.name, reverse_map),
                    arguments: tc.function.arguments,
                })
                .collect();

            let (embedded_thinking, cleaned_content) = choice
                .message
                .content
                .as_deref()
                .map(super::think_tag::strip_think_tags)
                .map(|(thinking, content)| (thinking, non_empty(content)))
                .unwrap_or((None, None));

            let reasoning = choice.message.reasoning_content.or(embedded_thinking);
            (stop_reason, tool_calls, cleaned_content, reasoning)
        }
        None => (StopReason::Stop, vec![], None, None),
    };

    let usage = raw.usage.map(|u| Usage {
        prompt_tokens:     u.input.unwrap_or(0),
        completion_tokens: u.output.unwrap_or(0),
        total_tokens:      u.total.unwrap_or(0),
    });

    CompletionResponse {
        content,
        reasoning_content,
        tool_calls,
        stop_reason,
        usage,
        model: raw.model.unwrap_or_else(|| request.model.clone()),
    }
}

/// Parse a Responses API non-streaming response.
fn parse_responses_completion(
    raw: Value,
    request: &CompletionRequest,
    reverse_map: &HashMap<String, String>,
) -> Result<CompletionResponse> {
    let mut text = String::new();
    let mut reasoning = String::new();
    let mut tool_calls = Vec::new();
    let mut has_function_call = false;

    if let Some(output) = raw.get("output").and_then(|o| o.as_array()) {
        for item in output {
            let item_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
            match item_type {
                "message" => {
                    if let Some(content) = item.get("content").and_then(|c| c.as_array()) {
                        for part in content {
                            let part_type = part.get("type").and_then(|v| v.as_str()).unwrap_or("");
                            if part_type == "output_text" || part_type == "text" {
                                if let Some(t) = part.get("text").and_then(|v| v.as_str()) {
                                    text.push_str(t);
                                }
                            }
                        }
                    }
                }
                "reasoning" => {
                    if let Some(summary) = item.get("summary").and_then(|s| s.as_array()) {
                        for part in summary {
                            if let Some(t) = part.get("text").and_then(|v| v.as_str()) {
                                reasoning.push_str(t);
                            }
                        }
                    }
                }
                "function_call" => {
                    has_function_call = true;
                    let call_id = item
                        .get("call_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_owned();
                    let name = item
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_owned();
                    let arguments = item
                        .get("arguments")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_owned();
                    tool_calls.push(ToolCallRequest {
                        id: call_id,
                        name: unsanitize_tool_name(name, reverse_map),
                        arguments,
                    });
                }
                _ => {}
            }
        }
    }

    let usage = raw.get("usage").map(|u| {
        let input = u.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        let output = u.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        Usage {
            prompt_tokens:     input,
            completion_tokens: output,
            total_tokens:      input + output,
        }
    });

    let stop_reason = if has_function_call {
        StopReason::ToolCalls
    } else {
        let status = raw
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("completed");
        match status {
            "incomplete" => {
                let reason = raw
                    .get("incomplete_details")
                    .and_then(|d| d.get("reason"))
                    .and_then(|r| r.as_str());
                match reason {
                    Some("max_output_tokens") => StopReason::Length,
                    Some("content_filter") => StopReason::ContentFilter,
                    _ => StopReason::Stop,
                }
            }
            _ => StopReason::Stop,
        }
    };

    Ok(CompletionResponse {
        content: non_empty(text),
        reasoning_content: non_empty(reasoning),
        tool_calls,
        stop_reason,
        usage,
        model: raw
            .get("model")
            .and_then(|v| v.as_str())
            .map(|s| s.to_owned())
            .unwrap_or_else(|| request.model.clone()),
    })
}

// ---------------------------------------------------------------------------
// Streaming — Chat Completions format
// ---------------------------------------------------------------------------

/// Stream a Chat Completions API response using `StreamAccumulator`.
async fn stream_chat_completions(
    response: reqwest::Response,
    tx: mpsc::Sender<StreamDelta>,
    model: String,
    sse_idle_timeout: Duration,
    reverse_map: HashMap<String, String>,
) -> Result<CompletionResponse> {
    let mut event_stream = response.bytes_stream().eventsource();
    let mut acc = StreamAccumulator::new();
    acc.tool_name_reverse_map = reverse_map;

    loop {
        if tx.is_closed() {
            tracing::debug!("stream consumer disconnected, returning partial response");
            break;
        }

        let maybe_event = tokio::time::timeout(sse_idle_timeout, event_stream.next()).await;

        match maybe_event {
            Ok(Some(event_result)) => {
                let event = event_result.map_err(|e| KernelError::RetryableServer {
                    message: format!("SSE stream error: {}", crate::error::format_error_chain(&e))
                        .into(),
                })?;
                if event.data == "[DONE]" {
                    break;
                }
                let Ok(chunk) = serde_json::from_str::<RawStreamChunk>(&event.data) else {
                    let truncated = truncate_utf8(&event.data, 200);
                    tracing::debug!(data = truncated, "skipping unparseable SSE chunk");
                    continue;
                };
                acc.process_chunk(&chunk, &tx).await;
            }
            Ok(None) => break,
            Err(_elapsed) => {
                tracing::warn!(
                    timeout_secs = sse_idle_timeout.as_secs(),
                    "SSE stream idle timeout — no event received, aborting stream"
                );
                return Err(KernelError::RetryableServer {
                    message: "SSE stream idle timeout".into(),
                });
            }
        }
    }

    Ok(acc.finalize(&tx, model).await)
}

// ---------------------------------------------------------------------------
// Streaming — Responses API format
// ---------------------------------------------------------------------------

/// Tracks tool calls by `output_index` for the Responses API stream.
struct ResponsesToolIndexTracker {
    map:  HashMap<u64, u32>,
    next: u32,
}

impl ResponsesToolIndexTracker {
    fn new() -> Self {
        Self {
            map:  HashMap::new(),
            next: 0,
        }
    }

    fn index_for(&mut self, output_index: u64) -> u32 {
        *self.map.entry(output_index).or_insert_with(|| {
            let idx = self.next;
            self.next += 1;
            idx
        })
    }
}

/// Accumulated state while processing a Responses API SSE stream.
pub(crate) struct ResponsesStreamState {
    tool_tracker:                     ResponsesToolIndexTracker,
    pub(crate) accumulated_text:      String,
    pub(crate) accumulated_reasoning: String,
    pub(crate) has_function_call:     bool,
    pub(crate) final_stop:            StopReason,
    pub(crate) final_usage:           Option<Usage>,
    pub(crate) tool_name_reverse_map: HashMap<String, String>,
}

impl ResponsesStreamState {
    pub(crate) fn new() -> Self {
        Self {
            tool_tracker:          ResponsesToolIndexTracker::new(),
            accumulated_text:      String::new(),
            accumulated_reasoning: String::new(),
            has_function_call:     false,
            final_stop:            StopReason::Stop,
            final_usage:           None,
            tool_name_reverse_map: HashMap::new(),
        }
    }
}

/// Parse a single Responses API SSE event and emit `StreamDelta`s.
///
/// Returns `Some(true)` when the stream should terminate,
/// `Some(false)` on parse failure, `None` to continue.
pub(crate) fn parse_responses_event(
    event_type: &str,
    data: &str,
    tx: &mpsc::Sender<StreamDelta>,
    state: &mut ResponsesStreamState,
) -> Option<bool> {
    let parsed: Value = match serde_json::from_str(data) {
        Ok(v) => v,
        Err(_) => return Some(false),
    };

    match event_type {
        "response.output_text.delta" => {
            if let Some(delta) = parsed.get("delta").and_then(|d| d.as_str()) {
                state.accumulated_text.push_str(delta);
                let _ = tx.try_send(StreamDelta::TextDelta {
                    text: delta.to_owned(),
                });
            }
        }

        "response.refusal.delta" => {
            if let Some(delta) = parsed.get("delta").and_then(|d| d.as_str()) {
                let _ = tx.try_send(StreamDelta::TextDelta {
                    text: delta.to_owned(),
                });
            }
        }

        "response.reasoning_summary_text.delta" => {
            if let Some(delta) = parsed.get("delta").and_then(|d| d.as_str()) {
                state.accumulated_reasoning.push_str(delta);
                let _ = tx.try_send(StreamDelta::ReasoningDelta {
                    text: delta.to_owned(),
                });
            }
        }

        "response.reasoning_summary_part.added" => {
            tracing::trace!("reasoning summary part added");
        }

        "response.output_item.added" => {
            if let Some(item) = parsed.get("item") {
                let item_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
                let output_index = parsed
                    .get("output_index")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);

                match item_type {
                    "function_call" => {
                        let name = unsanitize_tool_name(
                            item.get("name")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_owned(),
                            &state.tool_name_reverse_map,
                        );
                        let call_id = item
                            .get("call_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_owned();
                        let index = state.tool_tracker.index_for(output_index);
                        let _ = tx.try_send(StreamDelta::ToolCallStart {
                            index,
                            id: call_id,
                            name,
                        });
                    }
                    "reasoning" => {
                        tracing::trace!(output_index, "reasoning item started");
                    }
                    "message" => {
                        tracing::trace!(output_index, "message item started");
                    }
                    other => {
                        tracing::debug!(item_type = other, "unhandled output_item.added type");
                    }
                }
            }
        }

        "response.output_item.done" => {
            if let Some(item) = parsed.get("item") {
                let item_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
                if item_type == "function_call" {
                    state.has_function_call = true;
                }
            }
        }

        "response.function_call_arguments.delta" => {
            if let Some(delta) = parsed.get("delta").and_then(|d| d.as_str()) {
                let output_index = parsed
                    .get("output_index")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let index = state.tool_tracker.index_for(output_index);
                let _ = tx.try_send(StreamDelta::ToolCallArgumentsDelta {
                    index,
                    arguments: delta.to_owned(),
                });
            }
        }

        "response.output_text.annotation.added" => {
            if let Some(annotation) = parsed.get("annotation") {
                let ann_type = annotation
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if ann_type == "url_citation" {
                    let url = annotation.get("url").and_then(|v| v.as_str()).unwrap_or("");
                    let title = annotation
                        .get("title")
                        .and_then(|v| v.as_str())
                        .unwrap_or(url);
                    if !url.is_empty() {
                        let citation = format!(" [{title}]({url})");
                        let _ = tx.try_send(StreamDelta::TextDelta { text: citation });
                    }
                }
            }
        }

        "response.created" => {
            if let Some(response) = parsed.get("response") {
                let id = response.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                let model = response
                    .get("model")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                tracing::debug!(response_id = id, model, "Responses API response created");
            }
        }

        "response.completed" | "response.incomplete" => {
            let is_incomplete = event_type == "response.incomplete";

            state.final_usage = parsed.get("response").and_then(|r| {
                let u = r.get("usage")?;
                let input = u.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                let output = u.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                Some(Usage {
                    prompt_tokens:     input,
                    completion_tokens: output,
                    total_tokens:      input + output,
                })
            });

            if is_incomplete {
                let reason = parsed
                    .get("response")
                    .and_then(|r| r.get("incomplete_details"))
                    .and_then(|d| d.get("reason"))
                    .and_then(|r| r.as_str());

                state.final_stop = match reason {
                    Some("max_output_tokens") => StopReason::Length,
                    Some("content_filter") => StopReason::ContentFilter,
                    Some(other) => {
                        tracing::warn!(
                            reason = other,
                            "Responses API response incomplete with unexpected reason"
                        );
                        if state.has_function_call {
                            StopReason::ToolCalls
                        } else {
                            StopReason::Stop
                        }
                    }
                    None => {
                        if state.has_function_call {
                            StopReason::ToolCalls
                        } else {
                            StopReason::Stop
                        }
                    }
                };
            } else {
                state.final_stop = if state.has_function_call {
                    StopReason::ToolCalls
                } else {
                    StopReason::Stop
                };
            }

            return Some(true);
        }

        "error" => {
            let code = parsed
                .get("code")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let message = parsed
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            tracing::error!(code, message, "Responses API error event");
        }

        _ => {
            tracing::debug!(event_type, "unhandled Responses API SSE event");
        }
    }

    None
}

/// Stream a Responses API response.
async fn stream_responses_api(
    response: reqwest::Response,
    tx: mpsc::Sender<StreamDelta>,
    model: String,
    sse_idle_timeout: Duration,
    reverse_map: HashMap<String, String>,
) -> Result<CompletionResponse> {
    let mut event_stream = response.bytes_stream().eventsource();
    let mut state = ResponsesStreamState::new();
    state.tool_name_reverse_map = reverse_map;

    loop {
        if tx.is_closed() {
            tracing::debug!("stream consumer disconnected, returning partial response");
            break;
        }

        let maybe_event = tokio::time::timeout(sse_idle_timeout, event_stream.next()).await;

        match maybe_event {
            Ok(Some(Ok(event))) => {
                if event.data == "[DONE]" {
                    break;
                }
                if let Some(terminal) =
                    parse_responses_event(&event.event, &event.data, &tx, &mut state)
                {
                    if terminal {
                        break;
                    }
                }
            }
            Ok(Some(Err(e))) => {
                return Err(KernelError::RetryableServer {
                    message: format!("Responses API SSE error: {e}").into(),
                });
            }
            Ok(None) => break,
            Err(_elapsed) => {
                tracing::warn!(
                    timeout_secs = sse_idle_timeout.as_secs(),
                    "Responses API SSE idle timeout — aborting stream"
                );
                return Err(KernelError::RetryableServer {
                    message: "SSE stream idle timeout".into(),
                });
            }
        }
    }

    // Stream-close salvage for the Responses API path. Providers routed
    // through this format (currently OpenAI + clones) shouldn't hit the
    // MiniMax failure mode, but the guard is symmetric with the chat
    // completions path and costs nothing when reasoning is empty.
    if state.accumulated_text.is_empty()
        && !state.accumulated_reasoning.is_empty()
        && !state.has_function_call
    {
        match salvage_after_think(&state.accumulated_reasoning) {
            Some(salvaged) => {
                tracing::warn!(
                    reasoning_len = state.accumulated_reasoning.len(),
                    salvaged_len = salvaged.len(),
                    "Responses API stream closed with empty content; salvaged text after </think>"
                );
                let _ = tx
                    .send(StreamDelta::TextDelta {
                        text: salvaged.clone(),
                    })
                    .await;
                state.accumulated_text.push_str(&salvaged);
            }
            None => {
                tracing::warn!(
                    reasoning_len = state.accumulated_reasoning.len(),
                    "Responses API stream closed with empty content and no salvageable text"
                );
                let _ = tx
                    .send(StreamDelta::Failure(
                        super::stream::StreamFailure::EmptyContent {
                            reasoning_len: state.accumulated_reasoning.len(),
                        },
                    ))
                    .await;
            }
        }
    }

    let _ = tx
        .send(StreamDelta::Done {
            stop_reason: state.final_stop,
            usage:       state.final_usage,
        })
        .await;

    let reasoning_content = non_empty(state.accumulated_reasoning);

    Ok(CompletionResponse {
        content: non_empty(state.accumulated_text),
        reasoning_content,
        tool_calls: vec![], // Populated by agent loop from StreamDelta events.
        stop_reason: state.final_stop,
        usage: state.final_usage,
        model,
    })
}

// ---------------------------------------------------------------------------
// LlmDriver implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl LlmDriver for OpenAiDriver {
    async fn complete(&self, mut request: CompletionRequest) -> Result<CompletionResponse> {
        let reverse_map = sanitize_request_tool_names(&mut request);
        let response = self.send_request(&request, false).await?;
        match self.api_format {
            ApiFormat::ChatCompletions => {
                let raw: RawCompletionResponse =
                    response.json().await.map_err(|e| KernelError::Provider {
                        message: format!("failed to parse LLM response: {e}").into(),
                    })?;
                Ok(parse_chat_completion_response(raw, &request, &reverse_map))
            }
            ApiFormat::Responses => {
                let raw: Value = response.json().await.map_err(|e| KernelError::Provider {
                    message: format!("failed to parse Responses API response: {e}").into(),
                })?;
                parse_responses_completion(raw, &request, &reverse_map)
            }
        }
    }

    async fn stream(
        &self,
        mut request: CompletionRequest,
        tx: mpsc::Sender<StreamDelta>,
    ) -> Result<CompletionResponse> {
        let reverse_map = sanitize_request_tool_names(&mut request);
        let response = self.send_request(&request, true).await?;
        match self.api_format {
            ApiFormat::ChatCompletions => {
                stream_chat_completions(
                    response,
                    tx,
                    request.model.clone(),
                    self.sse_idle_timeout,
                    reverse_map,
                )
                .await
            }
            ApiFormat::Responses => {
                stream_responses_api(
                    response,
                    tx,
                    request.model.clone(),
                    self.sse_idle_timeout,
                    reverse_map,
                )
                .await
            }
        }
    }

    async fn model_context_length(&self, model: &str) -> Option<usize> {
        let cache = self
            .models_cache
            .get_or_init(|| self.fetch_model_metadata())
            .await;
        cache.get(model).and_then(|m| m.context_length)
    }

    async fn model_supports_vision(&self, model: &str) -> Option<bool> {
        let cache = self
            .models_cache
            .get_or_init(|| self.fetch_model_metadata())
            .await;
        cache.get(model).map(|m| m.supports_vision)
    }
}

// ---------------------------------------------------------------------------
// LlmModelLister implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl LlmModelLister for OpenAiDriver {
    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        let response = self
            .send_authenticated_request(reqwest::Method::GET, "/models", None)
            .await?;

        let raw: RawModelsResponse = response.json().await.map_err(|e| KernelError::Provider {
            message: format!("failed to parse models response: {e}").into(),
        })?;

        let models = raw
            .data
            .into_iter()
            .map(|entry| ModelInfo {
                id:       entry.id,
                owned_by: entry.owned_by.unwrap_or_default(),
                created:  entry.created,
            })
            .collect();

        Ok(models)
    }
}

// ---------------------------------------------------------------------------
// LlmEmbedder implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl LlmEmbedder for OpenAiDriver {
    async fn embed(&self, request: EmbeddingRequest) -> Result<EmbeddingResponse> {
        let wire = WireEmbeddingRequest {
            model:      &request.model,
            input:      &request.input,
            dimensions: request.dimensions,
        };
        let body = serde_json::to_value(&wire).map_err(|e| KernelError::Provider {
            message: format!("failed to serialize embedding request: {e}").into(),
        })?;

        let response = self
            .send_authenticated_request(reqwest::Method::POST, "/embeddings", Some(body))
            .await?;

        let raw: RawEmbeddingResponse =
            response.json().await.map_err(|e| KernelError::Provider {
                message: format!("failed to parse embeddings response: {e}").into(),
            })?;

        // Sort by index to ensure the output order matches input order.
        let mut data = raw.data;
        data.sort_by_key(|d| d.index);

        let embeddings = data.into_iter().map(|d| d.embedding).collect();

        let usage = raw.usage.map(|u| Usage {
            prompt_tokens:     u.input.unwrap_or(0),
            completion_tokens: u.output.unwrap_or(0),
            total_tokens:      u.total.unwrap_or(0),
        });

        Ok(EmbeddingResponse {
            embeddings,
            model: raw.model,
            usage,
        })
    }
}

// ---------------------------------------------------------------------------
// StreamAccumulator
// ---------------------------------------------------------------------------

struct StreamAccumulator {
    text:                  String,
    reasoning:             String,
    think_parser:          super::think_tag::ThinkTagParser,
    tool_xml_parser:       super::tool_xml::ToolXmlParser,
    /// Auto-incrementing index for XML-extracted tool calls so they get
    /// unique slots in the agent loop's `pending_tool_calls` HashMap.
    xml_tool_index:        u32,
    tools:                 HashMap<u32, PendingToolCall>,
    stop_reason:           StopReason,
    usage:                 Option<Usage>,
    tool_name_reverse_map: HashMap<String, String>,
}

struct PendingToolCall {
    id:        String,
    name:      String,
    arguments: String,
    started:   bool,
}

impl StreamAccumulator {
    fn new() -> Self {
        Self {
            text:                  String::new(),
            reasoning:             String::new(),
            think_parser:          super::think_tag::ThinkTagParser::new(),
            tool_xml_parser:       super::tool_xml::ToolXmlParser::new(),
            xml_tool_index:        1000, // offset from JSON tool_calls (0-based)
            tools:                 HashMap::new(),
            tool_name_reverse_map: HashMap::new(),
            stop_reason:           StopReason::Stop,
            usage:                 None,
        }
    }

    async fn process_chunk(&mut self, chunk: &RawStreamChunk, tx: &mpsc::Sender<StreamDelta>) {
        for choice in &chunk.choices {
            // Text delta (split out embedded <think> blocks).
            if let Some(ref text) = choice.delta.content {
                if !text.is_empty() {
                    for segment in self.think_parser.push(text) {
                        match segment {
                            super::think_tag::Segment::Thinking(t) => {
                                self.reasoning.push_str(&t);
                                let _ = tx.send(StreamDelta::ReasoningDelta { text: t }).await;
                            }
                            super::think_tag::Segment::Text(t) => {
                                // Second pass: intercept XML tool calls
                                // that some models (MiniMax) embed in text.
                                for part in self.tool_xml_parser.push(&t) {
                                    match part {
                                        super::tool_xml::Segment::Text(t) => {
                                            self.text.push_str(&t);
                                            let _ =
                                                tx.send(StreamDelta::TextDelta { text: t }).await;
                                        }
                                        super::tool_xml::Segment::ToolCall { name, arguments } => {
                                            self.emit_xml_tool_call(&name, arguments, tx).await;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Reasoning delta
            if let Some(ref reasoning) = choice.delta.reasoning_content {
                if !reasoning.is_empty() {
                    self.reasoning.push_str(reasoning);
                    let _ = tx
                        .send(StreamDelta::ReasoningDelta {
                            text: reasoning.clone(),
                        })
                        .await;
                }
            }

            // Tool call deltas
            if let Some(ref tcs) = choice.delta.tool_calls {
                for tc in tcs {
                    let entry = self
                        .tools
                        .entry(tc.index)
                        .or_insert_with(|| PendingToolCall {
                            id:        String::new(),
                            name:      String::new(),
                            arguments: String::new(),
                            started:   false,
                        });

                    if let Some(ref id) = tc.id {
                        if !id.is_empty() {
                            entry.id = id.clone();
                        }
                    }

                    // Collect new arguments but defer sending the delta until
                    // after ToolCallStart has been emitted.  Some providers
                    // (e.g. OpenRouter) deliver name + arguments in a single
                    // SSE chunk; if we send ToolCallArgumentsDelta first the
                    // receiver has no pending entry yet and silently drops
                    // the arguments.
                    let mut new_args: Option<String> = None;

                    if let Some(ref func) = tc.function {
                        if let Some(ref name) = func.name {
                            if !name.is_empty() {
                                entry.name =
                                    unsanitize_tool_name(name.clone(), &self.tool_name_reverse_map);
                            }
                        }
                        if let Some(ref args) = func.arguments {
                            if !args.is_empty() {
                                entry.arguments.push_str(args);
                                new_args = Some(args.clone());
                            }
                        }
                    }

                    // Emit ToolCallStart exactly once when we first get both id and name.
                    // This MUST happen before ToolCallArgumentsDelta so the
                    // receiver registers the pending entry before accumulating
                    // arguments.
                    if !entry.started && !entry.id.is_empty() && !entry.name.is_empty() {
                        entry.started = true;
                        let _ = tx
                            .send(StreamDelta::ToolCallStart {
                                index: tc.index,
                                id:    entry.id.clone(),
                                name:  entry.name.clone(),
                            })
                            .await;
                    }

                    // Now send the argument delta (receiver entry is guaranteed
                    // to exist if ToolCallStart was emitted above).
                    if let Some(args) = new_args {
                        let _ = tx
                            .send(StreamDelta::ToolCallArgumentsDelta {
                                index:     tc.index,
                                arguments: args,
                            })
                            .await;
                    }
                }
            }

            // Finish reason
            if let Some(ref reason) = choice.finish_reason {
                self.stop_reason = parse_stop_reason(Some(reason.as_str()));
            }
        }

        // Usage (some providers send it in the last chunk)
        if let Some(ref usage) = chunk.usage {
            self.usage = Some(Usage {
                prompt_tokens:     usage.input.unwrap_or(0),
                completion_tokens: usage.output.unwrap_or(0),
                total_tokens:      usage.total.unwrap_or(0),
            });
        }
    }

    fn collect_tools(tools: HashMap<u32, PendingToolCall>) -> Vec<ToolCallRequest> {
        let mut entries: Vec<(u32, PendingToolCall)> = tools.into_iter().collect();
        entries.sort_by_key(|(idx, _)| *idx);
        entries
            .into_iter()
            .map(|(_, pt)| ToolCallRequest {
                id:        pt.id,
                name:      pt.name,
                arguments: pt.arguments,
            })
            .collect()
    }

    /// Convert an XML-extracted tool call into the same `StreamDelta`
    /// sequence that JSON tool_calls produce.
    async fn emit_xml_tool_call(
        &mut self,
        name: &str,
        arguments: serde_json::Value,
        tx: &mpsc::Sender<StreamDelta>,
    ) {
        let idx = self.xml_tool_index;
        self.xml_tool_index += 1;
        let id = format!("xml-tool-{idx}");
        let args_str = serde_json::to_string(&arguments).unwrap_or_default();
        let name = unsanitize_tool_name(name.to_owned(), &self.tool_name_reverse_map);

        tracing::debug!(
            tool_name = %name,
            index = idx,
            "intercepted XML tool call from content stream"
        );

        self.tools.insert(
            idx,
            PendingToolCall {
                id:        id.clone(),
                name:      name.clone(),
                arguments: args_str.clone(),
                started:   true,
            },
        );

        let _ = tx
            .send(StreamDelta::ToolCallStart {
                index: idx,
                id,
                name,
            })
            .await;
        let _ = tx
            .send(StreamDelta::ToolCallArgumentsDelta {
                index:     idx,
                arguments: args_str,
            })
            .await;
    }

    async fn finalize(
        mut self,
        tx: &mpsc::Sender<StreamDelta>,
        model: String,
    ) -> CompletionResponse {
        // Flush trailing partial content that was buffered for tag boundary
        // detection.  Think tags first, then XML tool calls.
        for segment in self.think_parser.flush() {
            match segment {
                super::think_tag::Segment::Text(t) => {
                    // Run flushed text through tool_xml_parser too.
                    for part in self.tool_xml_parser.push(&t) {
                        match part {
                            super::tool_xml::Segment::Text(t) => {
                                self.text.push_str(&t);
                                let _ = tx.send(StreamDelta::TextDelta { text: t }).await;
                            }
                            super::tool_xml::Segment::ToolCall { name, arguments } => {
                                self.emit_xml_tool_call(&name, arguments, tx).await;
                            }
                        }
                    }
                }
                super::think_tag::Segment::Thinking(t) => {
                    self.reasoning.push_str(&t);
                    let _ = tx.send(StreamDelta::ReasoningDelta { text: t }).await;
                }
            }
        }
        // Flush any remaining XML tool parser buffer.
        for part in self.tool_xml_parser.flush() {
            match part {
                super::tool_xml::Segment::Text(t) => {
                    self.text.push_str(&t);
                    let _ = tx.send(StreamDelta::TextDelta { text: t }).await;
                }
                super::tool_xml::Segment::ToolCall { name, arguments } => {
                    self.emit_xml_tool_call(&name, arguments, tx).await;
                }
            }
        }

        let Self {
            mut text,
            reasoning,
            tools,
            stop_reason,
            usage,
            ..
        } = self;
        let tool_calls = Self::collect_tools(tools);

        // Stream-close salvage: some reasoning-capable providers (MiniMax-M2)
        // emit the real answer inside `reasoning_content` after `</think>`
        // and then close the stream without ever emitting `content`. When we
        // see reasoning but no text, try to extract text after the last
        // `</think>` tag; if that fails, surface a typed failure so upstream
        // consumers can avoid writing an empty assistant record.
        if text.is_empty() && !reasoning.is_empty() && tool_calls.is_empty() {
            match salvage_after_think(&reasoning) {
                Some(salvaged) => {
                    tracing::warn!(
                        reasoning_len = reasoning.len(),
                        salvaged_len = salvaged.len(),
                        "stream closed with empty content; salvaged text after </think>"
                    );
                    let _ = tx
                        .send(StreamDelta::TextDelta {
                            text: salvaged.clone(),
                        })
                        .await;
                    text.push_str(&salvaged);
                }
                None => {
                    tracing::warn!(
                        reasoning_len = reasoning.len(),
                        "stream closed with empty content and no salvageable text after </think>"
                    );
                    let _ = tx
                        .send(StreamDelta::Failure(
                            super::stream::StreamFailure::EmptyContent {
                                reasoning_len: reasoning.len(),
                            },
                        ))
                        .await;
                }
            }
        }

        let _ = tx.send(StreamDelta::Done { stop_reason, usage }).await;

        CompletionResponse {
            content: non_empty(text),
            reasoning_content: non_empty(reasoning),
            tool_calls,
            stop_reason,
            usage,
            model,
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn non_empty(s: String) -> Option<String> { if s.is_empty() { None } else { Some(s) } }

/// Attempt to recover assistant text trailing the last `</think>` tag in a
/// reasoning buffer.
///
/// Observed MiniMax-M2 failure mode: the model streams
/// `reasoning_content` containing a complete `<think>...</think>` block (and
/// sometimes the real answer in the same field after the closing tag), then
/// closes the SSE stream without ever emitting `content`. Callers use this
/// helper at stream close to salvage that trailing text.
///
/// Returns `None` when the buffer contains no `</think>` tag, or when the
/// text after the last one is whitespace-only.
fn salvage_after_think(reasoning: &str) -> Option<String> {
    const CLOSE_TAG: &str = "</think>";
    let tail = reasoning.rsplit_once(CLOSE_TAG).map(|(_, rest)| rest)?;
    let trimmed = tail.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

/// Truncate a string to at most `max_bytes` bytes without splitting a UTF-8
/// code point.
fn truncate_utf8(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

fn parse_stop_reason(reason: Option<&str>) -> StopReason {
    match reason {
        Some("stop" | "end_turn") => StopReason::Stop,
        Some("tool_calls" | "function_call" | "tool_use") => StopReason::ToolCalls,
        Some("length" | "max_tokens") => StopReason::Length,
        Some("content_filter") => StopReason::ContentFilter,
        _ => StopReason::Stop,
    }
}

// ---------------------------------------------------------------------------
// Wire types — typed request serialization
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct ChatRequest<'a> {
    model:                 &'a str,
    messages:              Vec<WireMessage<'a>>,
    stream:                bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature:           Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens:            Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_completion_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools:                 Option<Vec<WireTool<'a>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice:           Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parallel_tool_calls:   Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking:              Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream_options:        Option<WireStreamOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    frequency_penalty:     Option<f32>,
    /// GLM-specific: stream tool-call argument deltas instead of buffering.
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_stream:           Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p:                 Option<f32>,
}

#[derive(Serialize)]
struct WireStreamOptions {
    include_usage: bool,
}

#[derive(Serialize)]
struct WireTool<'a> {
    r#type:   &'static str,
    function: WireToolFunction<'a>,
}

#[derive(Serialize)]
struct WireToolFunction<'a> {
    name:        &'a str,
    description: &'a str,
    parameters:  &'a serde_json::Value,
}

#[derive(Serialize)]
#[serde(untagged)]
enum WireContent<'a> {
    Text(&'a str),
    Multimodal(Vec<WireContentPart<'a>>),
}

#[derive(Serialize)]
#[serde(tag = "type")]
enum WireContentPart<'a> {
    #[serde(rename = "text")]
    Text { text: &'a str },
    #[serde(rename = "image_url")]
    ImageUrl { image_url: WireImageUrl<'a> },
}

#[derive(Serialize)]
struct WireImageUrl<'a> {
    url: std::borrow::Cow<'a, str>,
}

#[derive(Serialize)]
struct WireMessage<'a> {
    role:              &'static str,
    /// Per OpenAI spec, assistant messages with tool_calls may have `content:
    /// null`.
    content:           Option<WireContent<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls:        Option<Vec<WireToolCallRef<'a>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id:      Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_content: Option<&'a str>,
}

#[derive(Serialize)]
struct WireToolCallRef<'a> {
    id:       &'a str,
    r#type:   &'static str,
    function: WireFunctionRef<'a>,
}

#[derive(Serialize)]
struct WireFunctionRef<'a> {
    name:      &'a str,
    arguments: &'a str,
}

impl<'a> ChatRequest<'a> {
    fn from_completion(request: &'a CompletionRequest, stream: bool) -> Self {
        let provider = detect_provider_family(None, &request.model);

        let messages: Vec<WireMessage<'a>> = request
            .messages
            .iter()
            .map(|m| WireMessage::from_message(m, request.emit_reasoning))
            .collect();

        let (tools, tool_choice, parallel_tool_calls) = if request.tools.is_empty() {
            (None, None, None)
        } else {
            let tools: Vec<WireTool<'a>> = request
                .tools
                .iter()
                .map(|t| WireTool {
                    r#type:   "function",
                    function: WireToolFunction {
                        name:        &t.name,
                        description: &t.description,
                        parameters:  &t.parameters,
                    },
                })
                .collect();

            let tool_choice = match &request.tool_choice {
                ToolChoice::Auto => Some(serde_json::json!("auto")),
                ToolChoice::None => Some(serde_json::json!("none")),
                ToolChoice::Required => Some(serde_json::json!("required")),
                ToolChoice::Specific(name) => {
                    Some(serde_json::json!({"type": "function", "function": {"name": name}}))
                }
            };

            let parallel = Some(request.parallel_tool_calls);

            (Some(tools), tool_choice, parallel)
        };

        let thinking = request.thinking.as_ref().and_then(|t| {
            if t.enabled {
                let mut obj = serde_json::json!({"type": "enabled"});
                if let Some(budget) = t.budget_tokens {
                    obj["budget_tokens"] = serde_json::json!(budget);
                }
                Some(obj)
            } else {
                None
            }
        });

        let stream_options = if stream {
            Some(WireStreamOptions {
                include_usage: true,
            })
        } else {
            None
        };

        Self {
            model: &request.model,
            messages,
            stream,
            temperature: request.temperature,
            // Send both: `max_tokens` (legacy) and `max_completion_tokens` (new)
            // so the request works with older and newer OpenAI-compatible APIs.
            max_tokens: request.max_tokens,
            max_completion_tokens: request.max_tokens,
            tools,
            tool_choice,
            parallel_tool_calls,
            thinking,
            stream_options,
            frequency_penalty: request.frequency_penalty,
            // GLM requires `tool_stream: true` for streaming tool-call argument
            // deltas; omitted for other providers via skip_serializing_if.
            tool_stream: if provider == LlmProviderFamily::Glm && !request.tools.is_empty() {
                Some(true)
            } else {
                None
            },
            top_p: request.top_p,
        }
    }
}

impl<'a> WireMessage<'a> {
    fn from_message(msg: &'a Message, emit_reasoning: bool) -> Self {
        let role = match msg.role {
            Role::System => "system",
            Role::Developer => "developer",
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "tool",
        };

        let wire_content = match &msg.content {
            MessageContent::Text(text) => WireContent::Text(text),
            MessageContent::Multimodal(blocks) => {
                let parts = blocks
                    .iter()
                    .map(|b| match b {
                        ContentBlock::Text { text } => WireContentPart::Text { text },
                        ContentBlock::ImageUrl { url } => WireContentPart::ImageUrl {
                            image_url: WireImageUrl {
                                url: std::borrow::Cow::Borrowed(url),
                            },
                        },
                        ContentBlock::ImageBase64 { media_type, data } => {
                            let data_uri = format!("data:{media_type};base64,{data}");
                            WireContentPart::ImageUrl {
                                image_url: WireImageUrl {
                                    url: std::borrow::Cow::Owned(data_uri),
                                },
                            }
                        }
                        // Audio blocks should be transcribed before reaching the LLM.
                        // If one leaks through, convert to a text placeholder so the
                        // prompt is not silently incomplete.
                        ContentBlock::AudioBase64 { .. } => WireContentPart::Text {
                            text: "[audio: not transcribed]",
                        },
                        // Documents are extracted client-side and delivered as
                        // a sibling Text block; leak the raw-bytes variant
                        // through as a placeholder so the prompt structure is
                        // preserved.
                        ContentBlock::FileBase64 { .. } => WireContentPart::Text {
                            text: "[document: raw bytes omitted — extracted text provided \
                                   separately]",
                        },
                    })
                    .collect();
                WireContent::Multimodal(parts)
            }
        };

        let tool_calls = if msg.tool_calls.is_empty() {
            None
        } else {
            Some(
                msg.tool_calls
                    .iter()
                    .map(|tc| WireToolCallRef {
                        id:       &tc.id,
                        r#type:   "function",
                        function: WireFunctionRef {
                            name:      &tc.name,
                            arguments: &tc.arguments,
                        },
                    })
                    .collect(),
            )
        };

        // Per OpenAI spec, assistant messages with tool_calls should have
        // content: null when there is no meaningful text content.
        let content = if msg.role == Role::Assistant
            && tool_calls.is_some()
            && msg.content.as_text().is_empty()
        {
            None
        } else {
            Some(wire_content)
        };

        Self {
            role,
            content,
            tool_calls,
            tool_call_id: msg.tool_call_id.as_deref(),
            reasoning_content: if emit_reasoning {
                msg.reasoning_content.as_deref()
            } else {
                None
            },
        }
    }
}

// ---------------------------------------------------------------------------
// Raw SSE deserialization types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct RawStreamChunk {
    #[allow(dead_code)]
    id:      Option<String>,
    choices: Vec<RawStreamChoice>,
    usage:   Option<RawUsage>,
}

#[derive(Debug, Deserialize)]
struct RawStreamChoice {
    #[allow(dead_code)]
    index:         u32,
    delta:         RawDelta,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawDelta {
    content:           Option<String>,
    #[serde(alias = "reasoning")]
    reasoning_content: Option<String>,
    tool_calls:        Option<Vec<RawToolCallChunk>>,
    #[allow(dead_code)]
    role:              Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawToolCallChunk {
    index:    u32,
    id:       Option<String>,
    function: Option<RawFunctionChunk>,
}

#[derive(Debug, Deserialize)]
struct RawFunctionChunk {
    name:      Option<String>,
    arguments: Option<String>,
}

/// Token usage from the provider response.
///
/// Field names vary across providers (OpenAI uses `prompt_tokens` /
/// `completion_tokens`, Anthropic uses `input_tokens` / `output_tokens`),
/// so we accept all common variants via serde aliases.
#[derive(Debug, Deserialize)]
struct RawUsage {
    #[serde(alias = "prompt_tokens", alias = "input_tokens")]
    input:  Option<u32>,
    #[serde(alias = "completion_tokens", alias = "output_tokens")]
    output: Option<u32>,
    #[serde(alias = "total_tokens")]
    total:  Option<u32>,
}

// --- Non-streaming response types ---

#[derive(Debug, Deserialize)]
struct RawCompletionResponse {
    choices: Vec<RawResponseChoice>,
    usage:   Option<RawUsage>,
    model:   Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawResponseChoice {
    message:       RawResponseMessage,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawResponseMessage {
    content:           Option<String>,
    #[serde(alias = "reasoning")]
    reasoning_content: Option<String>,
    tool_calls:        Option<Vec<RawResponseToolCall>>,
}

#[derive(Debug, Deserialize)]
struct RawResponseToolCall {
    id:       String,
    function: RawResponseFunction,
}

#[derive(Debug, Deserialize)]
struct RawResponseFunction {
    name:      String,
    arguments: String,
}

// ---------------------------------------------------------------------------
// Wire types — /models endpoint
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct RawModelsResponse {
    data: Vec<RawModelEntry>,
}

#[derive(Deserialize)]
struct RawModelEntry {
    id:             String,
    #[serde(default)]
    owned_by:       Option<String>,
    #[serde(default)]
    created:        Option<u64>,
    /// Context window size in tokens.  Returned by providers like
    /// OpenRouter but absent from the standard OpenAI response.
    #[serde(default)]
    context_length: Option<usize>,
    /// Model architecture metadata including input/output modalities.
    /// Returned by OpenRouter but absent from the standard OpenAI response.
    #[serde(default)]
    architecture:   Option<RawModelArchitecture>,
}

#[derive(Deserialize)]
struct RawModelArchitecture {
    #[serde(default)]
    input_modalities: Vec<String>,
}

// ---------------------------------------------------------------------------
// Wire types — /embeddings endpoint
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct WireEmbeddingRequest<'a> {
    model:      &'a str,
    input:      &'a [String],
    #[serde(skip_serializing_if = "Option::is_none")]
    dimensions: Option<usize>,
}

#[derive(Deserialize)]
struct RawEmbeddingResponse {
    data:  Vec<RawEmbeddingData>,
    model: String,
    #[serde(default)]
    usage: Option<RawUsage>,
}

#[derive(Deserialize)]
struct RawEmbeddingData {
    embedding: Vec<f32>,
    index:     u32,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::types::{Message, ThinkingConfig, ToolDefinition};

    #[test]
    fn raw_model_entry_parses_input_modalities() {
        let json = serde_json::json!({
            "id": "openai/gpt-4o",
            "context_length": 128000,
            "architecture": {
                "input_modalities": ["text", "image"],
                "output_modalities": ["text"]
            }
        });
        let entry: RawModelEntry = serde_json::from_value(json).unwrap();
        assert_eq!(entry.id, "openai/gpt-4o");
        assert_eq!(entry.context_length, Some(128000));
        let arch = entry.architecture.unwrap();
        assert!(arch.input_modalities.contains(&"image".to_string()));
    }

    #[test]
    fn raw_model_entry_missing_architecture() {
        let json = serde_json::json!({
            "id": "some-model",
            "context_length": 4096
        });
        let entry: RawModelEntry = serde_json::from_value(json).unwrap();
        assert!(entry.architecture.is_none());
    }

    #[test]
    fn build_responses_request_user_message_uses_input_text_format() {
        let request = CompletionRequest {
            model:               "codex-mini".into(),
            messages:            vec![Message::user("hello world")],
            tools:               vec![],
            temperature:         None,
            max_tokens:          None,
            thinking:            None,
            tool_choice:         Default::default(),
            parallel_tool_calls: false,
            frequency_penalty:   None,
            top_p:               None,
            emit_reasoning:      false,
        };

        let body = build_responses_request(&request, ApiFormat::Responses);
        let input = body["input"].as_array().expect("input should be array");
        assert_eq!(input.len(), 1);

        let msg = &input[0];
        assert_eq!(msg["type"], "message");
        assert_eq!(msg["role"], "user");

        let content = msg["content"].as_array().expect("content should be array");
        assert_eq!(content.len(), 1);
        assert_eq!(content[0]["type"], "input_text");
        assert_eq!(content[0]["text"], "hello world");
    }

    #[test]
    fn build_responses_request_assistant_message_uses_output_text_format() {
        let request = CompletionRequest {
            model:               "codex-mini".into(),
            messages:            vec![Message::assistant("I can help")],
            tools:               vec![],
            temperature:         None,
            max_tokens:          None,
            thinking:            None,
            tool_choice:         Default::default(),
            parallel_tool_calls: false,
            frequency_penalty:   None,
            top_p:               None,
            emit_reasoning:      false,
        };

        let body = build_responses_request(&request, ApiFormat::Responses);
        let input = body["input"].as_array().expect("input should be array");
        let content = input[0]["content"]
            .as_array()
            .expect("content should be array");
        assert_eq!(content[0]["type"], "output_text");
        assert_eq!(content[0]["text"], "I can help");
    }

    #[test]
    fn build_responses_request_includes_codex_reasoning_defaults() {
        let request = CompletionRequest {
            model:               "codex-mini".into(),
            messages:            vec![],
            tools:               vec![],
            temperature:         None,
            max_tokens:          None,
            thinking:            None,
            tool_choice:         Default::default(),
            parallel_tool_calls: false,
            frequency_penalty:   None,
            top_p:               None,
            emit_reasoning:      false,
        };

        let body = build_responses_request(&request, ApiFormat::Responses);
        assert_eq!(body["store"], false);
        assert!(body.get("truncation").is_none());
        assert_eq!(body["reasoning"]["effort"], "medium");
        assert_eq!(body["reasoning"]["summary"], "auto");
    }

    #[test]
    fn build_responses_request_omits_chat_only_fields_and_keeps_tools() {
        let request = CompletionRequest {
            model:               "codex-mini".into(),
            messages:            vec![],
            tools:               vec![ToolDefinition {
                name:        "read_file".into(),
                description: "Read a file".into(),
                parameters:  serde_json::json!({"type": "object"}),
            }],
            temperature:         None,
            max_tokens:          Some(4096),
            thinking:            None,
            tool_choice:         Default::default(),
            parallel_tool_calls: true,
            frequency_penalty:   None,
            top_p:               None,
            emit_reasoning:      false,
        };

        let body = build_responses_request(&request, ApiFormat::Responses);
        assert!(body.get("max_output_tokens").is_none());
        assert!(body.get("parallel_tool_calls").is_none());
        assert_eq!(body["tools"].as_array().expect("tools array").len(), 1);
    }

    #[test]
    fn build_responses_request_high_thinking_budget_maps_to_high_effort() {
        let request = CompletionRequest {
            model:               "codex-mini".into(),
            messages:            vec![],
            tools:               vec![],
            temperature:         None,
            max_tokens:          None,
            thinking:            Some(ThinkingConfig {
                enabled:       true,
                budget_tokens: Some(20_000),
                effort:        None,
            }),
            tool_choice:         Default::default(),
            parallel_tool_calls: false,
            frequency_penalty:   None,
            top_p:               None,
            emit_reasoning:      false,
        };

        let body = build_responses_request(&request, ApiFormat::Responses);
        assert_eq!(body["reasoning"]["effort"], "high");
    }

    #[test]
    fn reasoning_effort_clamp_handles_gpt_5_4_quirks() {
        // gpt-5.4 accepts none | low | medium | high | xhigh. Off → None_
        // (wire "none"); Minimal isn't accepted so it bumps to Low.
        assert_eq!(
            ReasoningEffort::Off.clamp_for_model("gpt-5.4"),
            ReasoningEffort::None_
        );
        assert_eq!(
            ReasoningEffort::Minimal.clamp_for_model("gpt-5.4"),
            ReasoningEffort::Low
        );
        assert_eq!(
            ReasoningEffort::Xhigh.clamp_for_model("gpt-5.4-mini"),
            ReasoningEffort::Xhigh
        );
    }

    #[test]
    fn reasoning_effort_clamp_preserves_minimal_for_other_models() {
        // Non-gpt-5.4 reasoning models accept Minimal. Legacy reasoning
        // families (`o*`, `codex-*`) cap Xhigh at High.
        assert_eq!(
            ReasoningEffort::Off.clamp_for_model("gpt-5"),
            ReasoningEffort::Minimal
        );
        assert_eq!(
            ReasoningEffort::Minimal.clamp_for_model("gpt-5"),
            ReasoningEffort::Minimal
        );
        assert_eq!(
            ReasoningEffort::Xhigh.clamp_for_model("codex-mini"),
            ReasoningEffort::High
        );
        assert_eq!(
            ReasoningEffort::Xhigh.clamp_for_model("o3"),
            ReasoningEffort::High
        );
    }

    #[test]
    fn reasoning_effort_clamp_keeps_xhigh_on_gpt_5_family() {
        // gpt-5 (not just gpt-5.4) accepts Xhigh; the old budget→bucket
        // ladder silently downgraded it to High.
        assert_eq!(
            ReasoningEffort::Xhigh.clamp_for_model("gpt-5"),
            ReasoningEffort::Xhigh
        );
        assert_eq!(
            ReasoningEffort::Xhigh.clamp_for_model("gpt-5-mini"),
            ReasoningEffort::Xhigh
        );
    }

    #[test]
    fn reasoning_effort_clamp_strips_vendor_prefix() {
        // A router that prepends `<vendor>/` must not defeat family
        // detection — gpt-5.4 quirks still apply to `openai/gpt-5.4`.
        assert_eq!(
            ReasoningEffort::Minimal.clamp_for_model("openai/gpt-5.4"),
            ReasoningEffort::Low
        );
        assert_eq!(
            ReasoningEffort::Off.clamp_for_model("openai/gpt-5.4"),
            ReasoningEffort::None_
        );
        assert_eq!(
            ReasoningEffort::Xhigh.clamp_for_model("openai/gpt-5"),
            ReasoningEffort::Xhigh
        );
    }

    #[test]
    fn resolve_reasoning_effort_disabled_ignores_effort_hint() {
        // `enabled: false` is authoritative — a stray `effort: High` must
        // not leak through and ask the API for reasoning on a disabled turn.
        let thinking = ThinkingConfig {
            enabled:       false,
            budget_tokens: None,
            effort:        Some(ReasoningEffort::High),
        };
        assert_eq!(
            resolve_reasoning_effort("gpt-5.4", Some(&thinking)),
            ReasoningEffort::None_
        );
        assert_eq!(
            resolve_reasoning_effort("gpt-5", Some(&thinking)),
            ReasoningEffort::Minimal
        );
    }

    #[test]
    fn build_responses_request_uses_explicit_effort_hint() {
        // Xhigh round-trips end-to-end; the pre-typed-enum ladder capped
        // at "high" and silently lost the user's selection.
        let request = CompletionRequest {
            model:               "gpt-5.4".into(),
            messages:            vec![],
            tools:               vec![],
            temperature:         None,
            max_tokens:          None,
            thinking:            Some(ThinkingConfig {
                enabled:       true,
                budget_tokens: Some(32_768),
                effort:        Some(ReasoningEffort::Xhigh),
            }),
            tool_choice:         Default::default(),
            parallel_tool_calls: false,
            frequency_penalty:   None,
            top_p:               None,
            emit_reasoning:      false,
        };
        let body = build_responses_request(&request, ApiFormat::Responses);
        assert_eq!(body["reasoning"]["effort"], "xhigh");
    }

    #[test]
    fn build_responses_request_gpt_5_4_off_maps_to_none() {
        // Regression: pre-fix, Off was sent as `"minimal"` which gpt-5.4
        // rejects with HTTP 400. Must now emit `"none"`.
        let request = CompletionRequest {
            model:               "gpt-5.4".into(),
            messages:            vec![],
            tools:               vec![],
            temperature:         None,
            max_tokens:          None,
            thinking:            Some(ThinkingConfig {
                enabled:       false,
                budget_tokens: None,
                effort:        Some(ReasoningEffort::Off),
            }),
            tool_choice:         Default::default(),
            parallel_tool_calls: false,
            frequency_penalty:   None,
            top_p:               None,
            emit_reasoning:      false,
        };
        let body = build_responses_request(&request, ApiFormat::Responses);
        assert_eq!(body["reasoning"]["effort"], "none");
    }

    #[test]
    fn build_responses_request_formats_tool_call_and_tool_result() {
        let request = CompletionRequest {
            model:               "codex-mini".into(),
            messages:            vec![
                Message::assistant_with_tool_calls(
                    "",
                    vec![super::super::types::ToolCallRequest {
                        id:        "call_123".into(),
                        name:      "read_file".into(),
                        arguments: r#"{"path":"foo.rs"}"#.into(),
                    }],
                ),
                Message::tool_result("call_123", "file contents here"),
            ],
            tools:               vec![],
            temperature:         None,
            max_tokens:          None,
            thinking:            None,
            tool_choice:         Default::default(),
            parallel_tool_calls: false,
            frequency_penalty:   None,
            top_p:               None,
            emit_reasoning:      false,
        };

        let body = build_responses_request(&request, ApiFormat::Responses);
        let input = body["input"].as_array().expect("input should be array");

        assert_eq!(input[0]["type"], "function_call");
        assert_eq!(input[0]["name"], "read_file");
        assert_eq!(input[0]["call_id"], "call_123");

        assert_eq!(input[1]["type"], "function_call_output");
        assert_eq!(input[1]["call_id"], "call_123");
        assert_eq!(input[1]["output"], "file contents here");
    }

    #[test]
    fn parse_responses_text_delta_event() {
        let (tx, mut rx) = mpsc::channel(16);
        let mut state = ResponsesStreamState::new();

        let data = r#"{"delta":"hello ","item_id":"item_1","output_index":0}"#;
        let result = parse_responses_event("response.output_text.delta", data, &tx, &mut state);
        assert!(result.is_none());

        let delta = rx.try_recv().expect("should receive delta");
        match delta {
            StreamDelta::TextDelta { text } => assert_eq!(text, "hello "),
            other => panic!("unexpected delta: {other:?}"),
        }
        assert_eq!(state.accumulated_text, "hello ");
    }

    #[test]
    fn parse_responses_reasoning_delta_event() {
        let (tx, mut rx) = mpsc::channel(16);
        let mut state = ResponsesStreamState::new();

        let data = r#"{"delta":"thinking...","summary_index":0}"#;
        let result = parse_responses_event(
            "response.reasoning_summary_text.delta",
            data,
            &tx,
            &mut state,
        );
        assert!(result.is_none());

        let delta = rx.try_recv().expect("should receive reasoning delta");
        match delta {
            StreamDelta::ReasoningDelta { text } => assert_eq!(text, "thinking..."),
            other => panic!("unexpected delta: {other:?}"),
        }
        assert_eq!(state.accumulated_reasoning, "thinking...");
    }

    #[test]
    fn parse_responses_tool_call_flow() {
        let (tx, mut rx) = mpsc::channel(16);
        let mut state = ResponsesStreamState::new();

        let added = r#"{"item":{"type":"function_call","id":"item_1","call_id":"call_abc","name":"read_file","arguments":""},"output_index":2}"#;
        parse_responses_event("response.output_item.added", added, &tx, &mut state);

        let start = rx.try_recv().expect("should receive ToolCallStart");
        match start {
            StreamDelta::ToolCallStart { index, id, name } => {
                assert_eq!(index, 0);
                assert_eq!(id, "call_abc");
                assert_eq!(name, "read_file");
            }
            other => panic!("unexpected: {other:?}"),
        }

        let args_delta = r#"{"delta":"{\"path\"","output_index":2}"#;
        parse_responses_event(
            "response.function_call_arguments.delta",
            args_delta,
            &tx,
            &mut state,
        );

        let arg = rx.try_recv().expect("should receive args delta");
        match arg {
            StreamDelta::ToolCallArgumentsDelta { index, arguments } => {
                assert_eq!(index, 0);
                assert_eq!(arguments, r#"{"path""#);
            }
            other => panic!("unexpected: {other:?}"),
        }

        let done = r#"{"item":{"type":"function_call","id":"item_1","call_id":"call_abc","name":"read_file","arguments":"{\"path\":\"foo.rs\"}"},"output_index":2}"#;
        parse_responses_event("response.output_item.done", done, &tx, &mut state);
        assert!(state.has_function_call);
    }

    #[test]
    fn parse_responses_completed_event_with_usage() {
        let (tx, _rx) = mpsc::channel(16);
        let mut state = ResponsesStreamState::new();

        let data = r#"{"response":{"status":"completed","usage":{"input_tokens":100,"output_tokens":50}}}"#;
        let result = parse_responses_event("response.completed", data, &tx, &mut state);
        assert_eq!(result, Some(true));
        assert_eq!(state.final_stop, StopReason::Stop);

        let usage = state.final_usage.expect("should have usage");
        assert_eq!(usage.prompt_tokens, 100);
        assert_eq!(usage.completion_tokens, 50);
        assert_eq!(usage.total_tokens, 150);
    }

    #[test]
    fn parse_responses_completed_with_function_call_yields_tool_calls_stop() {
        let (tx, _rx) = mpsc::channel(16);
        let mut state = ResponsesStreamState::new();
        state.has_function_call = true;

        let data =
            r#"{"response":{"status":"completed","usage":{"input_tokens":10,"output_tokens":5}}}"#;
        let result = parse_responses_event("response.completed", data, &tx, &mut state);
        assert_eq!(result, Some(true));
        assert_eq!(state.final_stop, StopReason::ToolCalls);
    }

    #[test]
    fn parse_responses_incomplete_max_output_tokens() {
        let (tx, _rx) = mpsc::channel(16);
        let mut state = ResponsesStreamState::new();

        let data = r#"{"response":{"incomplete_details":{"reason":"max_output_tokens"},"usage":{"input_tokens":50,"output_tokens":200}}}"#;
        let result = parse_responses_event("response.incomplete", data, &tx, &mut state);
        assert_eq!(result, Some(true));
        assert_eq!(state.final_stop, StopReason::Length);
    }

    #[test]
    fn parse_responses_incomplete_content_filter() {
        let (tx, _rx) = mpsc::channel(16);
        let mut state = ResponsesStreamState::new();

        let data = r#"{"response":{"incomplete_details":{"reason":"content_filter"},"usage":{"input_tokens":50,"output_tokens":10}}}"#;
        let result = parse_responses_event("response.incomplete", data, &tx, &mut state);
        assert_eq!(result, Some(true));
        assert_eq!(state.final_stop, StopReason::ContentFilter);
    }

    #[test]
    fn parse_responses_unknown_event_type_does_not_crash() {
        let (tx, _rx) = mpsc::channel(16);
        let mut state = ResponsesStreamState::new();

        let data = r#"{"some":"data"}"#;
        let result = parse_responses_event("response.something.new", data, &tx, &mut state);
        assert!(result.is_none());
    }

    // -----------------------------------------------------------------
    // Stream-close salvage (issue #1632)
    // -----------------------------------------------------------------

    #[test]
    fn salvage_after_think_extracts_trailing_text() {
        let reasoning = "<think>planning...</think>\nHere is the answer.";
        let salvaged = salvage_after_think(reasoning).expect("should salvage");
        assert_eq!(salvaged, "Here is the answer.");
    }

    #[test]
    fn salvage_after_think_prefers_last_close_tag() {
        // Defensive: malformed reasoning with multiple close tags — take the
        // tail after the final one, matching real-world MiniMax output where
        // the last </think> marks the transition to the answer.
        let reasoning = "<think>step1</think>intermediate<think>step2</think>final answer";
        let salvaged = salvage_after_think(reasoning).expect("should salvage");
        assert_eq!(salvaged, "final answer");
    }

    #[test]
    fn salvage_after_think_no_close_tag_returns_none() {
        let reasoning = "unterminated reasoning with no close tag";
        assert!(salvage_after_think(reasoning).is_none());
    }

    #[test]
    fn salvage_after_think_whitespace_only_tail_returns_none() {
        let reasoning = "<think>all thoughts</think>   \n\t  ";
        assert!(salvage_after_think(reasoning).is_none());
    }

    #[tokio::test]
    async fn finalize_salvages_reasoning_and_emits_text_delta() {
        let (tx, mut rx) = mpsc::channel(32);
        let mut acc = StreamAccumulator::new();
        // Simulate a MiniMax-style stream: only reasoning_content arrived,
        // and the answer sits after a closing </think> tag.
        acc.reasoning
            .push_str("<think>deliberating...</think>The capital is Paris.");

        let response = acc.finalize(&tx, "minimax-m2".to_string()).await;

        // The salvaged text must be emitted as a TextDelta before Done.
        let first = rx.try_recv().expect("should receive a delta");
        match first {
            StreamDelta::TextDelta { text } => assert_eq!(text, "The capital is Paris."),
            other => panic!("expected TextDelta, got {other:?}"),
        }
        let second = rx.try_recv().expect("should receive Done");
        assert!(matches!(second, StreamDelta::Done { .. }));

        assert_eq!(response.content.as_deref(), Some("The capital is Paris."));
        assert!(response.reasoning_content.is_some());
    }

    #[tokio::test]
    async fn finalize_emits_failure_when_salvage_fails() {
        let (tx, mut rx) = mpsc::channel(32);
        let mut acc = StreamAccumulator::new();
        // Reasoning with no closing </think> tag — salvage must fail.
        acc.reasoning
            .push_str("<think>thinking forever without closing");

        let response = acc.finalize(&tx, "minimax-m2".to_string()).await;

        let first = rx.try_recv().expect("should receive failure");
        match first {
            StreamDelta::Failure(super::super::stream::StreamFailure::EmptyContent {
                reasoning_len,
            }) => {
                assert!(reasoning_len > 0);
            }
            other => panic!("expected Failure::EmptyContent, got {other:?}"),
        }
        let second = rx.try_recv().expect("should receive Done");
        assert!(matches!(second, StreamDelta::Done { .. }));

        assert!(response.content.is_none());
    }

    #[tokio::test]
    async fn finalize_no_reasoning_leaves_stream_unchanged() {
        // Non-reasoning provider: both buffers empty. Salvage must not fire;
        // the stream emits only Done.
        let (tx, mut rx) = mpsc::channel(32);
        let acc = StreamAccumulator::new();

        let response = acc.finalize(&tx, "gpt-4o".to_string()).await;

        let first = rx.try_recv().expect("should receive Done");
        assert!(matches!(first, StreamDelta::Done { .. }));
        assert!(rx.try_recv().is_err(), "no further deltas expected");

        assert!(response.content.is_none());
        assert!(response.reasoning_content.is_none());
    }

    #[tokio::test]
    async fn finalize_with_text_skips_salvage() {
        // Both text and reasoning present: salvage must NOT run, and no
        // synthetic TextDelta or Failure must be emitted.
        let (tx, mut rx) = mpsc::channel(32);
        let mut acc = StreamAccumulator::new();
        acc.text.push_str("already streamed");
        acc.reasoning
            .push_str("<think>ignored</think>trailing ignored");

        let response = acc.finalize(&tx, "minimax-m2".to_string()).await;

        let first = rx.try_recv().expect("should receive Done");
        assert!(matches!(first, StreamDelta::Done { .. }));
        assert!(rx.try_recv().is_err(), "no salvage delta expected");

        assert_eq!(response.content.as_deref(), Some("already streamed"));
    }
}
