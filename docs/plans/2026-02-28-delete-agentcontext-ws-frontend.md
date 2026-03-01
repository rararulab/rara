# Delete AgentContext & WebSocket Frontend Integration

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Remove the AgentContext trait (and its sole implementation AgentContextImpl), migrate workers to use settings/prompt_repo directly, wire StreamHub into WebAdapter for real-time token streaming, and update the frontend to use WebSocket.

**Architecture:** Workers currently use `agent_ctx.build_worker_policy()`, `.model_for_key()`, `.provider_hint()`, `.max_iterations()` — all thin wrappers over `settings_rx` and `prompt_repo`. We replace them with direct reads. For streaming, we inject `StreamHub` into `WebAdapterState` so the WebSocket handler can subscribe to session streams and forward `StreamEvent`s as `WebEvent`s to the client. The frontend switches from SSE POST to WebSocket at `/api/v1/kernel/chat/ws`.

**Tech Stack:** Rust (axum, tokio), TypeScript (React 19, native WebSocket API)

---

## Task 1: Add helper functions to replace AgentContext in workers

Workers need 4 things from AgentContext. Extract these as free functions in `worker_state.rs`.

**Files:**
- Modify: `crates/workers/src/worker_state.rs`

**Step 1: Add `build_worker_policy` helper**

After the `AppState` struct, add:

```rust
/// Build the system prompt for background worker agents.
///
/// Reads `workers/agent_policy.md` and `agent/soul.md` from the prompt repo
/// and combines them. Replaces `AgentContext::build_worker_policy()`.
pub async fn build_worker_policy(
    prompt_repo: &dyn rara_kernel::prompt::PromptRepo,
) -> String {
    let policy = prompt_repo
        .get("workers/agent_policy.md")
        .await
        .map(|e| e.content)
        .unwrap_or_default();
    let soul = prompt_repo
        .get("agent/soul.md")
        .await
        .map(|e| e.content)
        .unwrap_or_default();

    if soul.trim().is_empty() {
        policy
    } else {
        format!("{soul}\n\n# Operational Policy\n{policy}")
    }
}
```

**Step 2: Verify it compiles**

Run: `cargo check -p rara-workers`

**Step 3: Commit**

```
feat(workers): add build_worker_policy helper for AgentContext removal
```

---

## Task 2: Migrate scheduled_agent.rs and proactive.rs to direct settings

Replace `state.agent_ctx.*` calls with direct `state.settings_svc` + `state.prompt_repo` usage.

**Files:**
- Modify: `crates/workers/src/scheduled_agent.rs`
- Modify: `crates/workers/src/proactive.rs`

**Step 1: Update scheduled_agent.rs**

Replace lines 59-69:

```rust
// Before:
let policy = state.agent_ctx.build_worker_policy().await;
let model = state.agent_ctx.model_for_key("scheduled");
// ...
provider_hint: state.agent_ctx.provider_hint(),
max_iterations: Some(state.agent_ctx.max_iterations("scheduled")),
```

With:

```rust
let policy = crate::worker_state::build_worker_policy(state.prompt_repo.as_ref()).await;
let settings = state.settings_svc.current();
let model = settings.ai.model_for_key("scheduled");
let provider_hint = settings.ai.provider.clone();
let max_iterations = settings.agent.max_iterations.map(|n| n as usize).unwrap_or(25);
// ...
provider_hint: provider_hint,
max_iterations: Some(max_iterations),
```

**Step 2: Update proactive.rs**

Same pattern — replace lines 88-97:

```rust
let policy = crate::worker_state::build_worker_policy(state.prompt_repo.as_ref()).await;
let settings = state.settings_svc.current();
let model = settings.ai.model_for_key("proactive");
// ...
provider_hint: settings.ai.provider.clone(),
max_iterations: Some(settings.agent.max_iterations.map(|n| n as usize).unwrap_or(25)),
```

**Step 3: Verify it compiles**

Run: `cargo check -p rara-workers`

**Step 4: Commit**

```
refactor(workers): migrate scheduled/proactive agents to direct settings
```

---

## Task 3: Remove AgentContext from AppState and delete modules

Now that nothing uses `agent_ctx`, remove it from `AppState`, delete `orchestrator.rs`, and delete `agent_context.rs` from kernel.

**Files:**
- Modify: `crates/workers/src/worker_state.rs` — remove `agent_ctx` field and its initialization
- Modify: `crates/workers/src/lib.rs` — remove `pub mod orchestrator;`
- Delete: `crates/workers/src/orchestrator.rs`
- Modify: `crates/core/kernel/src/lib.rs` — remove `pub mod agent_context;`
- Delete: `crates/core/kernel/src/agent_context.rs`

**Step 1: Remove `agent_ctx` from AppState**

In `worker_state.rs`:

1. Remove the `agent_ctx` field (line 69)
2. Remove the `AgentContextImpl` creation block (lines 315-325)
3. Remove `agent_ctx` from the `Ok(Self { ... })` struct literal (line 385)

**Step 2: Delete orchestrator.rs**

```bash
rm crates/workers/src/orchestrator.rs
```

Update `crates/workers/src/lib.rs` — remove `pub mod orchestrator;`.

**Step 3: Delete agent_context.rs from kernel**

```bash
rm crates/core/kernel/src/agent_context.rs
```

Update `crates/core/kernel/src/lib.rs` — remove `pub mod agent_context;`.

**Step 4: Fix any remaining references**

Search for any remaining uses of `agent_context` or `AgentContext` across the workspace and fix.

Run: `cargo check --workspace` (this will show all compilation errors)

Expected areas to fix:
- `rara_kernel` may export `agent_context` types — remove from `lib.rs`
- Workers Cargo.toml may have unnecessary deps (rara_memory, rara_skills) used only by orchestrator — verify and potentially remove

**Step 5: Verify it compiles**

Run: `cargo check --workspace`

**Step 6: Commit**

```
refactor: delete AgentContext trait and AgentContextImpl

Workers now use settings_svc + prompt_repo directly.
The agent_context module and orchestrator are no longer needed
since process_loop builds AgentRunner without AgentContext.
```

---

## Task 4: Inject StreamHub into WebAdapter for token streaming

WebAdapter needs access to StreamHub so the WebSocket handler can subscribe to StreamEvent deltas and forward them to the client.

**Files:**
- Modify: `crates/core/channels/src/web.rs`

**Step 1: Add StreamHub to WebAdapter and WebAdapterState**

Add `stream_hub` field to `WebAdapter`:

```rust
pub struct WebAdapter {
    sessions:    Arc<DashMap<String, broadcast::Sender<String>>>,
    sink:        Arc<RwLock<Option<Arc<dyn InboundSink>>>>,
    stream_hub:  Arc<RwLock<Option<Arc<rara_kernel::io::stream::StreamHub>>>>,
    shutdown_tx: watch::Sender<bool>,
    shutdown_rx: watch::Receiver<bool>,
}
```

Add `set_stream_hub` method:

```rust
pub fn set_stream_hub(&self, hub: Arc<rara_kernel::io::stream::StreamHub>) {
    *self.stream_hub.blocking_lock() = Some(hub);
}
```

Wait — `RwLock` is sync. Use `tokio::sync::RwLock` but we already use `std::sync::RwLock` for sink... Actually we use `tokio::sync::RwLock` for sink already. Let's match:

```rust
pub async fn set_stream_hub(&self, hub: Arc<rara_kernel::io::stream::StreamHub>) {
    let mut guard = self.stream_hub.write().await;
    *guard = Some(hub);
}
```

Add to `WebAdapterState`:

```rust
struct WebAdapterState {
    sessions:    Arc<DashMap<String, broadcast::Sender<String>>>,
    sink:        Arc<RwLock<Option<Arc<dyn InboundSink>>>>,
    stream_hub:  Arc<RwLock<Option<Arc<rara_kernel::io::stream::StreamHub>>>>,
    shutdown_rx: watch::Receiver<bool>,
}
```

Update `new()` and `router()` to include `stream_hub`.

**Step 2: Add WebEvent variants for streaming**

Extend `WebEvent` to carry stream events:

```rust
pub enum WebEvent {
    Message { content: String },
    Typing,
    Phase { phase: String },
    Error { message: String },
    // New streaming variants
    TextDelta { text: String },
    ReasoningDelta { text: String },
    ToolCallStart { name: String, id: String },
    ToolCallEnd { id: String },
    Progress { stage: String },
    Done,
}
```

**Step 3: Add StreamHub subscription in ws_handler**

After ingest, the WebSocket handler should subscribe to StreamHub for the session. Modify `handle_ws` to:

1. After ingesting a user message, check if `stream_hub` is available
2. Subscribe to the session's streams via `subscribe_session()`
3. Forward `StreamEvent` deltas as `WebEvent` variants through the session broadcast

Add a spawned task in `handle_ws` recv loop, right after successful `s.ingest(raw)`:

```rust
// After successful ingest, spawn a stream forwarder
if let Some(hub) = stream_hub_guard.as_ref() {
    let session_id = rara_kernel::process::SessionId::new(&session_key);
    let sessions = Arc::clone(&sessions);
    let session_key = session_key.clone();
    let hub = Arc::clone(hub);
    tokio::spawn(async move {
        // Poll until stream appears (process_loop opens it asynchronously)
        let mut attempts = 0;
        let subs = loop {
            let s = hub.subscribe_session(&session_id);
            if !s.is_empty() || attempts > 50 {
                break s;
            }
            attempts += 1;
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        };

        for (_stream_id, mut rx) in subs {
            let sessions = Arc::clone(&sessions);
            let session_key = session_key.clone();
            tokio::spawn(async move {
                while let Ok(event) = rx.recv().await {
                    let web_event = match event {
                        StreamEvent::TextDelta(t) => WebEvent::TextDelta { text: t },
                        StreamEvent::ReasoningDelta(t) => WebEvent::ReasoningDelta { text: t },
                        StreamEvent::ToolCallStart { name, id } => WebEvent::ToolCallStart { name, id },
                        StreamEvent::ToolCallEnd { id } => WebEvent::ToolCallEnd { id },
                        StreamEvent::Progress { stage } => WebEvent::Progress { stage },
                    };
                    WebAdapter::broadcast_event(&sessions, &session_key, &web_event);
                }
                // Stream closed — send Done event
                WebAdapter::broadcast_event(&sessions, &session_key, &WebEvent::Done);
            });
        }
    });
}
```

**Step 4: Verify it compiles**

Run: `cargo check -p rara-channels`

**Step 5: Commit**

```
feat(channels): inject StreamHub into WebAdapter for token streaming
```

---

## Task 5: Wire StreamHub to WebAdapter in app startup

Pass the StreamHub from the I/O pipeline to the WebAdapter.

**Files:**
- Modify: `crates/app/src/lib.rs`

**Step 1: Call set_stream_hub after io_pipeline init**

After `init_io_pipeline()` returns, add:

```rust
web_adapter.set_stream_hub(io_pipeline.stream_hub.clone()).await;
```

**Step 2: Verify it compiles**

Run: `cargo check -p rara-app`

**Step 3: Commit**

```
feat(app): wire StreamHub to WebAdapter for real-time streaming
```

---

## Task 6: Update frontend to use WebSocket

Replace the SSE-based `sendMessage` with a WebSocket connection that subscribes to session events.

**Files:**
- Modify: `web/src/pages/Chat.tsx`

**Step 1: Create useWebSocket hook**

In `Chat.tsx`, add a custom hook that manages a WebSocket connection per session:

```typescript
function useSessionWebSocket(
  sessionKey: string | null,
  onEvent: (event: WebEvent) => void,
) {
  const wsRef = useRef<WebSocket | null>(null);

  const send = useCallback((text: string) => {
    if (wsRef.current?.readyState === WebSocket.OPEN) {
      wsRef.current.send(text);
    }
  }, []);

  useEffect(() => {
    if (!sessionKey) return;

    const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
    const host = import.meta.env.VITE_API_URL
      ? new URL(import.meta.env.VITE_API_URL).host
      : window.location.host;
    const url = `${protocol}//${host}/api/v1/kernel/chat/ws?session_key=${encodeURIComponent(sessionKey)}&user_id=web-user`;

    const ws = new WebSocket(url);
    wsRef.current = ws;

    ws.onmessage = (e) => {
      try {
        const event = JSON.parse(e.data) as WebEvent;
        onEvent(event);
      } catch {
        // ignore non-JSON messages
      }
    };

    ws.onerror = () => {
      onEvent({ type: "error", message: "WebSocket connection error" });
    };

    ws.onclose = () => {
      wsRef.current = null;
    };

    return () => {
      ws.close();
      wsRef.current = null;
    };
  }, [sessionKey, onEvent]);

  return { send };
}
```

**Step 2: Define WebEvent type**

Add to types or inline:

```typescript
type WebEvent =
  | { type: "message"; content: string }
  | { type: "typing" }
  | { type: "phase"; phase: string }
  | { type: "error"; message: string }
  | { type: "text_delta"; text: string }
  | { type: "reasoning_delta"; text: string }
  | { type: "tool_call_start"; name: string; id: string }
  | { type: "tool_call_end"; id: string }
  | { type: "progress"; stage: string }
  | { type: "done" };
```

**Step 3: Replace sendMessage in ChatThread**

Replace the SSE `sendMessage` with WebSocket send:

```typescript
const handleWebEvent = useCallback((event: WebEvent) => {
  switch (event.type) {
    case "text_delta":
      setStream((s) => ({ ...s, text: s.text + event.text }));
      break;
    case "reasoning_delta":
      setStream((s) => ({ ...s, reasoning: s.reasoning + event.text }));
      break;
    case "tool_call_start":
      setStream((s) => ({
        ...s,
        activeTools: [...s.activeTools, { id: event.id, name: event.name }],
      }));
      break;
    case "tool_call_end":
      setStream((s) => ({
        ...s,
        activeTools: s.activeTools.filter((t) => t.id !== event.id),
      }));
      break;
    case "typing":
      setStream((s) => ({ ...s, isThinking: true }));
      break;
    case "done":
    case "message":
      setStream(INITIAL_STREAM_STATE);
      queryClient.invalidateQueries({ queryKey: ["chat-messages", sessionKey] });
      queryClient.setQueryData<ChatSession[]>(["chat-sessions"], (old) =>
        old?.map((s) =>
          s.key === sessionKey
            ? { ...s, message_count: s.message_count + 2, updated_at: new Date().toISOString() }
            : s,
        ),
      );
      break;
    case "error":
      setStream((s) => ({ ...s, isStreaming: false, error: event.message }));
      break;
    case "progress":
      setStream((s) => ({ ...s, isThinking: event.stage === "thinking" }));
      break;
  }
}, [sessionKey, queryClient]);

const { send: wsSend } = useSessionWebSocket(sessionKey, handleWebEvent);

const sendMessage = useCallback((text: string, urls?: string[]) => {
  const trimmed = text.trim();
  if (!trimmed || stream.isStreaming || !isOnline) return;

  setInput("");
  setImageUrls([]);

  // Optimistic user message
  const previous = queryClient.getQueryData<ChatMessageData[]>(["chat-messages", sessionKey]);
  const content: ChatContentBlock[] | string = urls?.length
    ? [{ type: "text" as const, text: trimmed }, ...urls.map((url) => ({ type: "image_url" as const, url }))]
    : trimmed;
  const optimisticMsg: ChatMessageData = {
    seq: (previous?.length ?? 0) + 1,
    role: "user",
    content,
    created_at: new Date().toISOString(),
  };
  queryClient.setQueryData<ChatMessageData[]>(
    ["chat-messages", sessionKey],
    (old) => [...(old ?? []), optimisticMsg],
  );

  setStream({ ...INITIAL_STREAM_STATE, isStreaming: true });
  wsSend(trimmed);
}, [stream.isStreaming, isOnline, sessionKey, queryClient, wsSend]);
```

**Step 4: Remove old SSE code**

- Remove `parseSSEChunk` function
- Remove `ChatStreamEvent` import from types
- Remove `abortRef` usage (WebSocket handles its own lifecycle)

**Step 5: Build check**

Run: `cd web && npm run build`

**Step 6: Commit**

```
feat(web): switch chat from SSE to WebSocket via kernel path
```

---

## Task 7: Full workspace verification

**Step 1: Full workspace check**

Run: `cargo check --workspace`

**Step 2: Run tests**

```bash
cargo test -p rara-channels
cargo test -p rara-kernel
cargo test -p rara-workers
cargo test -p rara-backend-admin
```

**Step 3: Frontend build**

```bash
cd web && npm run build
```

**Step 4: Commit any fixes**

```
fix: resolve compilation issues after AgentContext deletion
```

---

## Execution Order & Dependencies

```
Task 1 (helper functions)
    ↓
Task 2 (migrate workers)  ← depends on Task 1
    ↓
Task 3 (delete AgentContext) ← depends on Task 2
    ↓
Task 4 (StreamHub → WebAdapter) ← independent of 1-3
    ↓
Task 5 (wire in app)      ← depends on Task 4
    ↓
Task 6 (frontend WS)      ← depends on Task 5
    ↓
Task 7 (verify all)        ← depends on all above
```

## Follow-up Tasks (NOT in this plan)

- Endpoint registration in WebAdapter (register/unregister in EndpointRegistry on WS connect/disconnect)
- Re-implement compaction/consolidation in kernel process_loop
- Image URL support in WebSocket messages (currently only text)
- WebSocket reconnection / keepalive in frontend
