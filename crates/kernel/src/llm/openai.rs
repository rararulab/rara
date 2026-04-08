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
use tokio::sync::mpsc;

use super::{
    driver::{LlmCredentialResolverRef, LlmDriver, LlmEmbedder, LlmModelLister},
    stream::StreamDelta,
    types::{
        CompletionRequest, CompletionResponse, ContentBlock, EmbeddingRequest, EmbeddingResponse,
        Message, MessageContent, ModelInfo, Role, StopReason, ToolCallRequest, ToolChoice, Usage,
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
pub struct OpenAiDriver {
    /// Client for non-streaming requests (with total timeout).
    client:           reqwest::Client,
    /// Client for streaming requests (no total timeout — SSE idle timeout
    /// handles stall detection instead).
    stream_client:    reqwest::Client,
    config_source:    OpenAiDriverConfigSource,
    /// Per-event idle timeout for SSE streaming. Defaults to
    /// [`Self::DEFAULT_SSE_IDLE_TIMEOUT`].
    sse_idle_timeout: Duration,
    /// Lazily populated cache of model metadata from the provider's
    /// `/models` endpoint.  Initialised at most once via
    /// [`tokio::sync::OnceCell`] to avoid duplicate fetches under
    /// concurrent access.
    models_cache:     tokio::sync::OnceCell<HashMap<String, ModelMeta>>,
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
    base_url: String,
    api_key:  String,
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
const RATE_LIMIT_MAX_DELAY: Duration = Duration::from_secs(60);

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
            .timeout(Duration::from_secs(300));
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
        }
    }

    async fn resolve_config(&self) -> Result<ResolvedConfig> {
        match &self.config_source {
            OpenAiDriverConfigSource::Static { base_url, api_key } => Ok(ResolvedConfig {
                base_url: base_url.clone(),
                api_key:  api_key.clone(),
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

                Ok(ResolvedConfig { base_url, api_key })
            }
            OpenAiDriverConfigSource::Dynamic { resolver } => {
                let cred = resolver.resolve().await?;
                Ok(ResolvedConfig {
                    base_url: cred.base_url,
                    api_key:  cred.api_key,
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
        let body = ChatRequest::from_completion(request, stream);

        tracing::debug!(
            model = request.model.as_str(),
            messages = request.messages.len(),
            tools = request.tools.len(),
            stream,
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
            let response = http
                .post(format!("{}/chat/completions", config.base_url))
                .bearer_auth(&config.api_key)
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
// LlmDriver implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl LlmDriver for OpenAiDriver {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse> {
        let response = self.send_request(&request, false).await?;

        let raw: RawCompletionResponse =
            response.json().await.map_err(|e| KernelError::Provider {
                message: format!("failed to parse LLM response: {e}").into(),
            })?;

        let choice = raw
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| KernelError::Provider {
                message: "no choices in response".into(),
            })?;

        let stop_reason = parse_stop_reason(choice.finish_reason.as_deref());

        let tool_calls = choice
            .message
            .tool_calls
            .unwrap_or_default()
            .into_iter()
            .map(|tc| ToolCallRequest {
                id:        tc.id,
                name:      tc.function.name,
                arguments: tc.function.arguments,
            })
            .collect();

        let usage = raw.usage.map(|u| Usage {
            prompt_tokens:     u.input.unwrap_or(0),
            completion_tokens: u.output.unwrap_or(0),
            total_tokens:      u.total.unwrap_or(0),
        });

        // Strip embedded <think> tags from content for providers that place
        // reasoning there instead of `reasoning_content`.
        let (embedded_thinking, cleaned_content) = choice
            .message
            .content
            .as_deref()
            .map(super::think_tag::strip_think_tags)
            .map(|(thinking, content)| (thinking, non_empty(content)))
            .unwrap_or((None, None));

        // Prefer explicit reasoning_content when present, otherwise use
        // extracted `<think>` content.
        let reasoning_content = choice.message.reasoning_content.or(embedded_thinking);

        Ok(CompletionResponse {
            content: cleaned_content,
            reasoning_content,
            tool_calls,
            stop_reason,
            usage,
            model: raw.model.unwrap_or_else(|| request.model.clone()),
        })
    }

    async fn stream(
        &self,
        request: CompletionRequest,
        tx: mpsc::Sender<StreamDelta>,
    ) -> Result<CompletionResponse> {
        let response = self.send_request(&request, true).await?;

        let mut event_stream = response.bytes_stream().eventsource();
        let mut acc = StreamAccumulator::new();

        loop {
            // Break early if the receiver has been dropped (e.g. user cancelled)
            if tx.is_closed() {
                tracing::debug!("stream consumer disconnected, returning partial response");
                break;
            }

            let maybe_event =
                tokio::time::timeout(self.sse_idle_timeout, event_stream.next()).await;

            match maybe_event {
                Ok(Some(event_result)) => {
                    let event = event_result.map_err(|e| KernelError::Provider {
                        message: format!(
                            "SSE stream error: {}",
                            crate::error::format_error_chain(&e)
                        )
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
                        timeout_secs = self.sse_idle_timeout.as_secs(),
                        "SSE stream idle timeout — no event received, aborting stream"
                    );
                    return Err(KernelError::RetryableServer {
                        message: "SSE stream idle timeout".into(),
                    });
                }
            }
        }

        Ok(acc.finalize(&tx, request.model.clone()).await)
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
    text:         String,
    reasoning:    String,
    think_parser: super::think_tag::ThinkTagParser,
    tools:        HashMap<u32, PendingToolCall>,
    stop_reason:  StopReason,
    usage:        Option<Usage>,
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
            text:         String::new(),
            reasoning:    String::new(),
            think_parser: super::think_tag::ThinkTagParser::new(),
            tools:        HashMap::new(),
            stop_reason:  StopReason::Stop,
            usage:        None,
        }
    }

    async fn process_chunk(&mut self, chunk: &RawStreamChunk, tx: &mpsc::Sender<StreamDelta>) {
        for choice in &chunk.choices {
            // Text delta (split out embedded <think> blocks).
            if let Some(ref text) = choice.delta.content {
                if !text.is_empty() {
                    for segment in self.think_parser.push(text) {
                        match segment {
                            super::think_tag::Segment::Text(t) => {
                                self.text.push_str(&t);
                                let _ = tx.send(StreamDelta::TextDelta { text: t }).await;
                            }
                            super::think_tag::Segment::Thinking(t) => {
                                self.reasoning.push_str(&t);
                                let _ = tx.send(StreamDelta::ReasoningDelta { text: t }).await;
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
                                entry.name = name.clone();
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

    async fn finalize(
        mut self,
        tx: &mpsc::Sender<StreamDelta>,
        model: String,
    ) -> CompletionResponse {
        // Flush trailing partial content that was buffered for tag boundary
        // detection.
        for segment in self.think_parser.flush() {
            match segment {
                super::think_tag::Segment::Text(t) => {
                    self.text.push_str(&t);
                    let _ = tx.send(StreamDelta::TextDelta { text: t }).await;
                }
                super::think_tag::Segment::Thinking(t) => {
                    self.reasoning.push_str(&t);
                    let _ = tx.send(StreamDelta::ReasoningDelta { text: t }).await;
                }
            }
        }

        let Self {
            text,
            reasoning,
            tools,
            stop_reason,
            usage,
            ..
        } = self;
        let tool_calls = Self::collect_tools(tools);

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
    role:         &'static str,
    /// Per OpenAI spec, assistant messages with tool_calls may have `content:
    /// null`.
    content:      Option<WireContent<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls:   Option<Vec<WireToolCallRef<'a>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<&'a str>,
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
        let messages: Vec<WireMessage<'a>> = request
            .messages
            .iter()
            .map(WireMessage::from_message)
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
        }
    }
}

impl<'a> WireMessage<'a> {
    fn from_message(msg: &'a Message) -> Self {
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
}
