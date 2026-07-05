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

//! In-process OpenAI-compatible fake provider (wiremock, streaming SSE).
//!
//! Follows the `openai/codex` `codex-rs/core/tests/common/responses.rs`
//! pattern: canned SSE chat-completion bodies served over a real socket via
//! `ResponseTemplate::new(200).set_body_raw(body, "text/event-stream")`, with
//! post-hoc request assertions driven by `MockServer::received_requests()`.
//!
//! This exercises the REAL openai driver stack — HTTP client + auth + URL
//! building (`crates/kernel/src/llm/openai.rs`) and SSE parsing / accumulation
//! (`stream_chat_completions` + `StreamAccumulator`) — which trait-level DI
//! (`ScriptedLlmDriver`) deliberately skips. Decision chain:
//! #1930 → #1941 → #2016 → #2178 → #2190.

use serde_json::{Value, json};
use wiremock::{
    Mock, MockServer, Request, ResponseTemplate,
    matchers::{body_partial_json, body_string_contains, method, path},
};

/// Request path the real openai driver builds (`openai.rs`:
/// `format!("{}{}", base_url, "/chat/completions")`). Single definition so a
/// deliberate shape-break (red/green proof) only needs one edit.
pub const CHAT_COMPLETIONS_PATH: &str = "/chat/completions";

/// A streamed text delta chunk (Chat Completions SSE shape).
pub fn text_chunk(text: &str) -> Value {
    json!({"choices": [{"index": 0, "delta": {"content": text}}]})
}

/// A streamed reasoning delta chunk (`delta.reasoning_content`).
pub fn reasoning_chunk(text: &str) -> Value {
    json!({"choices": [{"index": 0, "delta": {"reasoning_content": text}}]})
}

/// First chunk of a streamed tool call: carries id + name (index 0).
pub fn tool_call_start_chunk(id: &str, name: &str) -> Value {
    json!({"choices": [{"index": 0, "delta": {"tool_calls": [
        {"index": 0, "id": id, "function": {"name": name, "arguments": ""}}
    ]}}]})
}

/// Follow-up chunk carrying a fragment of the tool call's JSON arguments.
pub fn tool_call_args_chunk(fragment: &str) -> Value {
    json!({"choices": [{"index": 0, "delta": {"tool_calls": [
        {"index": 0, "function": {"arguments": fragment}}
    ]}}]})
}

/// Final chunk with a `finish_reason` and usage block.
pub fn finish_chunk(reason: &str) -> Value {
    json!({
        "choices": [{"index": 0, "delta": {}, "finish_reason": reason}],
        "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
    })
}

/// Assemble chunks into an SSE body (`data: {...}` events + `data: [DONE]`).
fn sse_body(chunks: &[Value]) -> String {
    let mut body = String::new();
    for chunk in chunks {
        body.push_str("data: ");
        body.push_str(&chunk.to_string());
        body.push_str("\n\n");
    }
    body.push_str("data: [DONE]\n\n");
    body
}

/// A 200 `text/event-stream` response streaming the given chunks.
pub fn sse_response(chunks: &[Value]) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_raw(sse_body(chunks), "text/event-stream")
}

/// Mount a scripted SSE response for `POST /chat/completions` requests whose
/// body contains `marker`.
///
/// Later conversation turns embed earlier turns' text in their request
/// history, so markers overlap across turns — disambiguate by giving the
/// LATEST turn the numerically lowest (= highest) `priority`.
pub async fn mount_chat_sse(server: &MockServer, marker: &str, priority: u8, chunks: &[Value]) {
    Mock::given(method("POST"))
        .and(path(CHAT_COMPLETIONS_PATH))
        .and(body_string_contains(marker))
        .respond_with(sse_response(chunks))
        .with_priority(priority)
        .mount(server)
        .await;
}

/// Catch-all mounts for LLM traffic this test does not script per-turn:
/// background agents (title_gen, knowledge_extractor) and the knowledge
/// layer's embedder. Mounted at the lowest priority so scripted turn mocks
/// always win.
pub async fn mount_fallbacks(server: &MockServer) {
    // Streaming chat fallback.
    Mock::given(method("POST"))
        .and(path(CHAT_COMPLETIONS_PATH))
        .and(body_partial_json(json!({"stream": true})))
        .respond_with(sse_response(&[
            text_chunk("mock background reply"),
            finish_chunk("stop"),
        ]))
        .with_priority(250)
        .mount(server)
        .await;

    // Non-streaming chat fallback (`LlmDriver::complete`).
    Mock::given(method("POST"))
        .and(path(CHAT_COMPLETIONS_PATH))
        .and(body_partial_json(json!({"stream": false})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "cmpl-mock",
            "object": "chat.completion",
            "created": 0,
            "model": "mock-model",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "mock background reply"},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 1}
        })))
        .with_priority(250)
        .mount(server)
        .await;

    // Embeddings fallback: one zero-vector per input, dimensions matching the
    // test config's `knowledge.embedding_dimensions`.
    Mock::given(method("POST"))
        .and(path("/embeddings"))
        .respond_with(embeddings_response)
        .with_priority(250)
        .mount(server)
        .await;
}

/// Dimensions served by the embeddings fallback — keep in sync with the
/// `knowledge.embedding_dimensions` value in the test-authored config.
pub const EMBEDDING_DIMENSIONS: usize = 8;

/// Dynamic responder: echoes one zero-embedding per request input so batch
/// sizes always line up.
fn embeddings_response(request: &Request) -> ResponseTemplate {
    let input_len = serde_json::from_slice::<Value>(&request.body)
        .ok()
        .and_then(|body| body.get("input").and_then(|i| i.as_array()).map(Vec::len))
        .unwrap_or(1);
    let data: Vec<Value> = (0..input_len)
        .map(|index| {
            json!({
                "object": "embedding",
                "index": index,
                "embedding": vec![0.0f32; EMBEDDING_DIMENSIONS]
            })
        })
        .collect();
    ResponseTemplate::new(200).set_body_json(json!({
        "object": "list",
        "data": data,
        "model": "mock-embedding",
        "usage": {"prompt_tokens": 1, "total_tokens": 1}
    }))
}

/// All captured `POST /chat/completions` request bodies, parsed as JSON, in
/// arrival order.
pub async fn chat_request_bodies(server: &MockServer) -> Vec<Value> {
    server
        .received_requests()
        .await
        .expect("request recording should be enabled")
        .iter()
        .filter(|r| r.method.as_str() == "POST" && r.url.path() == CHAT_COMPLETIONS_PATH)
        .map(|r| serde_json::from_slice(&r.body).expect("chat request body should be JSON"))
        .collect()
}

/// Index of the `role: "user"` message whose content EQUALS `text`.
///
/// Equality (not `contains`) keeps the lookup unique: background agents
/// (title_gen, knowledge_extractor) embed conversation text inside larger
/// prompts, and context assembly appends system-reminder user messages after
/// the real one — neither is byte-equal to the original turn input.
pub fn user_message_index(body: &Value, text: &str) -> Option<usize> {
    body.get("messages")?.as_array()?.iter().position(|m| {
        m.get("role").and_then(Value::as_str) == Some("user")
            && m.get("content").and_then(Value::as_str) == Some(text)
    })
}
