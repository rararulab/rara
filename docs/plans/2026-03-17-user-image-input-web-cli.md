# User Image Input for Web and CLI Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add end-to-end user image input support for both the web chat and CLI chat so both entrypoints can submit `MessageContent::Multimodal` to the kernel.

**Architecture:** The kernel already supports multimodal user messages, so the missing work is at the ingress edges. Web will move from text-only message frames to a backward-compatible structured payload that can carry `text`, `image_url`, and `image_base64` blocks; CLI will add explicit image attachment staging from local file paths and build multimodal raw messages before ingest. Both paths should preserve text-only behavior and be covered by focused adapter/helper tests plus targeted UI tests.

**Tech Stack:** Rust, Axum WebSocket/JSON, Ratatui CLI, React 19, TypeScript, Vitest, Playwright, existing `rara_kernel::channel::types::MessageContent` and `rara_kernel::llm::image`

---

### Task 1: Web ingress contract supports multimodal payloads

**Files:**
- Modify: `crates/channels/src/web.rs`
- Test: `crates/channels/src/web.rs`

**Step 1: Write the failing tests**

Add unit tests near the existing `stream_event_to_web_event` tests for:

```rust
#[test]
fn parses_legacy_text_frame_as_plain_text_message() {
    let payload = parse_inbound_text_frame("hello world").expect("payload");

    assert!(matches!(payload.content, MessageContent::Text(text) if text == "hello world"));
}

#[test]
fn parses_multimodal_json_frame() {
    let raw = serde_json::json!({
        "content": [
            { "type": "text", "text": "look at this" },
            {
                "type": "image_base64",
                "media_type": "image/png",
                "data": "AAAA"
            }
        ]
    })
    .to_string();

    let payload = parse_inbound_text_frame(&raw).expect("payload");

    assert!(matches!(payload.content, MessageContent::Multimodal(blocks) if blocks.len() == 2));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p rara-channels parses_multimodal_json_frame -- --nocapture`
Expected: FAIL with unresolved `parse_inbound_text_frame` / text-only request assumptions

**Step 3: Write minimal implementation**

In `crates/channels/src/web.rs`:

- Introduce a backward-compatible inbound payload type:

```rust
#[derive(Debug, Deserialize)]
struct InboundWebMessage {
    content: MessageContent,
}

fn parse_inbound_text_frame(text: &str) -> anyhow::Result<InboundWebMessage> {
    if let Ok(payload) = serde_json::from_str::<InboundWebMessage>(text) {
        return Ok(payload);
    }

    Ok(InboundWebMessage {
        content: MessageContent::Text(text.to_owned()),
    })
}
```

- Change `build_raw_platform_message()` to accept `MessageContent` instead of `&str`
- Update both WebSocket and `POST /messages` handlers to feed parsed `MessageContent` into `RawPlatformMessage`
- Extend `SendMessageRequest` from `content: String` to `content: MessageContent`

**Step 4: Run tests to verify they pass**

Run: `cargo test -p rara-channels web::tests -- --nocapture`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/channels/src/web.rs
git commit -m "feat(channels): accept multimodal web chat payloads (#464)" -m "Closes #464"
```

---

### Task 2: Web frontend serialization and rendering support `image_base64`

**Files:**
- Create: `web/src/lib/chat-attachments.ts`
- Create: `web/src/lib/chat-attachments.test.ts`
- Modify: `web/src/api/types.ts`
- Modify: `web/src/pages/Chat.tsx`

**Step 1: Write the failing tests**

Create `web/src/lib/chat-attachments.test.ts` with:

```ts
import { describe, expect, it } from "vitest";

import {
  buildOutboundChatContent,
  imageBlockSrc,
} from "./chat-attachments";

describe("chat attachments", () => {
  it("builds a multimodal payload when urls or inline images exist", () => {
    expect(
      buildOutboundChatContent("look", [
        { type: "image_url", url: "https://example.com/cat.png" },
      ]),
    ).toEqual([
      { type: "text", text: "look" },
      { type: "image_url", url: "https://example.com/cat.png" },
    ]);
  });

  it("renders image_base64 blocks as data urls", () => {
    expect(
      imageBlockSrc({
        type: "image_base64",
        media_type: "image/png",
        data: "AAAA",
      }),
    ).toBe("data:image/png;base64,AAAA");
  });
});
```

**Step 2: Run test to verify it fails**

Run: `npm --prefix web test -- src/lib/chat-attachments.test.ts`
Expected: FAIL with missing module / missing `image_base64` type

**Step 3: Write minimal implementation**

In `web/src/lib/chat-attachments.ts` add helpers:

```ts
import type { ChatContentBlock } from "@/api/types";

export function buildOutboundChatContent(
  text: string,
  blocks: ChatContentBlock[],
): string | ChatContentBlock[] {
  const trimmed = text.trim();
  if (blocks.length === 0) return trimmed;

  return [
    ...(trimmed ? [{ type: "text", text: trimmed } satisfies ChatContentBlock] : []),
    ...blocks,
  ];
}

export function imageBlockSrc(block: Extract<ChatContentBlock, { type: "image_url" | "image_base64" }>): string {
  return block.type === "image_url"
    ? block.url
    : `data:${block.media_type};base64,${block.data}`;
}
```

Update `web/src/api/types.ts`:

```ts
export type ChatContentBlock =
  | { type: "text"; text: string }
  | { type: "image_url"; url: string }
  | { type: "image_base64"; media_type: string; data: string };
```

Update `web/src/pages/Chat.tsx` rendering so `MessageBubble` and attachment previews use `imageBlockSrc()` and handle `image_base64`.

**Step 4: Run tests to verify they pass**

Run: `npm --prefix web test -- src/lib/chat-attachments.test.ts`
Expected: PASS

**Step 5: Commit**

```bash
git add web/src/api/types.ts web/src/lib/chat-attachments.ts web/src/lib/chat-attachments.test.ts web/src/pages/Chat.tsx
git commit -m "feat(web): add chat attachment serialization helpers (#464)" -m "Closes #464"
```

---

### Task 3: Web chat UI can attach local images and send structured payloads

**Files:**
- Modify: `web/src/pages/Chat.tsx`
- Test: `web/e2e/chat.spec.ts`

**Step 1: Write the failing test**

Add a Playwright test that verifies the chat composer can submit a structured payload:

```ts
test("web chat sends multimodal payloads for local image attachments", async ({ page }) => {
  await page.goto("/agent?tab=chat");

  await page.route("**/api/v1/kernel/chat/ws**", async (route) => {
    await route.continue();
  });

  // Implementation detail: assert UI exposes an image file input and preview.
  await expect(page.locator('input[type="file"][accept*="image"]')).toHaveCount(1);
});
```

If Playwright-level WebSocket inspection is too awkward, replace this with a narrower helper-driven test plus a smoke assertion that the upload control is visible. Do not skip automated coverage entirely.

**Step 2: Run test to verify it fails**

Run: `npm --prefix web run test:e2e -- chat.spec.ts --grep "multimodal payloads"`
Expected: FAIL because there is no local image input or structured send path

**Step 3: Write minimal implementation**

In `web/src/pages/Chat.tsx`:

- Replace `imageUrls: string[]` with attachment blocks:

```ts
const [attachments, setAttachments] = useState<ChatContentBlock[]>([]);
```

- Add a hidden file input:

```tsx
<input
  ref={fileInputRef}
  type="file"
  accept="image/*"
  multiple
  className="hidden"
  onChange={handleFileSelection}
/>
```

- In `handleFileSelection`, convert each selected file into `{ type: "image_base64", media_type, data }`
- Keep URL paste support by pushing `{ type: "image_url", url }`
- Change `sendMessage()` to send JSON over WebSocket:

```ts
wsRef.current.send(JSON.stringify({
  content: buildOutboundChatContent(trimmed, attachments),
}));
```

- Allow image-only sends by treating non-empty attachments as sendable even when trimmed text is empty

**Step 4: Run tests to verify they pass**

Run: `npm --prefix web test -- src/lib/chat-attachments.test.ts`
Expected: PASS

Run: `npm --prefix web run build`
Expected: PASS

**Step 5: Commit**

```bash
git add web/src/pages/Chat.tsx web/e2e/chat.spec.ts
git commit -m "feat(web): support local image attachments in chat composer (#464)" -m "Closes #464"
```

---

### Task 4: CLI chat stages local image paths and builds multimodal raw messages

**Files:**
- Modify: `crates/cmd/src/chat/app.rs`
- Modify: `crates/cmd/src/chat/mod.rs`
- Test: `crates/cmd/src/chat/app.rs`
- Test: `crates/cmd/src/chat/mod.rs`

**Step 1: Write the failing tests**

In `crates/cmd/src/chat/app.rs` add a state test:

```rust
#[test]
fn enter_without_text_sends_when_images_are_staged() {
    let mut chat = ChatState::new("default".into(), "local".into());
    chat.staged_images.push("/tmp/cat.png".into());

    let action = chat.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

    assert!(matches!(action, ChatAction::SendMessage(text) if text.is_empty()));
}
```

In `crates/cmd/src/chat/mod.rs` add a builder test:

```rust
#[test]
fn cli_raw_message_is_multimodal_when_image_paths_are_present() {
    let raw = build_cli_raw_message(
        "default",
        "local",
        "describe",
        vec![rara_kernel::channel::types::ContentBlock::ImageBase64 {
            media_type: "image/png".to_owned(),
            data: "AAAA".to_owned(),
        }],
    );

    assert!(matches!(raw.content, MessageContent::Multimodal(blocks) if blocks.len() == 2));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p rara-cli enter_without_text_sends_when_images_are_staged -- --nocapture`
Expected: FAIL because `staged_images` does not exist and `build_cli_raw_message()` is text-only

**Step 3: Write minimal implementation**

In `crates/cmd/src/chat/app.rs`:

- Add staged image paths to `ChatState`
- Teach `/help` output about `/image <path>` and `/images`
- Permit `Enter` to dispatch when either text is present or image paths are staged

In `crates/cmd/src/chat/mod.rs`:

- Add a small helper that loads local files, runs `rara_kernel::llm::image::compress_image()`, and converts them into `ContentBlock::ImageBase64`
- Change `ChatAction::SendMessage` handling to build multimodal content from current text + staged images
- Add slash commands:

```rust
"/image /abs/path/to/file.png"
"/images"
"/clear-images"
```

- Change `build_cli_raw_message()` to accept `Vec<ContentBlock>` attachments and emit `MessageContent::Multimodal` when non-empty

**Step 4: Run tests to verify they pass**

Run: `cargo test -p rara-cli chat::app::tests -- --nocapture`
Expected: PASS

Run: `cargo test -p rara-cli chat::tests -- --nocapture`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/cmd/src/chat/app.rs crates/cmd/src/chat/mod.rs
git commit -m "feat(cli): stage local image attachments for chat (#464)" -m "Closes #464"
```

---

### Task 5: End-to-end verification and final cleanup

**Files:**
- Modify: `docs/plans/2026-03-17-user-image-input-web-cli.md`

**Step 1: Run focused Rust verification**

Run: `cargo test -p rara-channels web::tests -- --nocapture`
Expected: PASS

Run: `cargo test -p rara-cli chat::tests -- --nocapture`
Expected: PASS

**Step 2: Run focused frontend verification**

Run: `npm --prefix web test -- src/lib/chat-attachments.test.ts`
Expected: PASS

Run: `npm --prefix web run build`
Expected: PASS

**Step 3: Record any environment-specific blockers**

If the worktree still lacks frontend dependencies, install them first:

```bash
npm --prefix web install
```

Then rerun the web commands above. Record the exact blocker and fix in the PR description if anything non-obvious was required.

**Step 4: Final commit**

```bash
git add docs/plans/2026-03-17-user-image-input-web-cli.md
git commit -m "chore(plan): record verification for user image input work (#464)" -m "Closes #464"
```

