# Kernel Path: Web Adapter Integration & Chat Service Slimming

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Wire up the existing WebAdapter to the kernel's I/O Bus pipeline so the frontend can chat through the kernel path (IngressPipeline → TickLoop → process_loop), then slim the backend-admin chat module to session/memory management only.

**Architecture:** WebAdapter is already fully implemented (`channels/src/web.rs`) but not instantiated or mounted. We add it to `io_pipeline.rs`, implement `EgressAdapter` for outbound delivery, and mount its router into the HTTP server. Then we remove `send_message`/`send_message_streaming` from ChatService, delete ChatAgent, and remove the AgentContext dependency chain.

**Tech Stack:** Rust, axum, tokio broadcast channels, rara-kernel I/O Bus (InboundBus/OutboundBus), WebSocket/SSE

---

## Task 1: Implement EgressAdapter for WebAdapter

WebAdapter currently implements `ChannelAdapter` (old trait) but not `EgressAdapter` (new I/O Bus trait). The Egress engine needs `EgressAdapter::send(endpoint, PlatformOutbound)` to deliver replies.

**Files:**
- Modify: `crates/core/channels/src/web.rs:467-516`

**Step 1: Add EgressAdapter import and impl**

After the existing `ChannelAdapter` impl block (line 516), add:

```rust
#[async_trait]
impl EgressAdapter for WebAdapter {
    fn channel_type(&self) -> ChannelType { ChannelType::Web }

    async fn send(&self, endpoint: &Endpoint, msg: PlatformOutbound) -> Result<(), EgressError> {
        // Extract session key from PlatformOutbound
        let (session_key, content) = match msg {
            PlatformOutbound::Reply { session_key, content, .. } => (session_key, content),
            PlatformOutbound::StreamChunk { session_key, delta, .. } => (session_key, delta),
            PlatformOutbound::Progress { session_key, text } => (session_key, text),
        };

        WebAdapter::broadcast_event(
            &self.sessions,
            &session_key,
            &WebEvent::Message { content },
        );
        Ok(())
    }
}
```

Add the necessary imports at the top of `web.rs`:
```rust
use rara_kernel::io::egress::{EgressAdapter, EgressError, Endpoint, PlatformOutbound};
```

**Step 2: Verify it compiles**

Run: `cargo check -p rara-channels`

**Step 3: Commit**

```
feat(channels): implement EgressAdapter for WebAdapter
```

---

## Task 2: Wire WebAdapter into I/O Pipeline

Add WebAdapter creation and registration in `io_pipeline.rs`, similar to how TelegramAdapter is handled.

**Files:**
- Modify: `crates/app/src/io_pipeline.rs:81-145`

**Step 1: Accept and register WebAdapter**

Change `init_io_pipeline` signature to accept an optional `WebAdapter`:

```rust
pub fn init_io_pipeline(
    telegram_adapter: Option<Arc<rara_channels::telegram::TelegramAdapter>>,
    web_adapter: Option<Arc<rara_channels::web::WebAdapter>>,
    session_repo: Arc<dyn rara_kernel::session_manager::SessionRepository>,
    mut kernel: Kernel,
) -> IoBusPipeline {
```

Add `web_adapter` field to `IoBusPipeline`:

```rust
pub struct IoBusPipeline {
    // ... existing fields ...
    pub web_adapter: Option<Arc<rara_channels::web::WebAdapter>>,
}
```

In the egress adapters map (after telegram insertion), add:

```rust
if let Some(ref web) = web_adapter {
    adapters.insert(ChannelType::Web, web.clone() as Arc<dyn EgressAdapter>);
}
```

Return `web_adapter` in the struct.

**Step 2: Update call site in `lib.rs`**

In `crates/app/src/lib.rs`, create WebAdapter and pass it:

```rust
let web_adapter = Arc::new(rara_channels::web::WebAdapter::new());

let io_pipeline = io_pipeline::init_io_pipeline(
    telegram_adapter.clone(),
    Some(web_adapter.clone()),
    app_state.kernel_session_repo.clone(),
    kernel,
);
```

**Step 3: Start WebAdapter with ingress pipeline**

After the Telegram adapter start block (around line 411), add:

```rust
// Start WebAdapter with the I/O Bus ingress pipeline.
{
    use rara_kernel::channel::adapter::ChannelAdapter as _;
    match web_adapter.start(io_pipeline.ingress_pipeline.clone()).await {
        Ok(()) => info!("WebAdapter started (I/O Bus)"),
        Err(e) => warn!(
            error = %e,
            "Failed to start WebAdapter, I/O Bus web ingress inactive"
        ),
    }
}
```

**Step 4: Mount WebAdapter router into HTTP server**

The WebAdapter exposes `adapter.router()` which returns a plain `axum::Router` with `/ws`, `/events`, `/messages`. Nest it under `/api/v1/kernel/chat` in the routes_fn closure:

```rust
let web_router = web_adapter.router();
let routes_fn: Box<dyn Fn(axum::Router) -> axum::Router + Send + Sync> =
    Box::new(move |router| {
        health_routes(router)
            .merge(domain_routes.clone())
            .merge(swagger_ui.clone())
            .nest("/api/v1/kernel/chat", web_router.clone())
    });
```

**Step 5: Verify it compiles**

Run: `cargo check -p rara-app`

**Step 6: Commit**

```
feat(app): wire WebAdapter into I/O Bus pipeline and HTTP server
```

---

## Task 3: Slim ChatService — Remove LLM Execution

Remove `send_message`, `send_message_streaming`, `ChatAgent` dependency, compaction, and consolidation from ChatService. Keep session/message CRUD, model catalog, channel bindings, memory export, fork.

**Files:**
- Modify: `crates/extensions/backend-admin/src/chat/service.rs`
- Modify: `crates/extensions/backend-admin/src/chat/router.rs`
- Modify: `crates/extensions/backend-admin/src/chat/mod.rs`
- Delete: `crates/extensions/backend-admin/src/chat/agent.rs`
- Delete: `crates/extensions/backend-admin/src/chat/stream.rs`

**Step 1: Remove ChatAgent from ChatService**

In `service.rs`:

1. Remove `chat_agent: ChatAgent` field from `ChatService` struct
2. Remove `ChatAgent` from `new()` parameter and body
3. Remove `current_default_model()` method (line 101) — this uses `chat_agent.ctx()`
4. Remove `current_system_prompt()` method (lines 103-107) — same
5. Remove `tools()` method (line 320) — same
6. Remove `persist_compaction()` method (lines 322-342)
7. Remove `prepare_session_data()` method (lines 352-451) and `SessionData` struct
8. Remove `send_message()` method (lines 455-517)
9. Remove `send_message_streaming()` method (lines 521-613)
10. Remove `extract_exchange_pairs()` helper function (lines 784-805)
11. Remove `SESSION_INACTIVITY_THRESHOLD` constant (line 47)
12. Remove unused imports: `ChatAgent`, `ChatStreamEvent`, `UserContent`, `ToolRegistry`, `jiff`, `tracing_opentelemetry::OpenTelemetrySpanExt`, `mpsc`

For `create_session`, the `model` and `system_prompt` parameters now just use the provided values directly (no fallback via AgentContext):

```rust
pub async fn create_session(
    &self,
    key: SessionKey,
    title: Option<String>,
    model: Option<String>,
    system_prompt: Option<String>,
) -> Result<SessionEntry, ChatError> {
    let now = Utc::now();
    let entry = SessionEntry {
        key,
        title,
        model,
        system_prompt,
        message_count: 0,
        preview: None,
        metadata: None,
        created_at: now,
        updated_at: now,
    };
    let created = self.session_repo.create_session(&entry).await?;
    info!(key = %created.key, "session created");
    Ok(created)
}
```

Update `new()`:

```rust
pub fn new(
    session_repo: Arc<dyn SessionRepository>,
    settings_updater: Arc<dyn rara_domain_shared::settings::SettingsUpdater>,
    settings_rx: watch::Receiver<Settings>,
) -> Self {
    Self {
        session_repo,
        model_catalog: ModelCatalog::new(),
        settings_updater,
        settings_rx,
    }
}
```

**Step 2: Remove send/stream routes from router.rs**

In `router.rs`:

1. Remove `send_message` handler function and its `#[utoipa::path]` annotation
2. Remove `stream_message` handler function and its `#[utoipa::path]` annotation
3. Remove `SendMessageRequest` and `SendMessageResponse` types
4. Remove `message_routes` from `routes()` function — move `get_messages` and `clear_messages` into `session_routes`:

```rust
pub fn routes(service: ChatService) -> OpenApiRouter {
    model_routes(service.clone())
        .merge(session_routes(service))
}

fn session_routes(service: ChatService) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(create_session, list_sessions))
        .routes(routes!(get_session, update_session, delete_session))
        .routes(routes!(get_messages, clear_messages))
        .routes(routes!(fork_session))
        .routes(routes!(bind_channel))
        .routes(routes!(get_channel_binding))
        .with_state(service)
}
```

5. Remove unused imports: `Sse`, `Event`, `KeepAlive`, `StreamExt`, `ReceiverStream`, `ChatStreamEvent`

**Step 3: Delete agent.rs and stream.rs**

```bash
rm crates/extensions/backend-admin/src/chat/agent.rs
rm crates/extensions/backend-admin/src/chat/stream.rs
```

**Step 4: Update mod.rs**

Remove `pub mod agent;` and `pub mod stream;` declarations. Remove `SendMessageRequest` and `SendMessageResponse` from re-exports.

**Step 5: Update worker_state.rs**

In `crates/workers/src/worker_state.rs`:

1. Remove `ChatAgent` creation (lines 333-334)
2. Update `ChatService::new()` call to not pass `chat_agent` (line 336-343)
3. Keep `agent_ctx` for now (other things may still use it) — or remove if nothing else references it

**Step 6: Verify it compiles**

Run: `cargo check -p rara-backend-admin && cargo check -p rara-workers && cargo check -p rara-app`

Check for any remaining references to ChatAgent or send_message in other crates.

**Step 7: Commit**

```
refactor(chat): remove LLM execution from ChatService, delete ChatAgent

ChatService is now purely session/message/model management.
LLM chat execution moves to the kernel path via WebAdapter.
```

---

## Task 4: Fix Remaining Compilation & Verify

After tasks 1-3, do a full workspace check and fix any remaining issues.

**Step 1: Full workspace check**

Run: `cargo check --workspace`

Fix any compilation errors from removed types/methods being referenced elsewhere (workers, other extensions).

**Step 2: Run existing tests**

Run: `cargo test -p rara-channels` (WebAdapter tests)
Run: `cargo test -p rara-kernel` (process_loop tests)
Run: `cargo test -p rara-backend-admin` (remaining chat tests)

**Step 3: Frontend build check**

Run: `cd web && npm run build`

The frontend still hits `/api/v1/chat/sessions/{key}/stream` — this will now 404. The frontend needs to be updated to use `/api/v1/kernel/chat/ws` or `/api/v1/kernel/chat/messages` instead. This is a separate follow-up task.

**Step 4: Commit any fixes**

```
fix: resolve compilation issues after ChatAgent removal
```

---

## Execution Order & Dependencies

```
Task 1 (EgressAdapter)
    ↓
Task 2 (Wire WebAdapter)  ← depends on Task 1
    ↓
Task 3 (Slim ChatService) ← independent, but logical after Task 2
    ↓
Task 4 (Verify)           ← depends on all above
```

## Follow-up Tasks (NOT in this plan)

- Update frontend to use WebAdapter endpoints (`/api/v1/kernel/chat/ws` or `/messages`)
- Add StreamHub SSE endpoint for real-time token streaming
- Implement endpoint registration in WebAdapter (register/unregister in EndpointRegistry on WS/SSE connect/disconnect)
- Delete `AgentContext` trait + `AgentContextImpl` once nothing references them
- Re-implement compaction/consolidation in kernel's process_loop
