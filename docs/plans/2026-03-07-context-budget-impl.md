# Context Budget Management Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Enable agent self-managed context budget via usage tracking, handoff tools, and image compression.

**Architecture:** Three independent tracks — (A) usage collection + tape tools, (B) image compression pipeline, (C) ContextWindow error UX. Track A and B can be parallelized; C is independent.

**Tech Stack:** Rust, `image` crate (resize/jpeg), `base64` crate, existing tape/tool infrastructure.

**Design doc:** `docs/plans/2026-03-07-context-budget-management.md`

---

## Task 1: Usage Collection + Tape Tools + Context Contract

**Files:**
- Modify: `crates/kernel/src/agent.rs:695` (StreamDelta::Done)
- Modify: `crates/kernel/src/memory/service.rs:43-57` (TapeInfo), `service.rs:331-373` (info())
- Create: `crates/app/src/tools/tape_info.rs`
- Create: `crates/app/src/tools/tape_handoff.rs`
- Modify: `crates/app/src/tools/mod.rs` (register new tools)
- Modify: `crates/kernel/src/agent.rs:532-535` (system prompt)

### Step 1: Collect usage from StreamDelta::Done

Modify `crates/kernel/src/agent.rs:695`. Change:

```rust
llm::StreamDelta::Done { stop_reason, .. } => {
    has_tool_calls = stop_reason == llm::StopReason::ToolCalls;
    break;
}
```

To:

```rust
llm::StreamDelta::Done { stop_reason, usage } => {
    has_tool_calls = stop_reason == llm::StopReason::ToolCalls;
    if let Some(u) = usage {
        if let Err(e) = tape.append_event(
            tape_name,
            "llm.run",
            serde_json::json!({
                "usage": {
                    "prompt_tokens": u.prompt_tokens,
                    "completion_tokens": u.completion_tokens,
                    "total_tokens": u.total_tokens
                }
            }),
        ).await {
            warn!(error = %e, "failed to persist llm usage event");
        }
    }
    break;
}
```

Note: `tape` is the `TapeService` parameter (line 481), `tape_name` is `&str` (line 482). `append_event` takes `(tape_name, event_name, data)` — see `service.rs:166-176`.

### Step 2: Verify it compiles

Run: `cargo check -p rara-kernel`
Expected: PASS (no new types, just using existing `append_event`)

### Step 3: Update TapeInfo to read usage from llm.run events

The `TapeInfo` struct at `service.rs:43-57` already has `last_token_usage: Option<u64>` and the `info()` method at `service.rs:331-373` already reads from `"run"` events. Update `info()` to also match `"llm.run"` event name.

In `service.rs` `info()` method, find this block (around line 351):

```rust
let last_token_usage = entries.iter().rev().find_map(|entry| {
    if entry.kind != TapEntryKind::Event
        || entry.payload.get("name") != Some(&Value::String("run".to_owned()))
    {
        return None;
    }
```

Change the condition to match both `"run"` and `"llm.run"`:

```rust
let last_token_usage = entries.iter().rev().find_map(|entry| {
    if entry.kind != TapEntryKind::Event {
        return None;
    }
    let event_name = entry.payload.get("name").and_then(Value::as_str);
    if !matches!(event_name, Some("run" | "llm.run")) {
        return None;
    }
```

### Step 4: Verify it compiles

Run: `cargo check -p rara-kernel`
Expected: PASS

### Step 5: Create tape_info tool

Create `crates/app/src/tools/tape_info.rs`:

```rust
use async_trait::async_trait;
use rara_kernel::{memory::TapeService, tool::AgentTool};
use serde_json::json;

pub struct TapeInfoTool {
    tape_service: TapeService,
}

impl TapeInfoTool {
    pub fn new(tape_service: TapeService) -> Self { Self { tape_service } }
}

#[async_trait]
impl AgentTool for TapeInfoTool {
    fn name(&self) -> &str { "tape_info" }

    fn description(&self) -> &str {
        "Show tape summary with entry counts, anchor info, and last token usage. \
         Use this to check how much context has been consumed."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {},
            "required": []
        })
    }

    async fn execute(
        &self,
        _params: serde_json::Value,
        context: &rara_kernel::tool::ToolContext,
    ) -> anyhow::Result<serde_json::Value> {
        let tape_name = context
            .tape_name
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("no tape in session context"))?;

        let info = self
            .tape_service
            .info(tape_name)
            .await
            .map_err(|e| anyhow::anyhow!("failed to read tape info: {e}"))?;

        let usage_str = info
            .last_token_usage
            .map(|u| u.to_string())
            .unwrap_or_else(|| "unknown".to_string());

        Ok(json!({
            "output": format!(
                "tape={}\nentries={}\nanchors={}\nlast_anchor={}\nentries_since_last_anchor={}\nlast_token_usage={}",
                info.name,
                info.entries,
                info.anchors,
                info.last_anchor.as_deref().unwrap_or("-"),
                info.entries_since_last_anchor,
                usage_str,
            )
        }))
    }
}
```

Note: Check that `ToolContext` has a `tape_name` field. If not, use `session_id` to derive the tape name, or use the same pattern as `user_note.rs` which accesses `context.user_id`. The tape name may need to be derived from session context — check `ToolContext` definition in `crates/kernel/src/tool.rs`.

### Step 6: Create tape_handoff tool

Create `crates/app/src/tools/tape_handoff.rs`:

```rust
use async_trait::async_trait;
use rara_kernel::{
    memory::{HandoffState, TapeService},
    tool::AgentTool,
};
use serde_json::json;

pub struct TapeHandoffTool {
    tape_service: TapeService,
}

impl TapeHandoffTool {
    pub fn new(tape_service: TapeService) -> Self { Self { tape_service } }
}

#[async_trait]
impl AgentTool for TapeHandoffTool {
    fn name(&self) -> &str { "tape_handoff" }

    fn description(&self) -> &str {
        "Create a tape anchor handoff to shorten context history. Use this when \
         context is getting too long. Provide a summary of what happened so far \
         and next steps so context can be truncated without losing important info."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Anchor name (default: 'handoff')"
                },
                "summary": {
                    "type": "string",
                    "description": "Summary of what happened before this point"
                },
                "next_steps": {
                    "type": "string",
                    "description": "What should happen next"
                }
            },
            "required": []
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        context: &rara_kernel::tool::ToolContext,
    ) -> anyhow::Result<serde_json::Value> {
        let tape_name = context
            .tape_name
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("no tape in session context"))?;

        let anchor_name = params
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("handoff");

        let summary = params.get("summary").and_then(|v| v.as_str()).map(str::to_owned);
        let next_steps = params.get("next_steps").and_then(|v| v.as_str()).map(str::to_owned);

        let state = HandoffState {
            summary,
            next_steps,
            owner: Some("agent".to_owned()),
            ..Default::default()
        };

        self.tape_service
            .handoff(tape_name, anchor_name, state)
            .await
            .map_err(|e| anyhow::anyhow!("handoff failed: {e}"))?;

        Ok(json!({
            "output": format!("handoff created: {anchor_name}")
        }))
    }
}
```

Note: Check `HandoffState` struct definition in `crates/kernel/src/memory/anchors.rs` for exact field names. It may use builder pattern or different field names. Also check if `HandoffState` derives `Default`.

### Step 7: Register new tools

Modify `crates/app/src/tools/mod.rs`:

Add module declarations after line 40:

```rust
mod tape_handoff;
mod tape_info;
```

Add use statements:

```rust
use tape_handoff::TapeHandoffTool;
use tape_info::TapeInfoTool;
```

In `register_all()`, add to the `tools` vec (before `dispatch_rara`):

```rust
        // Tape management tools
        Arc::new(TapeInfoTool::new(deps.tape_service.clone())),
        Arc::new(TapeHandoffTool::new(deps.tape_service.clone())),
```

### Step 8: Add context_contract to system prompt

In `crates/kernel/src/agent.rs`, after the system prompt assembly (around line 532-535), append the context contract:

```rust
let effective_prompt = match &manifest.soul_prompt {
    Some(soul) => format!("{soul}\n\n---\n\n{}", manifest.system_prompt),
    None => manifest.system_prompt.clone(),
};
let effective_prompt = format!(
    "{effective_prompt}\n\n\
     <context_contract>\n\
     Excessively long context may cause model call failures. \
     In this case, you SHOULD first use tape_handoff tool to \
     shorten the length of the retrieved history.\n\
     </context_contract>"
);
```

### Step 9: Verify everything compiles

Run: `cargo check -p rara-kernel -p rara-app`
Expected: PASS

### Step 10: Commit

```bash
git add crates/kernel/src/agent.rs \
        crates/kernel/src/memory/service.rs \
        crates/app/src/tools/tape_info.rs \
        crates/app/src/tools/tape_handoff.rs \
        crates/app/src/tools/mod.rs
git commit -m "feat(kernel): usage collection, tape tools, and context contract

- Collect LLM usage from StreamDelta::Done into tape events
- Add tape_info tool for agent to check token usage
- Add tape_handoff tool for agent-driven context truncation
- Add <context_contract> to system prompt

Closes #N"
```

---

## Task 2: Image Compression Pipeline

**Files:**
- Modify: `crates/kernel/src/llm/types.rs:45-48` (ContentBlock)
- Modify: `crates/kernel/src/llm/openai.rs:493-503` (WireContentPart)
- Create: `crates/kernel/src/llm/image.rs`
- Modify: `crates/kernel/src/llm/mod.rs` (add module)
- Modify: `crates/kernel/Cargo.toml` (add `image` dep)
- Modify: `crates/channels/src/telegram/adapter.rs:1471-1476` (photo handling)
- Modify: `crates/channels/Cargo.toml` (add `image` dep if not already present)

### Step 1: Add ImageBase64 variant to ContentBlock

Modify `crates/kernel/src/llm/types.rs:45-48`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text { text: String },
    ImageUrl { url: String },
    ImageBase64 { media_type: String, data: String },
}
```

Also add a constructor on `Message` (near the existing `user_multimodal`):

```rust
impl ContentBlock {
    pub fn image_base64(media_type: impl Into<String>, data: impl Into<String>) -> Self {
        Self::ImageBase64 {
            media_type: media_type.into(),
            data: data.into(),
        }
    }
}
```

### Step 2: Update WireContentPart serialization

Modify `crates/kernel/src/llm/openai.rs`. The `WireContentPart` enum stays the same (only `Text` and `ImageUrl` variants), but update the `from_message` match to map `ImageBase64` to a data URI:

In the `from_message` function (around line 621-627), change:

```rust
.map(|b| match b {
    ContentBlock::Text { text } => WireContentPart::Text { text },
    ContentBlock::ImageUrl { url } => WireContentPart::ImageUrl {
        image_url: WireImageUrl { url },
    },
})
```

To:

```rust
.map(|b| match b {
    ContentBlock::Text { text } => WireContentPart::Text { text },
    ContentBlock::ImageUrl { url } => WireContentPart::ImageUrl {
        image_url: WireImageUrl { url },
    },
    ContentBlock::ImageBase64 { media_type, data } => {
        // OpenAI/Claude both support data URIs in image_url field
        let data_uri = format!("data:{media_type};base64,{data}");
        WireContentPart::ImageUrl {
            image_url: WireImageUrl { url: &data_uri },
        }
    }
})
```

**Lifetime issue:** `WireImageUrl<'a>` borrows `&'a str`, but `data_uri` is a local `String`. Two options:
- Option A: Change `WireImageUrl` to own a `Cow<'a, str>` instead of `&'a str`
- Option B: Change `WireContent` to store owned strings for the image case

Choose Option A — modify `WireImageUrl`:

```rust
#[derive(Serialize)]
struct WireImageUrl<'a> {
    url: std::borrow::Cow<'a, str>,
}
```

Then update existing `ImageUrl` match arm:

```rust
ContentBlock::ImageUrl { url } => WireContentPart::ImageUrl {
    image_url: WireImageUrl { url: std::borrow::Cow::Borrowed(url) },
},
ContentBlock::ImageBase64 { media_type, data } => {
    let data_uri = format!("data:{media_type};base64,{data}");
    WireContentPart::ImageUrl {
        image_url: WireImageUrl { url: std::borrow::Cow::Owned(data_uri) },
    }
}
```

### Step 3: Verify kernel compiles

Run: `cargo check -p rara-kernel`
Expected: PASS

### Step 4: Add `image` crate dependency

Modify `crates/kernel/Cargo.toml`, add under `[dependencies]`:

```toml
image = { version = "0.25", default-features = false, features = ["jpeg", "png", "webp", "gif"] }
```

Also check workspace `Cargo.toml` — if `image` is already a workspace dep, use `image = { workspace = true }` instead.

### Step 5: Create image compression module

Create `crates/kernel/src/llm/image.rs`:

```rust
//! Image compression utilities for LLM vision input.
//!
//! Resizes images to fit within a maximum edge length and converts to JPEG
//! to minimize token consumption.

use image::ImageReader;
use std::io::Cursor;

/// Default maximum edge length in pixels (Anthropic recommendation).
pub const DEFAULT_MAX_EDGE: u32 = 1568;
/// Default JPEG quality (0-100).
pub const DEFAULT_QUALITY: u8 = 85;

/// Compress an image: resize so neither edge exceeds `max_edge`, then encode as JPEG.
///
/// Returns `(jpeg_bytes, "image/jpeg")`.
pub fn compress_image(
    bytes: &[u8],
    max_edge: u32,
    quality: u8,
) -> anyhow::Result<(Vec<u8>, String)> {
    let img = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()?
        .decode()?;

    let (w, h) = (img.width(), img.height());
    let img = if w > max_edge || h > max_edge {
        let ratio = max_edge as f64 / w.max(h) as f64;
        let new_w = (w as f64 * ratio).round() as u32;
        let new_h = (h as f64 * ratio).round() as u32;
        img.resize_exact(new_w, new_h, image::imageops::FilterType::Lanczos3)
    } else {
        img
    };

    let mut buf = Vec::new();
    let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, quality);
    img.write_with_encoder(encoder)?;

    Ok((buf, "image/jpeg".to_owned()))
}
```

Add module to `crates/kernel/src/llm/mod.rs`:

```rust
pub mod image;
```

### Step 6: Verify kernel compiles

Run: `cargo check -p rara-kernel`
Expected: PASS

### Step 7: Update Telegram adapter to compress incoming photos

Modify `crates/channels/src/telegram/adapter.rs`. The current `telegram_to_raw_platform_message` function (line 1471) only extracts text/caption from photos. We need to:

1. Detect photos on the message
2. Download the photo file from Telegram
3. Compress it
4. Attach as `ContentBlock::ImageBase64` to the message

Find the function `telegram_to_raw_platform_message` and update it to handle photos. This requires access to the `Bot` instance to download files.

The exact implementation depends on how `RawPlatformMessage` carries multimodal content. Check `RawPlatformMessage` definition first — if it only has a `text: String` field, it may need an `attachments` or `images` field added.

Key changes needed:
1. Check if `msg.photo()` returns `Some(photos)` (Telegram sends multiple sizes)
2. Pick the largest photo size: `photos.last()` (Telegram sorts by size ascending)
3. Use `bot.get_file(file_id).await` to get the file path
4. Download the file bytes
5. Call `rara_kernel::llm::image::compress_image(bytes, DEFAULT_MAX_EDGE, DEFAULT_QUALITY)`
6. Base64-encode the result
7. Construct `ContentBlock::ImageBase64 { media_type, data }`
8. Attach to the platform message so it flows through to the LLM

This step requires checking `RawPlatformMessage` and the inbound message flow from channel → kernel to determine exactly where to inject the image content blocks. The changes may span:
- `RawPlatformMessage` struct (add `images: Vec<ContentBlock>`)
- `telegram_to_raw_platform_message` (populate images)
- Kernel message construction (include images in LLM messages)

Add `image` and `base64` to `crates/channels/Cargo.toml` if needed (`base64` is already present).

### Step 8: Verify channels compile

Run: `cargo check -p rara-channels`
Expected: PASS

### Step 9: Verify full build

Run: `cargo check`
Expected: PASS

### Step 10: Commit

```bash
git add crates/kernel/src/llm/types.rs \
        crates/kernel/src/llm/openai.rs \
        crates/kernel/src/llm/image.rs \
        crates/kernel/src/llm/mod.rs \
        crates/kernel/Cargo.toml \
        crates/channels/src/telegram/adapter.rs \
        crates/channels/Cargo.toml
git commit -m "feat(llm): image compression pipeline for vision input

- Add ImageBase64 variant to ContentBlock
- Wire serialization supports base64 data URIs
- Add compress_image() utility (resize + jpeg)
- Telegram adapter compresses incoming photos

Closes #N"
```

---

## Task 3: ContextWindow Error Notification

**Files:**
- Modify: `crates/kernel/src/kernel.rs:1735-1758` (handle_turn_completed error branch)

### Step 1: Improve ContextWindow error message

In `crates/kernel/src/kernel.rs`, the `handle_turn_completed` method already sends errors to users via `OutboundEnvelope::error` at line 1745-1758.

The error message comes as a raw string from `KernelError::ContextWindow.to_string()` which is `"context window exceeded"`.

We want to provide a more helpful message. Modify the `Err(err_msg)` branch (line 1735):

```rust
Err(err_msg) => {
    span.record("success", false);
    _turn_failed = err_msg != "interrupted by user";
    if _turn_failed {
        error!(session_key = %session_key, error = %err_msg, "turn failed");
    } else {
        info!(session_key = %session_key, "turn interrupted by user");
    }

    // Provide user-friendly message for context window errors.
    let user_msg = if err_msg.contains("context window") {
        "上下文已超出模型限制，本轮对话未完成。请发送 /handoff 或开始新对话。".to_string()
    } else {
        err_msg.clone()
    };

    let envelope = OutboundEnvelope::error(
        in_reply_to,
        user.clone(),
        egress_session_key.clone(),
        "agent_error",
        user_msg,
    )
    .with_origin(origin_endpoint.clone());
    if let Err(e) = &self
        .event_queue
        .try_push(KernelEventEnvelope::deliver(envelope))
    {
        error!(%e, "failed to push error Deliver event");
    }
}
```

### Step 2: Verify it compiles

Run: `cargo check -p rara-kernel`
Expected: PASS

### Step 3: Commit

```bash
git add crates/kernel/src/kernel.rs
git commit -m "fix(kernel): user-friendly context window error message

Show actionable Chinese message when context overflows instead
of raw error string.

Closes #N"
```

---

## Parallel Execution Strategy

Tasks 1, 2, and 3 are independent and can be worked on in parallel worktrees:

| Task | Branch | Worktree |
|------|--------|----------|
| 1 | `issue-N-usage-tape-tools` | `.worktrees/issue-N-usage-tape-tools` |
| 2 | `issue-M-image-compression` | `.worktrees/issue-M-image-compression` |
| 3 | `issue-P-context-error-ux` | `.worktrees/issue-P-context-error-ux` |

Merge order: Task 3 first (smallest), then Task 1, then Task 2 (Task 2 may need conflict resolution in `mod.rs` if Task 1 also touched it).
