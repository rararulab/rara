# OpenAI Driver Cleanup Design

**Date**: 2026-03-03
**Scope**: `crates/kernel/src/llm/openai.rs`

## Problem

Current implementation has four cleanliness issues:

1. `json!()` macros for request body — no compile-time structure checking
2. Manual SSE parsing with `buffer.find('\n')` — fragile, ~20 lines boilerplate
3. `process_stream_chunk` takes 7 params (5 `&mut`) — state scattered at call site
4. `complete` and `stream` duplicate ~15 lines of HTTP request/error handling

## Design

### 1. Typed Wire Types (`#[derive(Serialize)]`)

Add private wire types for OpenAI HTTP serialization:

- `ChatRequest<'a>` — borrows from `CompletionRequest`, zero-copy serialize
- `WireMessage<'a>` — serializes `Message` to OpenAI format
- `WireTool<'a>` — serializes `ToolDefinition`
- `WireToolCall<'a>` — serializes `ToolCallRequest` in message
- `StreamOptions` — `{ include_usage: true }`

`build_request_body` becomes `ChatRequest::from_completion(&request, stream)` returning a typed struct.

`tool_choice` and `thinking` remain `serde_json::Value` due to irregular OpenAI shapes.

### 2. SSE Parsing with `eventsource-stream`

Add dependency:

```toml
# workspace Cargo.toml
eventsource-stream = "0.2"

# crates/kernel/Cargo.toml
eventsource-stream.workspace = true
```

Replace manual buffer + line splitting with:

```rust
let event_stream = response.bytes_stream().eventsource();
tokio::pin!(event_stream);
while let Some(event) = event_stream.next().await { ... }
```

### 3. `StreamAccumulator` Struct

Extract streaming state into a struct:

```rust
struct StreamAccumulator {
    text:        String,
    reasoning:   String,
    tools:       HashMap<u32, PendingToolCall>,
    stop_reason: StopReason,
    usage:       Option<Usage>,
}
```

Methods:
- `new()` — initialize
- `process_chunk(&mut self, chunk, tx)` — handle one SSE chunk
- `finalize(self, tx, model) -> CompletionResponse` — send Done + build response
- `collect_tools(self) -> Vec<ToolCallRequest>` — private helper

### 4. Shared `send_request` Method

Extract common HTTP logic into `OpenAiDriver::send_request`:

```rust
async fn send_request(&self, request: &CompletionRequest, stream: bool) -> Result<reqwest::Response>
```

Handles: resolve config, build body, POST, status check, error mapping.

`complete` and `stream` each call `send_request` then diverge on response parsing.

## Files Changed

- `crates/kernel/src/llm/openai.rs` — full rewrite of internals, public API unchanged
- `crates/kernel/Cargo.toml` — add `eventsource-stream`
- `Cargo.toml` (workspace) — add `eventsource-stream` to workspace deps

## Test Impact

All existing tests continue to pass — they test `build_request_body` (now `ChatRequest` serialization), SSE deserialization types, and `message_to_json` (now `WireMessage` serialization). Test assertions remain the same; only internal function signatures change.
