# Subagent Session Isolation

**Date**: 2026-03-01
**Status**: Draft

## Problem

When a parent agent spawns a subagent, the child inherits the parent's `session_id`. This causes:

1. Child loads the parent's full conversation history (`load_session_messages`)
2. `session_index` is overwritten to point to the child, breaking parent routing
3. Child's messages pollute the parent's session
4. Defeats the purpose of subagents — context isolation to avoid context window overflow

## Design: exec() Model

Following the OS process model, spawn behaves like `posix_spawn()` / `exec()`:

- Each process gets its **own address space** (= own session)
- Child starts with a **clean context** — only system prompt + task input (argv)
- Parent-child communication via **IPC** (existing `AgentHandle` oneshot channel)
- Parent-child relationship tracked via `parent_id` in ProcessTable (already exists)

## Changes

### 1. Remove `session_id` from `KernelEvent::SpawnAgent`

The kernel generates the child's session internally. The caller no longer specifies it.

```rust
// Before
SpawnAgent {
    manifest:   AgentManifest,
    input:      String,
    principal:  Principal,
    session_id: SessionId,       // ← REMOVE
    parent_id:  Option<AgentId>,
    reply_tx:   oneshot::Sender<Result<AgentId>>,
}

// After
SpawnAgent {
    manifest:  AgentManifest,
    input:     String,
    principal: Principal,
    parent_id: Option<AgentId>,
    reply_tx:  oneshot::Sender<Result<AgentId>>,
}
```

### 2. `handle_spawn_agent()` — generate child session

Two paths based on whether this is an external entry or internal spawn:

```rust
async fn handle_spawn_agent(
    &self,
    manifest: AgentManifest,
    input: String,
    principal: Principal,
    parent_id: Option<AgentId>,
    runtimes: &RuntimeTable,
) -> Result<AgentId> {
    let agent_id = AgentId::new();

    // Each process gets its own session.
    let session_id = SessionId::new(format!("agent:{}", agent_id));
    inner.ensure_session(&session_id).await;
    // No load_session_messages — conversation starts empty.
    // Task input arrives as synthetic UserMessage (existing logic).
    let initial_messages = vec![];

    // ... rest unchanged
}
```

### 3. External entry path — IngressPipeline → `handle_user_message()`

The external entry (user message from WebAdapter/Telegram) still needs session routing.
`handle_user_message()` already handles this: it checks `session_index` first, then
spawns a new process if no binding exists. The spawned process gets its own session via
the new `handle_spawn_agent()` logic.

**Key change**: `handle_user_message()` currently passes the inbound message's `session_id`
to `handle_spawn_agent()`. After this change, `handle_spawn_agent()` generates a new
`agent:{id}` session. But the external channel session (e.g. `web:chat123`) still needs
to exist and be bound so that:
- Future messages from the same channel route to the same process
- Outbound messages carry the channel session for egress routing

Solution: keep `session_index` binding using the **inbound session_id** (channel session),
but the process internally uses its own `agent:{id}` session for conversation storage.

This means `AgentProcess` and `ProcessHandle` need two session concepts:

```rust
pub struct AgentProcess {
    pub agent_id: AgentId,
    pub parent_id: Option<AgentId>,
    pub channel_session_id: Option<SessionId>,  // External channel binding (for routing)
    pub session_id: SessionId,                   // Process's own session (for conversation)
    // ...
}
```

- `channel_session_id`: used by `session_index` for message routing, set only for root processes
- `session_id`: always `agent:{agent_id}`, used for conversation persistence

For subagents: `channel_session_id = None` (no external channel binding).
For root agents: `channel_session_id = Some("web:chat123")` (channel routing).

### 4. `ProcessHandle::spawn()` — remove `session_id` from event

```rust
pub async fn spawn(&self, manifest: AgentManifest, input: String) -> Result<AgentHandle> {
    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    let event = KernelEvent::SpawnAgent {
        manifest,
        input,
        principal: self.principal.clone(),
        // session_id removed — kernel generates it
        parent_id: Some(self.agent_id),
        reply_tx,
    };
    // ...
}
```

### 5. `TurnCompleted` — use process session, not channel session

`TurnCompleted` currently carries `session_id`. This should be the process's own session
for conversation persistence, not the channel session. Outbound routing should use
`channel_session_id` when delivering to egress.

### 6. ProcessTable adjustments

- `session_index` keyed by `channel_session_id` (only for root processes with channel binding)
- `find_by_session()` looks up channel binding, unchanged externally
- Subagents are NOT in `session_index` — they're only reachable via `parent_id` / `AgentHandle`

## Summary of Changes

| File | Change |
|------|--------|
| `unified_event.rs` | Remove `session_id` from `SpawnAgent` |
| `process_handle.rs` | Remove `session_id` from `spawn()` call |
| `event_loop.rs` `handle_spawn_agent()` | Generate `agent:{id}` session, empty conversation |
| `event_loop.rs` `handle_user_message()` | Pass `channel_session_id` for routing, `session_id` for process |
| `process/mod.rs` | Add `channel_session_id` to `AgentProcess`, update `session_index` logic |
| Tests | Update spawn tests to verify session isolation |

## Non-Changes

- `AgentHandle` / oneshot reply — already correct
- `parent_id` tracking — already correct
- Session persistence — already correct (ensure_session auto-creates)
- Cancellation cascade — already correct (via parent's CancellationToken)
