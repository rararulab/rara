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

//! Integration tests for the agent OS kernel.
//!
//! These tests require a running Ollama instance at `OLLAMA_BASE_URL`
//! (default: `https://ollama.rara.local/v1`).
//!
//! Run with:
//! ```sh
//! cargo test -p rara-kernel --test integration
//! ```
//!
//! The tests are marked `#[ignore]` by default since they require an
//! external Ollama instance. Remove `#[ignore]` or run with
//! `--ignored` when an instance is available.

use std::sync::Arc;

use async_openai::types::chat::CreateChatCompletionRequestArgs;
use async_trait::async_trait;
use futures::StreamExt;

use rara_kernel::{
    provider::{
        LlmProviderLoader, LlmProviderLoaderRef, OllamaProviderLoader,
        ProviderRegistryBuilder,
    },
    runner::{AgentRunner, RunnerEvent, UserContent},
    testing::TestKernelBuilder,
    tool::{AgentTool, ToolRegistry},
};

/// Default Ollama base URL (OpenAI-compatible API endpoint).
const OLLAMA_BASE_URL: &str = "https://ollama.rara.local/v1";

/// Default model to use for Ollama integration tests.
const OLLAMA_MODEL: &str = "qwen3.5:cloud";

/// Helper: build an OllamaProviderLoader from env or defaults.
fn ollama_loader() -> OllamaProviderLoader {
    let base_url =
        std::env::var("OLLAMA_BASE_URL").unwrap_or_else(|_| OLLAMA_BASE_URL.to_string());
    OllamaProviderLoader::new(base_url)
}

/// Helper: resolve the model name from env or defaults.
fn ollama_model() -> String {
    std::env::var("OLLAMA_MODEL").unwrap_or_else(|_| OLLAMA_MODEL.to_string())
}

/// Simple echo tool for integration testing.
struct EchoTool;

#[async_trait]
impl AgentTool for EchoTool {
    fn name(&self) -> &str { "echo_tool" }

    fn description(&self) -> &str { "Echoes back the input as-is." }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "text": {
                    "type": "string",
                    "description": "The text to echo back"
                }
            },
            "required": ["text"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        Ok(params)
    }
}

// ---------------------------------------------------------------------------
// Test 1: LLM connectivity smoke test
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Ollama instance"]
async fn test_ollama_connectivity() {
    let loader = ollama_loader();
    let provider = loader
        .acquire_provider()
        .await
        .expect("failed to acquire Ollama provider");

    let model = ollama_model();
    let request = CreateChatCompletionRequestArgs::default()
        .model(&model)
        .messages(vec![
            async_openai::types::chat::ChatCompletionRequestSystemMessageArgs::default()
                .content("You are a helpful assistant. Reply concisely.")
                .build()
                .unwrap()
                .into(),
            async_openai::types::chat::ChatCompletionRequestUserMessageArgs::default()
                .content("Say hello in one sentence.")
                .build()
                .unwrap()
                .into(),
        ])
        .temperature(0.3_f32)
        .build()
        .expect("failed to build request");

    let response = provider
        .chat_completion(request)
        .await
        .expect("Ollama chat completion failed");

    let text = response
        .choices
        .first()
        .and_then(|c| c.message.content.as_deref())
        .unwrap_or_default();

    assert!(
        !text.trim().is_empty(),
        "expected non-empty response from Ollama, got empty"
    );
}

// ---------------------------------------------------------------------------
// Test 2: Streaming smoke test
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Ollama instance"]
async fn test_ollama_streaming() {
    let loader = ollama_loader();
    let provider = loader
        .acquire_provider()
        .await
        .expect("failed to acquire Ollama provider");

    let model = ollama_model();
    let request = CreateChatCompletionRequestArgs::default()
        .model(&model)
        .messages(vec![
            async_openai::types::chat::ChatCompletionRequestSystemMessageArgs::default()
                .content("You are a helpful assistant.")
                .build()
                .unwrap()
                .into(),
            async_openai::types::chat::ChatCompletionRequestUserMessageArgs::default()
                .content("Count from 1 to 5, one number per line.")
                .build()
                .unwrap()
                .into(),
        ])
        .temperature(0.3_f32)
        .build()
        .expect("failed to build request");

    let mut stream = provider
        .chat_completion_stream(request)
        .await
        .expect("Ollama streaming request failed");

    let mut chunks = Vec::new();
    let mut accumulated_text = String::new();

    while let Some(result) = stream.next().await {
        let response = result.expect("streaming chunk error");
        if let Some(choice) = response.choices.first() {
            if let Some(ref text) = choice.delta.content {
                accumulated_text.push_str(text);
            }
        }
        chunks.push(response);
    }

    assert!(
        !chunks.is_empty(),
        "expected at least one streaming chunk from Ollama"
    );
    assert!(
        !accumulated_text.trim().is_empty(),
        "expected non-empty accumulated text from streaming"
    );
}

// ---------------------------------------------------------------------------
// Test 3: AgentRunner with Ollama (non-streaming)
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Ollama instance"]
async fn test_agent_runner_with_ollama() {
    let loader = Arc::new(ollama_loader()) as LlmProviderLoaderRef;
    let model = ollama_model();
    let tools = ToolRegistry::new();

    let runner = AgentRunner::builder()
        .llm_provider(loader)
        .model_name(model)
        .system_prompt("You are a concise assistant. Reply in one sentence.")
        .user_content(UserContent::Text(
            "What is 2 + 2?".to_string(),
        ))
        .max_iterations(3_usize)
        .build();

    let result = runner
        .run(&tools, None)
        .await
        .expect("AgentRunner failed with Ollama");

    let text = result.response_text();
    assert!(
        !text.trim().is_empty(),
        "expected non-empty response from AgentRunner"
    );
    assert!(
        !result.truncated,
        "simple question should not truncate"
    );
}

// ---------------------------------------------------------------------------
// Test 4: AgentRunner streaming with Ollama
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Ollama instance"]
async fn test_agent_runner_streaming_with_ollama() {
    let loader = Arc::new(ollama_loader()) as LlmProviderLoaderRef;
    let model = ollama_model();
    let tools = ToolRegistry::new();

    let runner = AgentRunner::builder()
        .llm_provider(loader)
        .model_name(model)
        .system_prompt("You are a concise assistant.")
        .user_content(UserContent::Text(
            "Reply with exactly: Hello, world!".to_string(),
        ))
        .max_iterations(3_usize)
        .build();

    let mut rx = runner.run_streaming(Arc::new(tools));
    let mut got_text = false;
    let mut got_done = false;

    while let Some(event) = rx.recv().await {
        match event {
            RunnerEvent::TextDelta(ref text) if !text.is_empty() => {
                got_text = true;
            }
            RunnerEvent::Done { ref text, .. } => {
                assert!(
                    !text.trim().is_empty(),
                    "Done event should contain non-empty text"
                );
                got_done = true;
            }
            RunnerEvent::Error(err) => {
                panic!("streaming agent runner errored: {err}");
            }
            _ => {}
        }
    }

    assert!(got_text, "expected at least one TextDelta event");
    assert!(got_done, "expected a Done event");
}

// ---------------------------------------------------------------------------
// Test 5: Tool schema generation with Ollama
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires running Ollama instance"]
async fn test_tool_schema_with_ollama() {
    let loader = Arc::new(ollama_loader()) as LlmProviderLoaderRef;
    let model = ollama_model();

    let mut tools = ToolRegistry::new();
    tools.register_builtin(Arc::new(EchoTool));

    // Verify tool schema can be generated.
    let tool_defs = tools
        .to_chat_completion_tools()
        .expect("failed to generate tool schemas");
    assert_eq!(tool_defs.len(), 1, "expected exactly one tool definition");

    // Build and send a request that includes tool definitions.
    let runner = AgentRunner::builder()
        .llm_provider(loader)
        .model_name(model)
        .system_prompt(
            "You are a tool-using assistant. Always call echo_tool exactly once before replying.",
        )
        .user_content(UserContent::Text(
            "Call echo_tool with {\"text\":\"integration-test\"} and then reply.".to_string(),
        ))
        .max_iterations(5_usize)
        .build();

    let result = runner
        .run(&tools, None)
        .await
        .expect("AgentRunner with tools failed");

    // The model should have made at least one tool call.
    assert!(
        result.tool_calls_made > 0,
        "expected at least one tool call, got {}",
        result.tool_calls_made
    );

    let text = result.response_text();
    assert!(
        !text.trim().is_empty(),
        "expected non-empty final response after tool call"
    );
}

// ---------------------------------------------------------------------------
// Test 6: TestKernelBuilder smoke test (no Ollama needed)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_kernel_builder_creates_kernel() {
    // Use a stub provider loader for this test — no real LLM needed.
    use async_openai::types::chat::{
        ChatCompletionResponseStream, CreateChatCompletionRequest,
        CreateChatCompletionResponse,
    };

    struct StubProvider;

    #[async_trait]
    impl rara_kernel::provider::LlmProvider for StubProvider {
        async fn chat_completion(
            &self,
            _request: CreateChatCompletionRequest,
        ) -> rara_kernel::Result<CreateChatCompletionResponse> {
            Err(rara_kernel::KernelError::Other {
                message: "stub".into(),
            })
        }

        async fn chat_completion_stream(
            &self,
            _request: CreateChatCompletionRequest,
        ) -> rara_kernel::Result<ChatCompletionResponseStream> {
            Err(rara_kernel::KernelError::Other {
                message: "stub".into(),
            })
        }
    }

    let registry = Arc::new(
        ProviderRegistryBuilder::new("test", "test-model")
            .provider("test", Arc::new(StubProvider) as Arc<dyn rara_kernel::provider::LlmProvider>)
            .build(),
    );

    let kernel = TestKernelBuilder::new()
        .provider_registry(registry)
        .tool(Arc::new(EchoTool))
        .max_concurrency(4)
        .max_iterations(10)
        .build();

    assert_eq!(kernel.config().max_concurrency, 4);
    assert_eq!(kernel.config().default_max_iterations, 10);
    assert_eq!(kernel.tool_registry().len(), 1);
    assert!(kernel.agent_registry().get("scout").is_some());
}
