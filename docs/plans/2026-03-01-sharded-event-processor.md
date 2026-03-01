# Sharded EventProcessor — Multi-Processor Event Loop

**Date**: 2026-03-01
**Status**: Draft
**Issue**: TBD

## Problem

The kernel event loop (`event_loop.rs:106-138`) is a **single-consumer sequential loop**:

```rust
loop {
    let events = self.event_queue().drain(32).await;
    for (event, wal_id) in events {
        self.handle_event(event, &runtimes).await;  // sequential!
    }
}
```

When multiple agents run concurrently, their Syscalls (GetManifest, ResolveProvider, MemStore, MemRecall, etc.) all queue behind each other. A single busy agent's TurnCompleted handler (which does session persistence + egress delivery) blocks all other agents' Syscalls.

## Design: Sharded Event Processors

Inspired by Go's GMP model where **P (Processor)** runs goroutines independently:

- **G** (goroutine) = `KernelEvent`
- **M** (OS thread) = tokio task
- **P** (processor) = `EventProcessor` (our new abstraction)

### Architecture

```
External callers (ProcessHandle, LLM tasks, IngressPipeline)
          │
          ▼
┌─────────────────────────────────────────────────┐
│              ShardedEventQueue                   │
│                                                  │
│  push(event) {                                   │
│    match classify(event) {                       │
│      Shardable(agent_id) => shards[hash(aid)%N] │
│      Global              => global_queue         │
│    }                                             │
│  }                                               │
└──────┬──────────────────┬──────────┬─────────────┘
       │                  │          │
┌──────▼──────┐  ┌────────▼──┐  ┌───▼─────────┐
│ GlobalProc  │  │ ShardProc0│  │ ShardProcN  │
│             │  │           │  │             │
│ UserMessage │  │ Syscall   │  │ Syscall     │
│ SpawnAgent  │  │ TurnDone  │  │ TurnDone    │
│ Timer       │  │ ChildDone │  │ ChildDone   │
│ Shutdown    │  │ Signal    │  │ Signal      │
│             │  │ Deliver   │  │ Deliver     │
└─────────────┘  └───────────┘  └─────────────┘
```

### Event Classification

Events are classified into two categories:

**Global** (requires complex routing or global state):
- `UserMessage` — 3-path routing logic needs session_index lookup, AgentRegistry access, spawn decisions
- `SpawnAgent` — global semaphore, process table insertion, initial message routing
- `Timer` — not agent-specific
- `Shutdown` — broadcast to all processors

**Shardable** (has a clear `AgentId` affinity):
- `Syscall(*)` — all variants carry `agent_id` (directly or via process lookup)
- `TurnCompleted { agent_id, .. }` — always for a specific agent
- `ChildCompleted { parent_id, .. }` — targets the parent agent
- `SendSignal { target, .. }` — targets a specific agent
- `Deliver(envelope)` — can be processed by any shard (no RuntimeTable access), sharded by session for locality

### Core Types

```rust
/// A single shard queue with its own notification mechanism.
struct ShardQueue {
    queues: [Mutex<VecDeque<KernelEvent>>; 3],  // Priority tiers
    notify: Notify,
    pending: AtomicUsize,
}

/// Sharded event queue — routes events to N shard queues + 1 global queue.
pub struct ShardedEventQueue {
    shards: Vec<ShardQueue>,
    global: ShardQueue,
    num_shards: usize,
    capacity: usize,       // Total across all shards
    total_pending: AtomicUsize,
}

/// A single event processor — drains its assigned queue and processes events.
struct EventProcessor {
    id: usize,
    queue: Arc<ShardQueue>,
    kernel: Arc<Kernel>,      // Actually just &Kernel methods
    runtimes: Arc<RuntimeTable>,
}
```

### Routing Logic

```rust
enum EventTarget {
    Global,
    Shard(u64),  // agent_id hash
}

fn classify(event: &KernelEvent) -> EventTarget {
    match event {
        KernelEvent::UserMessage(_) => EventTarget::Global,
        KernelEvent::SpawnAgent { .. } => EventTarget::Global,
        KernelEvent::Timer { .. } => EventTarget::Global,
        KernelEvent::Shutdown => EventTarget::Global,

        KernelEvent::Syscall(syscall) => {
            let aid = syscall.agent_id();
            EventTarget::Shard(aid.hash())
        }
        KernelEvent::TurnCompleted { agent_id, .. } => {
            EventTarget::Shard(agent_id.hash())
        }
        KernelEvent::ChildCompleted { parent_id, .. } => {
            EventTarget::Shard(parent_id.hash())
        }
        KernelEvent::SendSignal { target, .. } => {
            EventTarget::Shard(target.hash())
        }
        KernelEvent::Deliver(env) => {
            // Hash by session_id for locality
            EventTarget::Shard(env.session_id.hash())
        }
    }
}
```

### New `run_event_loop`

```rust
pub async fn run_event_loop(&self, shutdown: CancellationToken) {
    let runtimes: Arc<RuntimeTable> = Arc::new(DashMap::new());
    let queue = self.sharded_event_queue();
    let num_shards = queue.num_shards();

    let mut handles = Vec::new();

    // Spawn global processor
    let global_proc = EventProcessor::new(0, queue.global(), self, &runtimes);
    handles.push(tokio::spawn(global_proc.run(shutdown.clone())));

    // Spawn shard processors
    for i in 0..num_shards {
        let shard_proc = EventProcessor::new(i + 1, queue.shard(i), self, &runtimes);
        handles.push(tokio::spawn(shard_proc.run(shutdown.clone())));
    }

    // Wait for all processors to finish
    for handle in handles {
        let _ = handle.await;
    }
}
```

### EventQueue Trait Compatibility

`ShardedEventQueue` implements the existing `EventQueue` trait so external callers don't change:

```rust
#[async_trait]
impl EventQueue for ShardedEventQueue {
    async fn push(&self, event: KernelEvent) -> Result<(), BusError> {
        // Route to correct shard
        let target = classify(&event);
        match target {
            EventTarget::Global => self.global.push(event),
            EventTarget::Shard(hash) => {
                let idx = (hash as usize) % self.num_shards;
                self.shards[idx].push(event)
            }
        }
    }

    fn try_push(&self, event: KernelEvent) -> Result<(), BusError> {
        // Same routing, sync version
    }

    // drain() and wait() are for backward compat / tests
    // Processors use internal per-shard drain/wait
}
```

### Syscall.agent_id() Helper

Add a method to extract the agent_id from any Syscall variant:

```rust
impl Syscall {
    pub fn agent_id(&self) -> AgentId {
        match self {
            Self::QueryStatus { target, .. } => *target,
            Self::QueryChildren { parent, .. } => *parent,
            Self::MemStore { agent_id, .. } => *agent_id,
            Self::MemRecall { agent_id, .. } => *agent_id,
            Self::SharedStore { agent_id, .. } => *agent_id,
            Self::SharedRecall { agent_id, .. } => *agent_id,
            Self::CreatePipe { owner, .. } => *owner,
            Self::CreateNamedPipe { owner, .. } => *owner,
            Self::ConnectPipe { connector, .. } => *connector,
            Self::RequiresApproval { .. } => AgentId::nil(), // Global
            Self::RequestApproval { agent_id, .. } => *agent_id,
            Self::GetManifest { agent_id, .. } => *agent_id,
            Self::GetToolRegistry { .. } => AgentId::nil(), // Global
            Self::ResolveProvider { agent_id, .. } => *agent_id,
            Self::PublishEvent { agent_id, .. } => *agent_id,
        }
    }
}
```

For agent-less syscalls (`RequiresApproval`, `GetToolRegistry`), use a nil AgentId that hashes to shard 0 — effectively routing them to a consistent shard.

### Shutdown Sequence

On shutdown:
1. Cancel the shutdown token → all processors exit their main loop
2. Each processor drains its remaining critical events (SendSignal, Shutdown)
3. Global processor handles final cleanup

### Configuration

```rust
pub struct KernelConfig {
    /// Number of shard processors. Default: num_cpus / 2, minimum 1.
    pub num_event_processors: usize,
    /// Per-shard queue capacity. Default: 1024.
    pub shard_queue_capacity: usize,
    // ... existing fields
}
```

## Implementation Steps

### Step 1: Add `Syscall::agent_id()` helper
- Simple method on Syscall enum to extract the primary agent_id
- Add `AgentId::nil()` for agent-less syscalls

### Step 2: Create `ShardQueue` struct
- Extract the per-tier queue logic from `InMemoryEventQueue` into `ShardQueue`
- Same 3-tier priority + Notify + AtomicUsize pattern
- Internal `push()`, `drain()`, `wait()` methods

### Step 3: Create `ShardedEventQueue`
- N `ShardQueue`s + 1 global `ShardQueue`
- `classify()` routing function
- Implement `EventQueue` trait (push routes, drain/wait for compat)
- Add `shard(idx)` and `global()` accessors for processors

### Step 4: Create `EventProcessor`
- Holds: shard queue ref + kernel ref + runtimes ref
- `run()` loop: wait → drain → handle_event for each
- Same shutdown logic (drain critical events)

### Step 5: Rewrite `run_event_loop`
- Create ShardedEventQueue (replace InMemoryEventQueue)
- Spawn N+1 processors
- Await all handles

### Step 6: Boot crate integration
- Update `default_event_queue()` to create `ShardedEventQueue`
- Configurable shard count via KernelConfig

### Step 7: Tests
- Unit tests for ShardQueue (same as existing InMemoryEventQueue tests)
- Unit tests for routing classification
- Unit tests for ShardedEventQueue (events land in correct shards)
- Integration: multi-agent concurrent syscalls processed in parallel

## Risks & Mitigations

| Risk | Mitigation |
|------|------------|
| Same-agent ordering violated | Sharding by agent_id guarantees same-agent events always go to same shard |
| UserMessage routing needs RuntimeTable | Global processor handles UserMessage, has access to shared RuntimeTable |
| Shutdown race | Each processor independently drains critical events |
| Metric counting | `total_pending` AtomicUsize aggregates across shards |
| Backward compat | `ShardedEventQueue` implements `EventQueue` trait |

## Testing Strategy

1. **ShardQueue unit tests** — priority ordering, capacity, drain limits (mirror existing)
2. **classify() tests** — each KernelEvent variant routes correctly
3. **ShardedEventQueue tests** — push routes to correct shard, total_pending accurate
4. **EventProcessor tests** — processes events, respects shutdown
5. **Integration** — spawn 4 agents, verify all syscalls processed concurrently
6. **Regression** — all 236 existing kernel tests must still pass
