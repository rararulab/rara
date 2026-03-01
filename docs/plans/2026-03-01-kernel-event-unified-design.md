# Kernel Event Unified Design

**Date**: 2026-03-01
**Status**: Approved

## Problem

Current kernel architecture has multiple message passing mechanisms:

- `InboundBus` for user messages (single-consumer queue)
- `OutboundBus` for replies (pub/sub broadcast)
- `ProcessMessage` mailbox per process (mpsc channel)
- `PriorityScheduler` for ordering
- `TickLoop` struct as event dispatcher

Internal operations (spawn, kill, pause) bypass the event queue entirely — they're direct method calls through `ProcessOps` or mailbox `Signal` sends. This creates two parallel control paths and makes the system harder to reason about.

## Solution

**Unify all kernel interactions into a single `KernelEvent` enum**, processed by a single event loop (`Kernel::run()`).

### Core Principles

1. **Single event queue** — all interactions are `KernelEvent`
2. **Single event loop** — `Kernel::run()` is the only driver, no process mailboxes
3. **Spawn + callback** — LLM turns spawn async tasks, completion inserts `TurnCompleted` back into the queue
4. **Output unified** — sending replies is also a `KernelEvent::Deliver`
5. **Tiered queue** — Signal > Internal(callbacks) > UserMessage

---

## Design

### 1. KernelEvent Enum

```rust
pub enum KernelEvent {
    // === Input: from external sources ===
    UserMessage(InboundMessage),

    // === Process control ===
    SpawnAgent {
        manifest: AgentManifest,
        input: String,
        principal: Principal,
        session_id: SessionId,
        parent_id: Option<AgentId>,
        reply_tx: oneshot::Sender<Result<AgentId>>,
    },
    SendSignal {
        target: AgentId,
        signal: Signal,
    },

    // === Internal callbacks: from async task completion ===
    TurnCompleted {
        agent_id: AgentId,
        session_id: SessionId,
        result: Result<AgentTurnResult, String>,
        in_reply_to: MessageId,
        user: UserId,
    },
    ChildCompleted {
        parent_id: AgentId,
        child_id: AgentId,
        result: AgentResult,
    },

    // === Output ===
    Deliver(OutboundEnvelope),

    // === System ===
    Timer { name: String, payload: Value },
    Shutdown,
}
```

### 2. EventQueue — Tiered Priority Queue

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    Critical = 0,  // Signal, Shutdown
    Normal = 1,    // TurnCompleted, ChildCompleted, Deliver
    Low = 2,       // UserMessage, SpawnAgent, Timer
}

pub struct EventQueue {
    queues: [Mutex<VecDeque<KernelEvent>>; 3],
    notify: Notify,
    pending: AtomicUsize,
    capacity: usize,
}

impl EventQueue {
    pub async fn push(&self, event: KernelEvent) -> Result<(), BusError>;
    pub fn try_push(&self, event: KernelEvent) -> Result<(), BusError>;
    pub async fn drain(&self, max: usize) -> Vec<KernelEvent>;
    pub async fn wait(&self);
}

impl KernelEvent {
    fn priority(&self) -> Priority {
        match self {
            Self::SendSignal { .. } | Self::Shutdown => Priority::Critical,
            Self::TurnCompleted { .. }
            | Self::ChildCompleted { .. }
            | Self::Deliver(_) => Priority::Normal,
            Self::UserMessage(_)
            | Self::SpawnAgent { .. }
            | Self::Timer { .. } => Priority::Low,
        }
    }
}
```

Replaces `InboundBus` + `PriorityScheduler`. Priority is auto-inferred from event type.

### 3. Kernel::run() — Unified Event Loop

```rust
impl Kernel {
    pub async fn run(&self, shutdown: CancellationToken) {
        loop {
            tokio::select! {
                _ = self.event_queue.wait() => {
                    let events = self.event_queue.drain(self.batch_size).await;
                    for event in events {
                        self.handle(event).await;
                    }
                }
                _ = shutdown.cancelled() => {
                    self.drain_critical().await;
                    break;
                }
            }
        }
    }

    async fn handle(&self, event: KernelEvent) {
        match event {
            KernelEvent::UserMessage(msg) => self.handle_user_message(msg).await,
            KernelEvent::SpawnAgent { .. } => { /* spawn process, reply via oneshot */ }
            KernelEvent::SendSignal { target, signal } => self.handle_signal(target, signal).await,
            KernelEvent::TurnCompleted { .. } => self.handle_turn_completed(..).await,
            KernelEvent::ChildCompleted { .. } => self.handle_child_completed(..).await,
            KernelEvent::Deliver(envelope) => self.deliver(envelope).await,
            KernelEvent::Timer { .. } => { /* handle timer */ }
            KernelEvent::Shutdown => self.graceful_shutdown().await,
        }
    }
}
```

Replaces `TickLoop` struct. Lives directly on `Kernel`.

### 4. AgentProcess — Pure State Record (No Mailbox)

```rust
pub struct AgentProcess {
    pub agent_id: AgentId,
    pub session_id: SessionId,
    pub principal: Principal,
    pub manifest: AgentManifest,
    pub state: ProcessState,
    pub parent_id: Option<AgentId>,

    // Previously local variables in process_loop, now kernel-managed
    pub conversation: Vec<ChatMessage>,
    pub turn_cancel: CancellationToken,
    pub paused: bool,
    pub pause_buffer: Vec<KernelEvent>,
    pub metrics: ProcessMetrics,
}
```

Process is no longer a long-lived tokio task. Kernel manages state directly.

When `handle_user_message` fires:
1. Find or create process for session
2. If paused or already Running → buffer event in `pause_buffer`
3. Set state to `Running`, spawn async task for LLM turn
4. Task completes → pushes `TurnCompleted` back to EventQueue

When `handle_turn_completed` fires:
1. Persist result, update conversation history
2. Push `Deliver` event for reply
3. Set state to `Waiting`
4. Pop next event from `pause_buffer` if any, re-inject into queue

### 5. External Interface Changes

**IngressPipeline**: holds `Arc<EventQueue>` instead of `Arc<dyn InboundBus>`. Pushes `KernelEvent::UserMessage`.

**ScopedKernelHandle**: holds `Arc<EventQueue>`. `ProcessOps::spawn()` pushes `SpawnAgent` + waits on oneshot. `kill()`/`interrupt()` push `SendSignal` (fire-and-forget via `try_push`).

**Egress**: no longer subscribes to OutboundBus. `Kernel::deliver()` calls Egress directly when handling `KernelEvent::Deliver`.

**StreamHub**: preserved as-is. Ephemeral real-time deltas bypass the event queue.

---

## Deletion List

```
Delete:
├── InboundBus trait + InMemoryInboundBus     → replaced by EventQueue
├── OutboundBus trait + InMemoryOutboundBus   → replaced by KernelEvent::Deliver
├── OutboundSubscriber trait                   → Egress called directly by kernel
├── TickLoop struct (tick.rs)                  → replaced by Kernel::run()
├── PriorityScheduler                          → EventQueue has built-in tiering
├── ProcessMessage enum                        → replaced by KernelEvent
├── process mailbox (mpsc channel)             → deleted
├── AgentHandle::mailbox field                 → deleted
├── process_loop function                      → logic split into Kernel::handle_* methods
├── Egress::run() subscribe loop               → changed to Kernel::deliver() direct call
└── boot/bus.rs InboundBus/OutboundBus factory → replaced by EventQueue factory

Preserve:
├── StreamHub                    — ephemeral delta bypass, not queued
├── IngressPipeline              — holds EventQueue instead of InboundBus
├── ScopedKernelHandle           — pushes events instead of direct calls
├── ProcessTable                 — preserved, AgentProcess becomes pure state
├── Egress + EgressAdapter       — dispatch logic preserved, no longer self-subscribing
├── EndpointRegistry             — unchanged
└── KernelHandle traits          — interface unchanged, implementation changed
```

## Migration Steps

1. Add `KernelEvent` + `EventQueue` — pure new code, no impact
2. Extend `AgentProcess` — add conversation, turn_cancel, pause_buffer fields
3. Implement `Kernel::run()` + `handle_*` methods — new event loop
4. Rewire `IngressPipeline` — hold EventQueue
5. Rewire `ScopedKernelHandle` — push events instead of direct calls
6. Rewire `Egress` — from subscribe model to kernel direct call
7. Delete old code — TickLoop, InboundBus, OutboundBus, process_loop, PriorityScheduler
8. Update boot layer — factory functions for new structure
9. Update tests — all bus-based tests migrate to EventQueue
