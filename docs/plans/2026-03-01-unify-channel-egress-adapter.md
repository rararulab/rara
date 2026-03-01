# Unify ChannelAdapter / EgressAdapter — Remove ChannelAdapter::send (#379)

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Remove the duplicate `send()` method from `ChannelAdapter`, making outbound delivery the exclusive responsibility of `EgressAdapter`. `ChannelAdapter` becomes a pure lifecycle + UX hook trait.

**Architecture:** The I/O Bus model has two adapter interfaces — `InboundSink` (ingress) and `EgressAdapter` (egress). The older `ChannelAdapter` trait still carries a `send()` method that overlaps with `EgressAdapter::send()`. We remove `ChannelAdapter::send()` and the `OutboundMessage` type it uses, since all outbound delivery now goes through the Egress engine. `PhotoAttachment`, `ReplyMarkup`, and `InlineButton` types stay — they're channel-level concepts that `EgressAdapter` impls may reference.

**Tech Stack:** Rust, axum, async-trait

---

### Task 1: Remove `send()` from `ChannelAdapter` trait

**Files:**
- Modify: `crates/core/kernel/src/channel/adapter.rs`

**Step 1: Remove the `send` method from the trait definition**

Remove the `send` method and the `OutboundMessage` import:

```rust
// adapter.rs — BEFORE (line 26):
use super::types::{AgentPhase, ChannelType, OutboundMessage};

// adapter.rs — AFTER:
use super::types::{AgentPhase, ChannelType};
```

Remove this line from the trait body (line 55):
```rust
    async fn send(&self, message: OutboundMessage) -> Result<(), KernelError>;
```

Also update the doc comment — remove references to `send` from the `# Lifecycle` section (lines 31-38). The lifecycle becomes: start → (typing/phase) → stop.

**Step 2: Run `cargo check -p rara-kernel`**

Expected: PASS (OutboundMessage is still defined in types.rs, just not imported here)

**Step 3: Commit**

```bash
git add crates/core/kernel/src/channel/adapter.rs
git commit -m "refactor(kernel): remove send() from ChannelAdapter trait (#379)"
```

---

### Task 2: Remove `ChannelAdapter::send` impl from WebAdapter

**Files:**
- Modify: `crates/core/channels/src/web.rs`

**Step 1: Remove the `send` impl and unused import**

In the `ChannelAdapter for WebAdapter` impl block (around line 669), delete the `send` method (lines 679-688):

```rust
    // DELETE this entire method:
    async fn send(&self, message: OutboundMessage) -> Result<(), KernelError> {
        WebAdapter::broadcast_event(
            &self.sessions,
            &message.session_key,
            &WebEvent::Message {
                content: message.content,
            },
        );
        Ok(())
    }
```

Remove `OutboundMessage` from the import at line 57:
```rust
// BEFORE:
        types::{AgentPhase, ChannelType, MessageContent, OutboundMessage},
// AFTER:
        types::{AgentPhase, ChannelType, MessageContent},
```

Also update the module doc comment at line 27 — remove the `send` reference:
```rust
// BEFORE:
//! [`send`](ChannelAdapter::send).
// AFTER (delete the line or replace with):
//! [`EgressAdapter`] implementation.
```

**Step 2: Remove the `send_broadcasts_to_session` test**

Delete the test at lines 896-915 that calls `ChannelAdapter::send`. The `EgressAdapter::send` test (if any) covers outbound delivery.

**Step 3: Run `cargo check -p rara-channels`**

Expected: PASS

**Step 4: Run `cargo test -p rara-channels`**

Expected: PASS (one less test, all remaining tests pass)

**Step 5: Commit**

```bash
git add crates/core/channels/src/web.rs
git commit -m "refactor(web): remove ChannelAdapter::send impl (#379)"
```

---

### Task 3: Remove `ChannelAdapter::send` impl from TelegramAdapter

**Files:**
- Modify: `crates/core/channels/src/telegram/adapter.rs`

**Step 1: Remove the `send` impl**

In the `ChannelAdapter for TelegramAdapter` impl block (around line 359), delete the entire `send` method (lines 412-485). This is the large method handling edit/photo/text cases — all now handled by `EgressAdapter::send`.

Remove `OutboundMessage` from the import at line 55:
```rust
// BEFORE:
            AgentPhase, ChannelType, InlineButton, MessageContent, OutboundMessage, ReplyMarkup,
// AFTER:
            AgentPhase, ChannelType, MessageContent,
```

Note: `InlineButton` and `ReplyMarkup` may still be needed by the `EgressAdapter` impl or by `convert_reply_markup`. Check if they're used elsewhere in the file — if only by the deleted `ChannelAdapter::send`, remove from the import.

**Step 2: Update the doc comment**

In `contacts/tracker.rs` line 21, update the reference:
```rust
// BEFORE:
//! notification via [`ChannelAdapter::send`].
// AFTER:
//! notification via [`EgressAdapter::send`].
```

**Step 3: Run `cargo check -p rara-channels`**

Expected: PASS. If `InlineButton`/`ReplyMarkup` imports become unused, remove them.

**Step 4: Run `cargo test -p rara-channels`**

Expected: PASS

**Step 5: Commit**

```bash
git add crates/core/channels/src/telegram/adapter.rs crates/core/channels/src/telegram/contacts/tracker.rs
git commit -m "refactor(telegram): remove ChannelAdapter::send impl (#379)"
```

---

### Task 4: Delete `OutboundMessage` and its test

**Files:**
- Modify: `crates/core/kernel/src/channel/types.rs`
- Modify: `crates/core/kernel/src/channel/mod.rs`

**Step 1: Delete `OutboundMessage` struct**

In `types.rs`, delete the `OutboundMessage` section (lines 336-362):

```rust
// DELETE this entire block:
// ---------------------------------------------------------------------------
// OutboundMessage
// ---------------------------------------------------------------------------

/// A message to send back through a channel.
///
/// The adapter is responsible for formatting the content appropriately
/// for its platform (e.g. Telegram HTML, Slack mrkdwn, plain text).
#[derive(Debug, Clone)]
pub struct OutboundMessage {
    pub channel_type:        ChannelType,
    pub session_key:         String,
    pub content:             String,
    pub metadata:            HashMap<String, serde_json::Value>,
    pub photo:               Option<PhotoAttachment>,
    pub reply_markup:        Option<ReplyMarkup>,
    pub edit_message_id:     Option<String>,
    pub reply_to_message_id: Option<String>,
}
```

**Step 2: Delete the `outbound_message_defaults` test**

In `types.rs`, delete the test (around lines 598-612):

```rust
    // DELETE:
    #[test]
    fn outbound_message_defaults() {
        let msg = OutboundMessage { ... };
        assert!(msg.photo.is_none());
        assert!(msg.reply_markup.is_none());
    }
```

**Step 3: Update the channel mod.rs doc**

In `mod.rs` line 42, remove the `OutboundMessage` reference:
```rust
// DELETE this line:
//! - [`OutboundMessage`](types::OutboundMessage) — response to send back
```

**Step 4: Run `cargo check -p rara-kernel`**

Expected: PASS. If `HashMap` import in types.rs becomes unused (check — it's also used by `ChannelMessage`), leave it.

**Step 5: Run `cargo test -p rara-kernel`**

Expected: PASS

**Step 6: Commit**

```bash
git add crates/core/kernel/src/channel/types.rs crates/core/kernel/src/channel/mod.rs
git commit -m "refactor(kernel): delete OutboundMessage type (#379)"
```

---

### Task 5: Clean up `rara-app` lib.rs comments (if any stale references)

**Files:**
- Modify: `crates/app/src/lib.rs` (only if needed)

**Step 1: Verify no stale `ChannelAdapter::send` references**

The `lib.rs` uses `ChannelAdapter` for `start()` and `stop()` only (lines 408-486). These are correct — no changes needed to the code itself. Just verify with:

```bash
cargo check -p rara-app
```

Expected: PASS

**Step 2: Run full workspace check**

```bash
cargo check --workspace
```

Expected: PASS

**Step 3: Run full test suite**

```bash
cargo test -p rara-kernel && cargo test -p rara-channels
```

Expected: All tests pass.

**Step 4: Commit (only if changes were needed)**

If no changes needed, skip this commit.

---

### Task 6: Fix existing diagnostics (pin_project + unused import)

**Files:**
- Modify: `crates/core/channels/src/web.rs` (pin_project error at line 597)
- Modify: `crates/core/channels/src/callbacks.rs` (unused import at line 183)

**Step 1: Fix pin_project error in web.rs**

Check line 597 of web.rs — there's a `pin_project` usage that fails to resolve. Either add `pin-project` to Cargo.toml dependencies or replace with `std::pin::Pin` manual projection.

**Step 2: Fix unused import in callbacks.rs**

Remove unused `McpServerStatus` import at line 183.

**Step 3: Run `cargo check -p rara-channels`**

Expected: PASS with no warnings

**Step 4: Commit**

```bash
git add crates/core/channels/src/web.rs crates/core/channels/src/callbacks.rs
git commit -m "fix(channels): resolve pin_project and unused import warnings (#379)"
```

---

### Summary of Changes

| File | Action |
|------|--------|
| `kernel/src/channel/adapter.rs` | Remove `send()` from trait, update docs |
| `kernel/src/channel/types.rs` | Delete `OutboundMessage` struct + test |
| `kernel/src/channel/mod.rs` | Remove `OutboundMessage` doc reference |
| `channels/src/web.rs` | Remove `ChannelAdapter::send` impl + test, fix pin_project |
| `channels/src/telegram/adapter.rs` | Remove `ChannelAdapter::send` impl |
| `channels/src/telegram/contacts/tracker.rs` | Update doc reference |
| `channels/src/callbacks.rs` | Remove unused import |

**Final state:** `ChannelAdapter` = lifecycle (start/stop) + UX hooks (typing/phase). `EgressAdapter` = outbound delivery. `InboundSink` = inbound ingestion. Zero overlap.
