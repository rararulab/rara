# Kernel I/O Bus Architecture — OS-Inspired Message Pipeline

> Date: 2026-02-28
> Status: Design Complete
> Supersedes: ChatService + ChannelBridge model

## Motivation

Current architecture has `ChannelAdapter → ChannelBridge.dispatch() → ChatService → AgentRunner`, which:

1. **ChatService** is a God object (session, messages, LLM, context compaction)
2. **ChannelBridge** couples channels to chat service — no decoupling
3. **No cross-channel sync** — TG message can't appear on Web
4. **Push-based** — malicious users can overwhelm the system

The OS kernel analogy gives us a clean I/O model: channels are devices, Kernel pulls from a message bus at its own pace.

## Architecture Overview

```
                    ┌──────────────────────────────────────┐
                    │            User Space                │
                    │                                      │
  ┌──────────┐     │  ┌──────────┐  ┌──────────┐          │
  │ Telegram │◄────┼──┤ TG       │  │ Web      │◄──HTTP── │
  │ Bot API  │────►┼──┤ Adapter  │  │ Adapter  │──►       │
  └──────────┘     │  └────┬─────┘  └────┬─────┘          │
                   │       │             │                 │
                   │       ▼             ▼                 │
                   │  ┌──────────────────────┐             │
                   │  │   IngressPipeline    │             │
                   │  │  (InboundSink impl)  │             │
                   │  └─────────┬────────────┘             │
                   │            ▼                          │
                   │  ┌──────────────────────┐             │
                   │  │    InboundBus        │ + notify    │
                   │  │  (trait, pull model) │─────┐       │
                   │  └──────────────────────┘     │       │
                   │                               │       │
                   ├───────────────────────────────┼───────┤
                   │         Kernel Space          │       │
                   │                               ▼       │
                   │  ┌───────────────────────────────┐    │
                   │  │          Kernel                │    │
                   │  │  ┌──────────┐ ┌─────────────┐ │    │
                   │  │  │ Tick     │ │ Session     │ │    │
                   │  │  │ Loop     │ │ Scheduler   │ │    │
                   │  │  └────┬─────┘ └─────────────┘ │    │
                   │  │       │                        │    │
                   │  │       ▼                        │    │
                   │  │  ┌─────────────┐               │    │
                   │  │  │ Agent      │──► StreamHub   │    │
                   │  │  │ Executor   │   (ephemeral)  │    │
                   │  │  └─────┬──────┘                │    │
                   │  │        │                        │    │
                   │  │        ▼                        │    │
                   │  │  ┌──────────────────────┐       │    │
                   │  │  │   OutboundBus        │       │    │
                   │  │  │  (pub/sub model)     │       │    │
                   │  │  └──────────┬───────────┘       │    │
                   │  └─────────────┼───────────────────┘    │
                   │                │                        │
                   ├────────────────┼────────────────────────┤
                   │                ▼       User Space       │
                   │  ┌──────────────────────────┐           │
                   │  │        Egress            │           │
                   │  │  (subscribe + deliver)   │           │
                   │  └───┬──────────┬───────────┘           │
                   │      │          │                       │
                   │      ▼          ▼                       │
                   │  ┌──────┐  ┌──────┐                     │
                   │  │ TG   │  │ Web  │                     │
                   │  │ Out  │  │ Out  │                     │
                   │  └──────┘  └──────┘                     │
                   └─────────────────────────────────────────┘
```

## Core Principles

1. **Kernel pulls, never pushed** — tick loop + `wait_for_messages()` wakeup
2. **Asymmetric buses** — InboundBus is single-consumer queue; OutboundBus is pub/sub broadcast
3. **Two-layer outbound** — OutboundBus for durable final messages; StreamHub for ephemeral deltas
4. **User-level broadcast** — same user's all connected channels receive final responses
5. **ChatService eliminated** — session/message/LLM absorbed into Kernel
6. **Session-serial execution** — same session messages execute one-at-a-time via SessionScheduler

## Component Design

### 1. InboundBus — Single Consumer Queue

```rust
#[async_trait]
pub trait InboundBus: Send + Sync + 'static {
    /// Ingress writes a message
    async fn publish(&self, msg: InboundMessage) -> Result<(), BusError>;

    /// Kernel tick pulls batch (exclusive consume, removes on read)
    async fn drain(&self, max: usize) -> Vec<InboundMessage>;

    /// Block until new messages available (encapsulates wakeup mechanism)
    async fn wait_for_messages(&self);

    /// Current backlog count (monitoring)
    fn pending_count(&self) -> usize;
}

pub enum BusError {
    Full,
    Internal(String),
}
```

Initial implementation: `InMemoryBus<InboundMessage>` backed by `Mutex<VecDeque>` + `tokio::sync::Notify`.

### 2. OutboundBus — Pub/Sub Broadcast

```rust
#[async_trait]
pub trait OutboundBus: Send + Sync + 'static {
    /// Kernel publishes final response
    async fn publish(&self, msg: OutboundEnvelope) -> Result<(), BusError>;

    /// Each Egress instance gets independent subscriber
    fn subscribe(&self) -> Box<dyn OutboundSubscriber>;
}

#[async_trait]
pub trait OutboundSubscriber: Send + 'static {
    async fn recv(&mut self) -> Option<OutboundEnvelope>;
}
```

Initial implementation: `InMemoryBroadcastBus` backed by `tokio::sync::broadcast`.

### 3. Message Types

#### InboundMessage

```rust
pub struct InboundMessage {
    pub id: MessageId,                        // ULID
    pub source: ChannelSource,                // Platform details (first-class fields)
    pub user: UserId,                         // Unified user ID
    pub session_id: SessionId,
    pub content: MessageContent,              // Text | Multimodal
    pub reply_context: Option<ReplyContext>,   // Thread/reply/interaction info
    pub timestamp: jiff::Timestamp,
    pub metadata: HashMap<String, Value>,     // True extension fields only
}

/// First-class platform source fields (not stuffed in metadata)
pub struct ChannelSource {
    pub channel_type: ChannelType,
    pub platform_message_id: Option<String>,  // Dedup / reply mapping
    pub platform_user_id: String,
    pub platform_chat_id: Option<String>,     // Platform thread/chat
}

/// Enough info for Egress to reply correctly
pub struct ReplyContext {
    pub thread_id: Option<String>,
    pub reply_to_platform_msg_id: Option<String>,
    pub interaction_type: InteractionType,
}

pub enum InteractionType {
    Message,
    Command(String),
    Callback(String),
}
```

#### OutboundEnvelope

```rust
pub struct OutboundEnvelope {
    pub id: MessageId,
    pub in_reply_to: MessageId,
    pub user: UserId,
    pub session_id: SessionId,
    pub routing: OutboundRouting,
    pub payload: OutboundPayload,
    pub timestamp: jiff::Timestamp,
}

pub enum OutboundRouting {
    /// Broadcast to all connected endpoints for this user
    BroadcastAll,
    /// Broadcast but exclude source channel (prevent echo)
    BroadcastExcept { exclude: ChannelType },
    /// Send to specific channels only
    Targeted { channels: Vec<ChannelType> },
}

pub enum OutboundPayload {
    Reply {
        content: MessageContent,
        attachments: Vec<Attachment>,
    },
    Progress {
        stage: String,
        detail: Option<String>,
    },
    StateChange {
        event_type: String,
        data: Value,
    },
    Error {
        code: String,
        message: String,
    },
}
```

### 4. Ingress Pipeline

Responsibilities split into focused components:

```
RawPlatformMessage (from Adapter)
  → IngressPipeline (orchestrator, implements InboundSink)
    → IdentityResolver (channel_type + platform_user_id + chat_id → UserId)
    → SessionResolver  (user + channel context → SessionId, supports cross-channel sharing)
    → InboundBus.publish()
```

```rust
/// Adapter's only interface — thin, single-responsibility
#[async_trait]
pub trait InboundSink: Send + Sync + 'static {
    async fn ingest(&self, raw: RawPlatformMessage) -> Result<(), IngestError>;
}

pub enum IngestError {
    SystemBusy,
    IdentityResolutionFailed(String),
    Internal(String),
}

/// Simplified ChannelAdapter — pure I/O device
#[async_trait]
pub trait ChannelAdapter: Send + Sync + 'static {
    fn channel_type(&self) -> ChannelType;
    async fn start(&self, sink: Arc<dyn InboundSink>) -> Result<()>;
    async fn send(&self, msg: PlatformOutbound) -> Result<()>;
    async fn stop(&self) -> Result<()>;
}

/// Orchestrates identity resolution, session resolution, bus publish
pub struct IngressPipeline {
    identity_resolver: Arc<dyn IdentityResolver>,
    session_resolver: Arc<dyn SessionResolver>,
    publisher: Arc<dyn InboundBus>,
}

#[async_trait]
pub trait IdentityResolver: Send + Sync + 'static {
    async fn resolve(
        &self,
        channel_type: ChannelType,
        platform_user_id: &str,
        platform_chat_id: Option<&str>,
    ) -> Result<UserId, IngestError>;
}

#[async_trait]
pub trait SessionResolver: Send + Sync + 'static {
    async fn resolve(
        &self,
        user: &UserId,
        channel_type: ChannelType,
        platform_chat_id: Option<&str>,
    ) -> Result<SessionId, IngestError>;
}
```

### 5. StreamHub — Ephemeral Real-time Events

Two-layer outbound design:
- **OutboundBus**: durable final messages (Reply, Error, StateChange)
- **StreamHub**: ephemeral incremental events (token deltas, tool progress)

```rust
pub type StreamId = String; // ULID, unique per agent run

pub struct StreamHub {
    streams: DashMap<StreamId, StreamEntry>,
    capacity: usize,
}

struct StreamEntry {
    session_id: SessionId,
    tx: broadcast::Sender<StreamEvent>,
}

impl StreamHub {
    /// Open a new stream for an agent run (returns handle with unique stream_id)
    pub fn open(&self, session_id: SessionId) -> StreamHandle;

    /// Close by stream_id (precise, won't close other streams on same session)
    pub fn close(&self, stream_id: &StreamId);

    /// Subscribe to all active streams for a session
    pub fn subscribe_session(&self, session_id: &SessionId)
        -> Vec<(StreamId, broadcast::Receiver<StreamEvent>)>;

    /// Subscribe to all active streams for a user
    pub fn subscribe_user(&self, user_id: &UserId)
        -> Vec<(StreamId, SessionId, broadcast::Receiver<StreamEvent>)>;
}

/// Agent holds this — Drop auto-closes
pub struct StreamHandle {
    stream_id: StreamId,
    tx: broadcast::Sender<StreamEvent>,
}

/// Only incremental/transient events — no Done/Error (those go through OutboundBus)
pub enum StreamEvent {
    TextDelta(String),
    ReasoningDelta(String),
    ToolCallStart { name: String, id: String },
    ToolCallEnd { id: String },
    Progress { stage: String },
}
```

### 6. Egress — Output Delivery

```rust
pub struct Egress {
    adapters: HashMap<ChannelType, Arc<dyn ChannelAdapter>>,
    endpoints: Arc<EndpointRegistry>,
    outbound_sub: Box<dyn OutboundSubscriber>,
    stream_hub: Arc<StreamHub>,
}

/// Concrete deliverable target (not coarse ChannelType)
pub struct Endpoint {
    pub channel_type: ChannelType,
    pub address: EndpointAddress,
}

pub enum EndpointAddress {
    Telegram { chat_id: i64, thread_id: Option<i64> },
    Web { connection_id: String },
    Cli { session_id: String },
}

/// Tracks per-user active endpoints
pub struct EndpointRegistry {
    connections: DashMap<UserId, HashSet<Endpoint>>,
}

/// What Adapter.send() receives
pub enum PlatformOutbound {
    Reply {
        session_key: String,
        content: String,
        attachments: Vec<Attachment>,
        reply_context: Option<ReplyContext>,
    },
    StreamChunk {
        session_key: String,
        delta: String,
        edit_target: Option<String>,
    },
    Progress {
        session_key: String,
        text: String,
    },
}
```

Egress delivery rules:
- **Durable messages** (Reply/Error): if user offline, write to OutboxStore for later delivery
- **Ephemeral messages** (Progress/StreamChunk): only deliver to online endpoints
- **Per-delivery timeout**: 10s per adapter.send()
- **Concurrent fan-out**: parallel delivery to all target endpoints

### 7. Kernel Tick Loop

```rust
pub struct KernelInner {
    // Process management
    process_table: ProcessTable,
    global_semaphore: Arc<Semaphore>,
    manifest_loader: ManifestLoader,

    // Scheduling
    session_scheduler: SessionScheduler,

    // I/O Buses
    inbound_bus: Arc<dyn InboundBus>,
    outbound_bus: Arc<dyn OutboundBus>,
    outbox_store: Arc<dyn OutboxStore>,
    stream_hub: Arc<StreamHub>,

    // Infrastructure (existing)
    llm_provider: LlmProviderLoaderRef,
    tool_registry: Arc<ToolRegistry>,
    memory: Arc<dyn Memory>,
    event_bus: Arc<dyn EventBus>,
    guard: Arc<dyn Guard>,
    session_manager: SessionManager,
}

impl Kernel {
    /// Main loop — woken by InboundBus, no polling fallback
    pub async fn run(&self, shutdown: CancellationToken) {
        loop {
            tokio::select! {
                _ = self.inner.inbound_bus.wait_for_messages() => {
                    self.tick().await;
                }
                _ = shutdown.cancelled() => {
                    self.shutdown().await;
                    break;
                }
            }
        }
    }

    async fn tick(&self) {
        let messages = self.inner.inbound_bus.drain(32).await;
        for msg in messages {
            self.dispatch(msg).await;
        }
    }

    async fn dispatch(&self, msg: InboundMessage) {
        match self.inner.session_scheduler.schedule(msg) {
            ScheduleResult::Ready(msg) => self.spawn_agent(msg),
            ScheduleResult::Queued => { /* waiting */ }
            ScheduleResult::Rejected => {
                // session queue full — reply "system busy" via OutboundBus
            }
        }
    }
}
```

### 8. SessionScheduler — Per-Session Serial Execution

```rust
pub struct SessionScheduler {
    slots: DashMap<SessionId, SessionSlot>,
    max_pending_per_session: usize,
}

struct SessionSlot {
    running: bool,
    pending: VecDeque<InboundMessage>,
}

pub enum ScheduleResult {
    Ready(InboundMessage),
    Queued,
    Rejected,
}

impl SessionScheduler {
    pub fn schedule(&self, msg: InboundMessage) -> ScheduleResult;

    /// Release current run, return next if any.
    /// Cleans up empty slots to prevent memory leak.
    pub fn release_and_next(&self, session_id: &SessionId) -> Option<InboundMessage>;
}
```

### 9. AgentExecutor — Extracted from Kernel

```rust
pub struct AgentExecutor {
    inner: Arc<KernelInner>,
}

impl AgentExecutor {
    pub async fn run(&self, msg: InboundMessage) {
        // 1. Acquire global semaphore (graceful on shutdown)
        // 2. Register in ProcessTable
        // 3. Load history (does NOT include current message)
        // 4. Persist current user message
        // 5. Open StreamHandle (keyed by stream_id, not session_id)
        // 6. Run AgentRunner streaming, bridge RunnerEvent → StreamEvent
        // 7. Close stream
        // 8. On success: persist reply, reliable_publish_reply() → OutboundBus (fallback: OutboxStore)
        //    On failure: reliable_publish_error() → OutboundBus (fallback: OutboxStore)
        // 9. Update ProcessState (Completed / Failed)
        // 10. Release semaphore
        // 11. release_and_next() — if next msg, re-publish to InboundBus for unified scheduling
    }
}
```

### 10. OutboxStore — Durable Message Guarantee

```rust
#[async_trait]
pub trait OutboxStore: Send + Sync + 'static {
    async fn append(&self, envelope: OutboundEnvelope) -> Result<()>;
    async fn drain_pending(&self, max: usize) -> Vec<OutboundEnvelope>;
    async fn mark_delivered(&self, id: &MessageId) -> Result<()>;
}
```

A background `OutboxDrainer` periodically polls `drain_pending()` and re-publishes to OutboundBus.

### 11. SessionManager — Absorbed from ChatService

```rust
pub struct SessionManager {
    session_repo: Arc<dyn SessionRepository>,
    message_repo: Arc<dyn MessageRepository>,
}

impl SessionManager {
    pub async fn ensure_session(&self, id: &SessionId, user: &UserId) -> Session;
    pub async fn get_history(&self, id: &SessionId) -> Vec<ChatMessage>;
    pub async fn append_message(&self, id: &SessionId, msg: &InboundMessage);
    pub async fn append_assistant_message(&self, id: &SessionId, content: &str);
}
```

## Boot Layer (`rara-boot`)

Boot only cares about Kernel components:

```rust
// New modules in rara-boot
pub mod bus;     // default_inbound_bus(), default_outbound_bus()
pub mod stream;  // default_stream_hub()
pub mod outbox;  // default_outbox_store()

// Existing modules (unchanged)
pub mod components;  // default_memory(), default_event_bus(), default_guard()
pub mod manifests;   // load_default_manifests()
pub mod skills;      // init_skill_registry()
pub mod mcp;         // init_mcp_manager()
```

App layer (`rara-app` / `rara-cmd`) handles:
- Creating adapters (TG, Web)
- Creating IngressPipeline
- Creating Egress + EndpointRegistry
- Spawning Kernel.run(), Egress.run(), OutboxDrainer
- Starting HTTP server

## What Gets Deleted

| Component | Fate |
|-----------|------|
| `ChatService` | **Deleted** — session mgmt → SessionManager, LLM → AgentExecutor |
| `ChatServiceBridge` | **Deleted** — replaced by IngressPipeline |
| `ChannelBridge` trait | **Deleted** — replaced by InboundSink |
| `ChannelRouter` trait | **Deleted** — routing moves to SessionScheduler + OutboundRouting |

## What Gets Kept

| Component | Status |
|-----------|--------|
| `ChannelAdapter` trait | **Simplified** — only `start(sink)`, `send(outbound)`, `stop()` |
| `AgentRunner` | **Kept** — core LLM loop, used by AgentExecutor |
| `ProcessTable` | **Kept** — agent lifecycle tracking |
| `StreamEvent` variants | **Kept** — reused in StreamHub (minus Done/Error) |
| `Memory`, `Guard`, `EventBus` traits | **Kept** |

## Key Design Decisions

1. **InboundBus ≠ OutboundBus** — different consumption models (queue vs pub/sub)
2. **StreamHub keyed by StreamId** (ULID per run), not SessionId — supports concurrent runs
3. **StreamEvent has no Done/Error** — those are OutboundBus's responsibility
4. **Endpoint, not ChannelType** — concrete delivery addresses for reliable fan-out
5. **Offline delivery via OutboxStore** — durable messages never lost
6. **release_and_next() re-publishes to InboundBus** — unified scheduling path
7. **SessionScheduler cleans empty slots** — prevents memory leak
8. **Boot only creates Kernel components** — app layer handles adapters/egress/HTTP
