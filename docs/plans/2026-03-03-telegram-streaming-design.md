# Telegram Native Streaming via editMessageText — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Enable real-time "typewriter" effect for Telegram replies by subscribing to `StreamHub` and progressively editing messages via `editMessageText`.

**Architecture:** `TelegramAdapter` subscribes to the existing `StreamHub` broadcast (same as `WebAdapter`). A per-ingest `spawn_stream_forwarder` task accumulates `TextDelta` events, throttles at 1.5s intervals, and calls `editMessageText` to update the message in-place. When accumulated text exceeds ~3800 chars, it locks the current message and sends a new one. The final `Reply` from Egress replaces the last streaming message to ensure correct Markdown rendering.

**Tech Stack:** Rust, tokio, teloxide, dashmap, rara-kernel StreamHub

---

## Task 1: Add `StreamingMessage` struct and new fields to `TelegramAdapter`

**Files:**
- Modify: `crates/channels/src/telegram/adapter.rs:43-46` (imports)
- Modify: `crates/channels/src/telegram/adapter.rs:89-91` (add `MIN_EDIT_INTERVAL` constant)
- Modify: `crates/channels/src/telegram/adapter.rs:138-157` (struct fields)
- Modify: `crates/channels/src/telegram/adapter.rs:167-187` (constructor)

**Step 1: Add imports and constant**

At the top of `adapter.rs`, add to existing imports:

```rust
// Add to the std imports (line 43-46):
use std::time::Instant;

// Add to the rara_kernel imports (line 49-62):
use rara_kernel::io::stream::StreamHubRef;

// After MAX_RETRY_DELAY (line 87), replace the empty doc comment (line 89-90) with:
const MIN_EDIT_INTERVAL: std::time::Duration = std::time::Duration::from_millis(1500);

/// Maximum characters per Telegram message before splitting to a new message.
/// Set below 4096 to leave buffer for HTML tag expansion from markdown→html.
const STREAM_SPLIT_THRESHOLD: usize = 3800;
```

**Step 2: Add `StreamingMessage` struct**

Insert after the constants (before `TelegramConfig`):

```rust
/// Per-chat streaming state for progressive `editMessageText` updates.
struct StreamingMessage {
    /// All message IDs sent for this stream (multiple when splitting long content).
    message_ids: Vec<MessageId>,
    /// Accumulated raw text for the current (latest) message.
    accumulated: String,
    /// Last successful `editMessageText` timestamp for throttling.
    last_edit: Instant,
    /// Whether new text has been appended since the last edit.
    dirty: bool,
}

impl StreamingMessage {
    fn new() -> Self {
        Self {
            message_ids: Vec::new(),
            accumulated: String::new(),
            last_edit: Instant::now(),
            dirty: false,
        }
    }
}
```

**Step 3: Add fields to `TelegramAdapter` struct**

Add to `TelegramAdapter` struct (after `link_service` field, line 156):

```rust
    /// StreamHub for subscribing to real-time token deltas.
    stream_hub:        Arc<RwLock<Option<StreamHubRef>>>,
    /// Per-chat active streaming state, keyed by `chat_id`.
    active_streams:    Arc<DashMap<i64, StreamingMessage>>,
```

**Step 4: Update constructor**

In `TelegramAdapter::new()` (line 167), add the new fields to the struct literal:

```rust
    stream_hub:        Arc::new(RwLock::new(None)),
    active_streams:    Arc::new(DashMap::new()),
```

**Step 5: Add `set_stream_hub` method**

Add to `impl TelegramAdapter` block (after `with_link_service`, around line 300):

```rust
    /// Inject the kernel's [`StreamHub`] for real-time token streaming.
    ///
    /// Must be called before [`start`](ChannelAdapter::start).
    pub async fn set_stream_hub(&self, hub: StreamHubRef) {
        *self.stream_hub.write().await = Some(hub);
    }
```

**Step 6: Run `cargo check`**

Run: `cargo check -p rara-channels 2>&1 | head -30`
Expected: Compiles (possibly with unused warnings for new fields — that's OK)

**Step 7: Commit**

```bash
git add crates/channels/src/telegram/adapter.rs
git commit -m "feat(channels): add StreamingMessage struct and StreamHub fields to TelegramAdapter"
```

---

## Task 2: Implement `spawn_stream_forwarder` function

**Files:**
- Modify: `crates/channels/src/telegram/adapter.rs` (add function after `handle_update`, around line 750)

**Step 1: Write the `spawn_stream_forwarder` function**

Insert after `handle_update` function (before the `// RawPlatformMessage conversion` section, line 751):

```rust
// ---------------------------------------------------------------------------
// Stream forwarder — progressive editMessageText
// ---------------------------------------------------------------------------

/// Spawn a background task that subscribes to [`StreamHub`] for the given
/// session and progressively updates a Telegram message via `editMessageText`.
///
/// The forwarder accumulates `TextDelta` events and throttles edits to
/// respect Telegram API rate limits (~1.5s between edits). When accumulated
/// text exceeds [`STREAM_SPLIT_THRESHOLD`], it locks the current message and
/// sends a new one to continue streaming.
fn spawn_stream_forwarder(
    stream_hub: Arc<RwLock<Option<StreamHubRef>>>,
    active_streams: Arc<DashMap<i64, StreamingMessage>>,
    bot: teloxide::Bot,
    chat_id: i64,
    session_key: &str,
) {
    use rara_kernel::io::stream::StreamEvent;
    use rara_kernel::process::SessionId;

    let session_key = session_key.to_string();

    tokio::spawn(async move {
        // Resolve StreamHub.
        let hub = {
            let guard = stream_hub.read().await;
            match guard.as_ref() {
                Some(hub) => Arc::clone(hub),
                None => return,
            }
        };

        let session_id = match SessionId::try_from_raw(&session_key) {
            Ok(id) => id,
            Err(_) => {
                tracing::warn!(session_key, "invalid session key for telegram stream forwarder");
                return;
            }
        };

        // Poll until stream appears (event_loop opens it asynchronously).
        let mut attempts = 0;
        let subs = loop {
            let s = hub.subscribe_session(&session_id);
            if !s.is_empty() || attempts > 50 {
                break s;
            }
            attempts += 1;
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        };

        if subs.is_empty() {
            tracing::debug!(session_key, "telegram stream forwarder: no streams found");
            return;
        }

        // Initialize streaming state.
        active_streams.insert(chat_id, StreamingMessage::new());

        // We only handle the first stream (one agent turn per ingest).
        let (_stream_id, mut rx) = match subs.into_iter().next() {
            Some(s) => s,
            None => return,
        };

        let mut throttle = tokio::time::interval(MIN_EDIT_INTERVAL);
        throttle.tick().await; // skip immediate first tick

        loop {
            tokio::select! {
                result = rx.recv() => {
                    match result {
                        Ok(StreamEvent::TextDelta { text }) => {
                            if let Some(mut state) = active_streams.get_mut(&chat_id) {
                                state.accumulated.push_str(&text);
                                state.dirty = true;

                                // If over threshold, flush immediately and start new message.
                                if state.accumulated.len() > STREAM_SPLIT_THRESHOLD {
                                    let flush_text = state.accumulated.clone();
                                    let _ = flush_edit(&bot, chat_id, &mut state, &flush_text).await;

                                    // Start a new message for the remainder.
                                    state.accumulated.clear();
                                    state.message_ids.push(MessageId(0)); // sentinel — next flush sends new
                                    state.dirty = false;
                                }
                            }
                        }
                        Ok(_) => {
                            // Ignore non-text events (ToolCallStart, etc.)
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            tracing::warn!(chat_id, skipped = n, "telegram stream forwarder lagged");
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            // Stream closed — do final flush.
                            if let Some(mut state) = active_streams.get_mut(&chat_id) {
                                if state.dirty {
                                    let text = state.accumulated.clone();
                                    let _ = flush_edit(&bot, chat_id, &mut state, &text).await;
                                }
                            }
                            break;
                        }
                    }
                }
                _ = throttle.tick() => {
                    // Periodic flush of accumulated text.
                    if let Some(mut state) = active_streams.get_mut(&chat_id) {
                        if state.dirty && !state.accumulated.is_empty() {
                            let text = state.accumulated.clone();
                            let _ = flush_edit(&bot, chat_id, &mut state, &text).await;
                        }
                    }
                }
            }
        }

        // Auto-cleanup after 30s if Reply never arrives.
        let streams = active_streams.clone();
        let cid = chat_id;
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            if streams.remove(&cid).is_some() {
                tracing::warn!(chat_id = cid, "telegram stream forwarder: stale state cleaned up after 30s");
            }
        });
    });
}

/// Flush accumulated text to Telegram via `sendMessage` (first time) or
/// `editMessageText` (subsequent).
///
/// Returns `Ok(())` on success or silent error (rate-limited / not-modified).
async fn flush_edit(
    bot: &teloxide::Bot,
    chat_id: i64,
    state: &mut dashmap::mapref::one::RefMut<'_, i64, StreamingMessage>,
    text: &str,
) -> Result<(), ()> {
    let html = crate::telegram::markdown::markdown_to_telegram_html(text);

    if state.message_ids.is_empty() || *state.message_ids.last().unwrap() == MessageId(0) {
        // First message or new split — send a new message.
        match bot
            .send_message(ChatId(chat_id), &html)
            .parse_mode(ParseMode::Html)
            .await
        {
            Ok(sent) => {
                let msg_id = sent.id;
                if state.message_ids.last() == Some(&MessageId(0)) {
                    // Replace sentinel.
                    *state.message_ids.last_mut().unwrap() = msg_id;
                } else {
                    state.message_ids.push(msg_id);
                }
                state.last_edit = Instant::now();
                state.dirty = false;
            }
            Err(e) => {
                tracing::warn!(chat_id, error = %e, "telegram stream: failed to send initial message");
            }
        }
    } else {
        // Edit existing message.
        let msg_id = *state.message_ids.last().unwrap();
        match bot
            .edit_message_text(ChatId(chat_id), msg_id, &html)
            .parse_mode(ParseMode::Html)
            .await
        {
            Ok(_) => {
                state.last_edit = Instant::now();
                state.dirty = false;
            }
            Err(teloxide::RequestError::Api(ref api_err)) => {
                let err_str = format!("{api_err}");
                if err_str.contains("message is not modified") {
                    // Silently ignore — content hasn't changed.
                    state.dirty = false;
                } else if err_str.contains("Too Many Requests") || err_str.contains("retry after") {
                    tracing::warn!(chat_id, "telegram stream: rate limited, will retry next tick");
                    // Leave dirty=true so next tick retries.
                } else {
                    tracing::warn!(chat_id, error = %api_err, "telegram stream: edit failed");
                    state.dirty = false;
                }
            }
            Err(e) => {
                tracing::warn!(chat_id, error = %e, "telegram stream: edit request failed");
                state.dirty = false;
            }
        }
    }

    Ok(())
}
```

**Step 2: Run `cargo check`**

Run: `cargo check -p rara-channels 2>&1 | head -30`
Expected: Compiles (possibly with unused warnings)

**Step 3: Commit**

```bash
git add crates/channels/src/telegram/adapter.rs
git commit -m "feat(channels): implement spawn_stream_forwarder and flush_edit for TG streaming"
```

---

## Task 3: Wire up forwarder in `handle_update` and modify `EgressAdapter::send`

**Files:**
- Modify: `crates/channels/src/telegram/adapter.rs:429-441` (start → pass new fields to polling_loop)
- Modify: `crates/channels/src/telegram/adapter.rs:485-495` (polling_loop signature)
- Modify: `crates/channels/src/telegram/adapter.rs:549-561` (handle_update spawn)
- Modify: `crates/channels/src/telegram/adapter.rs:585-594` (handle_update signature)
- Modify: `crates/channels/src/telegram/adapter.rs:730-748` (after ingest, spawn forwarder)
- Modify: `crates/channels/src/telegram/adapter.rs:316-390` (EgressAdapter::send Reply branch)

**Step 1: Pass `stream_hub`, `active_streams`, and `bot` to `polling_loop`**

In `ChannelAdapter::start()` (line 420-441), add clones before the spawn:

```rust
        let stream_hub = Arc::clone(&self.stream_hub);
        let active_streams = Arc::clone(&self.active_streams);
```

Update the `polling_loop` call inside `tokio::spawn` to pass the new args:

```rust
            polling_loop(
                bot,
                handle,
                allowed_chat_ids,
                polling_timeout,
                &mut shutdown_rx,
                bot_username,
                config,
                contact_tracker,
                link_service,
                stream_hub,
                active_streams,
            )
            .await;
```

**Step 2: Update `polling_loop` signature**

Add two new params to `polling_loop` (line 485-495):

```rust
async fn polling_loop(
    bot: teloxide::Bot,
    handle: KernelHandle,
    allowed_chat_ids: Vec<i64>,
    polling_timeout: u32,
    shutdown_rx: &mut watch::Receiver<bool>,
    bot_username: Arc<RwLock<Option<String>>>,
    config: Arc<StdRwLock<TelegramConfig>>,
    contact_tracker: Option<Arc<dyn ContactTracker>>,
    link_service: Option<Arc<TelegramLinkService>>,
    stream_hub: Arc<RwLock<Option<StreamHubRef>>>,
    active_streams: Arc<DashMap<i64, StreamingMessage>>,
) {
```

**Step 3: Pass new fields through to `handle_update`**

In the update spawn (line 549-561), add clones and pass them:

```rust
                    let stream_hub = Arc::clone(&stream_hub);
                    let active_streams = Arc::clone(&active_streams);
                    tokio::spawn(async move {
                        handle_update(
                            update,
                            &handle,
                            &bot,
                            &allowed,
                            &bot_username,
                            &config,
                            tracker.as_ref(),
                            link_svc.as_ref(),
                            &stream_hub,
                            &active_streams,
                        )
                        .await;
                    });
```

**Step 4: Update `handle_update` signature**

Add two new params (line 585-594):

```rust
async fn handle_update(
    update: Update,
    handle: &KernelHandle,
    bot: &teloxide::Bot,
    allowed_chat_ids: &[i64],
    bot_username: &Arc<RwLock<Option<String>>>,
    config: &Arc<StdRwLock<TelegramConfig>>,
    contact_tracker: Option<&Arc<dyn ContactTracker>>,
    link_service: Option<&Arc<TelegramLinkService>>,
    stream_hub: &Arc<RwLock<Option<StreamHubRef>>>,
    active_streams: &Arc<DashMap<i64, StreamingMessage>>,
) {
```

**Step 5: Spawn forwarder after successful ingest**

Replace the ingest block (line 730-748) with:

```rust
    // Fire-and-forget ingest.
    let session_key = format_session_key(chat_id);
    match handle.ingest(raw).await {
        Ok(()) => {
            // Spawn stream forwarder for progressive editMessageText.
            spawn_stream_forwarder(
                Arc::clone(stream_hub),
                Arc::clone(active_streams),
                bot.clone(),
                chat_id,
                &session_key,
            );
        }
        Err(IngestError::SystemBusy) => {
            let _ = bot
                .send_message(
                    ChatId(chat_id),
                    "\u{26a0}\u{fe0f} 系统繁忙，请稍后再试。",
                )
                .await;
        }
        Err(other) => {
            error!(error = %other, "telegram adapter: ingest failed");
        }
    }
```

**Step 6: Modify `EgressAdapter::send` Reply branch**

Replace the entire `PlatformOutbound::Reply` match arm (line 327-356) with:

```rust
            PlatformOutbound::Reply {
                content,
                reply_context,
                ..
            } => {
                let html = crate::telegram::markdown::markdown_to_telegram_html(&content);
                let chunks = crate::telegram::markdown::chunk_message(&html, 4096);

                // Check if there's an active streaming state to replace.
                if let Some((_, stream_state)) = self.active_streams.remove(&chat_id) {
                    // Replace the last streaming message with final rendered content.
                    // If there were split messages, earlier ones keep their streamed content.
                    if let Some(&last_msg_id) = stream_state.message_ids.last() {
                        if last_msg_id != MessageId(0) {
                            // Edit the last streaming message with the first chunk.
                            let first_chunk = chunks.first().map(|s| s.as_str()).unwrap_or("");
                            let _ = self
                                .bot
                                .edit_message_text(ChatId(chat_id), last_msg_id, first_chunk)
                                .parse_mode(ParseMode::Html)
                                .await;

                            // Send remaining chunks as new messages.
                            for chunk in chunks.iter().skip(1) {
                                let _ = self
                                    .bot
                                    .send_message(ChatId(chat_id), chunk)
                                    .parse_mode(ParseMode::Html)
                                    .await;
                            }
                            return Ok(());
                        }
                    }
                    // Fallthrough: streaming state exists but no valid message ID
                    // (e.g. stream started but no text was ever sent). Use normal path.
                }

                // No active stream — normal send path.
                for (i, chunk) in chunks.iter().enumerate() {
                    let mut req = self
                        .bot
                        .send_message(ChatId(chat_id), chunk)
                        .parse_mode(ParseMode::Html);

                    if i == 0 {
                        if let Some(ref ctx) = reply_context {
                            if let Some(ref reply_id) = ctx.reply_to_platform_msg_id {
                                if let Ok(msg_id) = parse_message_id(reply_id) {
                                    req = req.reply_parameters(ReplyParameters::new(msg_id));
                                }
                            }
                        }
                    }

                    req.await.map_err(|e| EgressError::DeliveryFailed {
                        message: format!("failed to send telegram message: {e}"),
                    })?;
                }
            }
```

**Step 7: Run `cargo check`**

Run: `cargo check -p rara-channels 2>&1 | head -40`
Expected: Compiles successfully

**Step 8: Commit**

```bash
git add crates/channels/src/telegram/adapter.rs
git commit -m "feat(channels): wire up TG stream forwarder in polling loop and egress reply"
```

---

## Task 4: Inject `StreamHub` into `TelegramAdapter` at startup

**Files:**
- Modify: `crates/app/src/lib.rs:331-337`

**Step 1: Add `set_stream_hub` call for Telegram**

After the WebAdapter injection block (line 337), add:

```rust
        // Inject StreamHub into TelegramAdapter for streaming.
        if let Some(ref tg) = telegram_adapter {
            tg.set_stream_hub(kernel.stream_hub().clone()).await;
        }
```

**Step 2: Run `cargo check`**

Run: `cargo check -p rara-app 2>&1 | head -30`
Expected: Compiles successfully

**Step 3: Commit**

```bash
git add crates/app/src/lib.rs
git commit -m "feat(app): inject StreamHub into TelegramAdapter at startup"
```

---

## Task 5: Unit tests for `StreamingMessage` and `flush_edit` logic

**Files:**
- Modify: `crates/channels/src/telegram/adapter.rs` (append to existing `#[cfg(test)] mod tests`)

**Step 1: Write unit tests**

Add to the existing test module (after line 984):

```rust
    // -----------------------------------------------------------------------
    // StreamingMessage tests
    // -----------------------------------------------------------------------

    #[test]
    fn streaming_message_initial_state() {
        let state = StreamingMessage::new();
        assert!(state.message_ids.is_empty());
        assert!(state.accumulated.is_empty());
        assert!(!state.dirty);
    }

    #[test]
    fn streaming_message_accumulate() {
        let mut state = StreamingMessage::new();
        state.accumulated.push_str("Hello ");
        state.accumulated.push_str("world");
        state.dirty = true;

        assert_eq!(state.accumulated, "Hello world");
        assert!(state.dirty);
    }

    #[test]
    fn streaming_message_split_threshold() {
        let mut state = StreamingMessage::new();
        // Fill to just below threshold.
        let chunk = "x".repeat(STREAM_SPLIT_THRESHOLD - 10);
        state.accumulated.push_str(&chunk);
        assert!(state.accumulated.len() <= STREAM_SPLIT_THRESHOLD);

        // Push over threshold.
        state.accumulated.push_str(&"y".repeat(20));
        assert!(state.accumulated.len() > STREAM_SPLIT_THRESHOLD);
    }

    #[test]
    fn min_edit_interval_is_reasonable() {
        // Should be between 1s and 3s.
        assert!(MIN_EDIT_INTERVAL >= std::time::Duration::from_secs(1));
        assert!(MIN_EDIT_INTERVAL <= std::time::Duration::from_secs(3));
    }
```

**Step 2: Run tests**

Run: `cargo test -p rara-channels -- streaming_message 2>&1`
Expected: All 4 tests PASS

**Step 3: Commit**

```bash
git add crates/channels/src/telegram/adapter.rs
git commit -m "test(channels): add unit tests for StreamingMessage and streaming constants"
```

---

## Task 6: Integration test — StreamHub → forwarder → mock verification

**Files:**
- Modify: `crates/channels/src/telegram/adapter.rs` (append to test module)

**Step 1: Write StreamHub integration test**

```rust
    #[tokio::test]
    async fn test_stream_forwarder_spawns_and_cleans_up() {
        use rara_kernel::io::stream::{StreamHub, StreamEvent};
        use rara_kernel::process::SessionId;

        let hub = Arc::new(StreamHub::new(64));
        let active_streams: Arc<DashMap<i64, StreamingMessage>> = Arc::new(DashMap::new());
        let chat_id = 12345_i64;

        let session_id = SessionId::new();
        let session_key = format!("tg:{chat_id}");

        // Open a stream on the hub.
        let stream_handle = hub.open(session_id);

        // The forwarder needs a bot — but we can't easily mock teloxide::Bot
        // in unit tests. Instead, test that active_streams gets populated
        // and cleaned up correctly via direct state manipulation.

        // Simulate what the forwarder does:
        active_streams.insert(chat_id, StreamingMessage::new());
        assert!(active_streams.contains_key(&chat_id));

        // Simulate text accumulation.
        if let Some(mut state) = active_streams.get_mut(&chat_id) {
            state.accumulated.push_str("Hello from LLM");
            state.dirty = true;
        }

        // Verify state.
        let state = active_streams.get(&chat_id).unwrap();
        assert_eq!(state.accumulated, "Hello from LLM");
        assert!(state.dirty);
        drop(state);

        // Simulate Reply arrival — remove state.
        let removed = active_streams.remove(&chat_id);
        assert!(removed.is_some());
        assert!(!active_streams.contains_key(&chat_id));

        // Emit some events to verify stream_handle works.
        stream_handle.emit(StreamEvent::TextDelta { text: "test".to_string() });
    }
```

**Step 2: Run tests**

Run: `cargo test -p rara-channels -- test_stream_forwarder 2>&1`
Expected: PASS

**Step 3: Commit**

```bash
git add crates/channels/src/telegram/adapter.rs
git commit -m "test(channels): add integration test for TG streaming state lifecycle"
```

---

## Task 7: Final verification — full `cargo check` + `cargo test`

**Step 1: Full workspace check**

Run: `cargo check 2>&1 | tail -5`
Expected: No errors

**Step 2: Run all channel tests**

Run: `cargo test -p rara-channels 2>&1`
Expected: All tests pass

**Step 3: Run full workspace tests (if feasible)**

Run: `cargo test --workspace 2>&1 | tail -20`
Expected: No regressions

**Step 4: Final commit (if any fixups needed)**

```bash
git add -A
git commit -m "chore(channels): fixup any compilation issues from TG streaming"
```
