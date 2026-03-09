# Background Tool Continuations Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Allow long-running tools to continue in the background without blocking user input, then resume the same conversation when the tool completes.

**Architecture:** Split tool execution into two classes: inline tools (current behavior) and detachable background tools. A detachable tool starts a kernel-managed background run, immediately returns a running handle to the agent/session, leaves the session `Ready` for new input, and later injects a completion event back into the same session so the agent can continue from the result.

**Tech Stack:** Rust, Tokio, rara kernel event queue, session table, StreamHub, Telegram/Web channel adapters.

---

### Task 1: Define background-tool primitives

**Files:**
- Modify: `crates/kernel/src/tool.rs`
- Modify: `crates/kernel/src/io.rs`
- Modify: `crates/kernel/src/event.rs`
- Test: `crates/kernel/src/tool.rs`

**Step 1: Write the failing test**

Add a unit test proving a tool can declare a detachable execution mode and that the resulting metadata is preserved in the registry.

```rust
#[test]
fn tool_capability_marks_detachable_tools() {
    let caps = ToolCapabilities {
        execution_mode: ToolExecutionMode::Detachable,
        status_label: Some("background".into()),
    };
    assert!(matches!(caps.execution_mode, ToolExecutionMode::Detachable));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p rara-kernel tool_capability_marks_detachable_tools -- --nocapture`

Expected: fail because `ToolCapabilities` / `ToolExecutionMode` do not exist yet.

**Step 3: Write minimal implementation**

Add new primitives:

```rust
pub enum ToolExecutionMode {
    Inline,
    Detachable,
}

pub struct ToolCapabilities {
    pub execution_mode: ToolExecutionMode,
    pub status_label: Option<String>,
}
```

Extend `AgentTool` with a default capability hook:

```rust
fn capabilities(&self) -> ToolCapabilities {
    ToolCapabilities {
        execution_mode: ToolExecutionMode::Inline,
        status_label: None,
    }
}
```

Add kernel I/O/event types for background lifecycle:

```rust
pub enum StreamEvent {
    BackgroundToolStarted { id: String, name: String, summary: Option<String> },
    BackgroundToolFinished { id: String, success: bool, summary: String },
    // existing variants...
}

pub enum KernelEvent {
    BackgroundToolCompleted { session_key: SessionKey, run_id: String, ... },
}
```

**Step 4: Run test to verify it passes**

Run: `cargo test -p rara-kernel tool_capability_marks_detachable_tools -- --nocapture`

Expected: PASS

**Step 5: Commit**

```bash
git add crates/kernel/src/tool.rs crates/kernel/src/io.rs crates/kernel/src/event.rs
git commit -m "feat(kernel): add background tool capability primitives"
```

### Task 2: Track background runs in session state

**Files:**
- Modify: `crates/kernel/src/session.rs`
- Modify: `crates/kernel/src/kernel.rs`
- Test: `crates/kernel/src/session.rs`

**Step 1: Write the failing test**

Add a unit test proving a session can register, inspect, and complete a background tool run without changing the session lifecycle to `Active`.

```rust
#[test]
fn session_tracks_background_runs_independently_of_turn_state() {
    let table = SessionTable::new();
    let key = SessionKey::new();
    // insert minimal session, register background run, assert state remains Ready
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p rara-kernel session_tracks_background_runs_independently_of_turn_state -- --nocapture`

Expected: fail because background-run storage does not exist yet.

**Step 3: Write minimal implementation**

Add session-scoped tracking:

```rust
pub struct BackgroundToolRun {
    pub id: String,
    pub tool_name: String,
    pub started_at: Timestamp,
    pub status: BackgroundToolStatus,
}

pub enum BackgroundToolStatus {
    Running,
    Succeeded { summary: String },
    Failed { error: String },
}
```

Extend `Session` / `SessionTable` with:

```rust
pub background_runs: Vec<BackgroundToolRun>;
pub fn insert_background_run(...)
pub fn complete_background_run(...)
pub fn background_runs(...)
```

Keep these separate from `pause_buffer` and `state`.

**Step 4: Run test to verify it passes**

Run: `cargo test -p rara-kernel session_tracks_background_runs_independently_of_turn_state -- --nocapture`

Expected: PASS

**Step 5: Commit**

```bash
git add crates/kernel/src/session.rs crates/kernel/src/kernel.rs
git commit -m "feat(kernel): track background tool runs per session"
```

### Task 3: Detach eligible tools from the inline agent loop

**Files:**
- Modify: `crates/kernel/src/agent.rs`
- Modify: `crates/kernel/src/kernel.rs`
- Modify: `crates/kernel/src/event.rs`
- Test: `crates/kernel/src/agent.rs`

**Step 1: Write the failing test**

Add a regression test proving a detachable tool returns an immediate “started” result instead of blocking the turn.

```rust
#[tokio::test]
async fn detachable_tool_returns_running_handle_without_waiting_for_completion() {
    // dummy tool waits on Notify forever
    // execute agent tool phase
    // assert returned result contains {"status":"running","run_id":"..."}
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p rara-kernel detachable_tool_returns_running_handle_without_waiting_for_completion -- --nocapture`

Expected: fail because the current code waits in `join_all(tool_futures).await`.

**Step 3: Write minimal implementation**

Replace the hard wait path for detachable tools:

```rust
match tool.capabilities().execution_mode {
    ToolExecutionMode::Inline => { /* current path */ }
    ToolExecutionMode::Detachable => {
        let run_id = register_background_tool_run(...);
        spawn_background_tool_task(...);
        immediate_results.push(json!({
            "status": "running",
            "run_id": run_id,
            "tool": name,
        }));
    }
}
```

The background task must:
- execute the tool outside the interactive turn
- persist completion/failure
- push `KernelEvent::BackgroundToolCompleted`

The interactive turn must:
- append a tool result message containing the running handle
- finish normally so the session returns to `Ready`

**Step 4: Run test to verify it passes**

Run: `cargo test -p rara-kernel detachable_tool_returns_running_handle_without_waiting_for_completion -- --nocapture`

Expected: PASS

**Step 5: Commit**

```bash
git add crates/kernel/src/agent.rs crates/kernel/src/kernel.rs crates/kernel/src/event.rs
git commit -m "feat(kernel): detach long-running tools from interactive turns"
```

### Task 4: Re-inject background completion into the same conversation

**Files:**
- Modify: `crates/kernel/src/kernel.rs`
- Modify: `crates/kernel/src/io.rs`
- Modify: `crates/kernel/src/memory/service.rs`
- Test: `crates/app/tests/real_tape_flow.rs`

**Step 1: Write the failing test**

Add an integration test proving that after a detachable tool finishes, the same session receives a synthetic follow-up message and can continue the dialogue.

```rust
#[tokio::test]
async fn background_tool_completion_reenters_original_session() {
    // start session
    // trigger detachable tool
    // complete background task
    // assert next turn trace appears on same session key
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p rara-app background_tool_completion_reenters_original_session -- --nocapture`

Expected: fail because no completion re-entry exists yet.

**Step 3: Write minimal implementation**

Handle `BackgroundToolCompleted` by appending tape/system state and injecting a synthetic message:

```rust
let synthetic = InboundMessage::synthetic(
    format!("[background tool completed] {summary}"),
    user.clone(),
    session_key,
);
self.event_queue.try_push(KernelEventEnvelope::user_message(synthetic))?;
```

Persist a durable event record so recovery and observability work:

```rust
tape.append_event(tape_name, "background_tool_completed", json!({...})).await?;
```

**Step 4: Run test to verify it passes**

Run: `cargo test -p rara-app background_tool_completion_reenters_original_session -- --nocapture`

Expected: PASS

**Step 5: Commit**

```bash
git add crates/kernel/src/kernel.rs crates/kernel/src/io.rs crates/kernel/src/memory/service.rs crates/app/tests/real_tape_flow.rs
git commit -m "feat(kernel): resume sessions after background tool completion"
```

### Task 5: Channel UX for pending and completed background tools

**Files:**
- Modify: `crates/channels/src/telegram/adapter.rs`
- Modify: `crates/channels/src/web.rs`
- Modify: `web/src/pages/Chat.tsx`
- Test: `crates/channels/src/telegram/adapter.rs`

**Step 1: Write the failing test**

Add channel tests proving background tool start/completion surface as status updates rather than blocking the live reply stream.

```rust
#[test]
fn typing_refresh_continues_while_background_tools_are_running() {
    // assert helper returns true for running tool state
}
```

```tsx
it("renders background tool status separately from streaming assistant text", () => {
  // assert pending background tool badge persists
})
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p rara-channels telegram::adapter::tests -- --nocapture`

Run: `cd web && npm test -- Chat`

Expected: fail because the channels do not yet understand background tool lifecycle events.

**Step 3: Write minimal implementation**

Telegram:
- keep the progress line visible for running background tools
- send completion notification when `BackgroundToolFinished` arrives

Web:
- render background tool chips separate from current token stream
- avoid marking the whole thread as blocked while a background tool is running

**Step 4: Run tests to verify they pass**

Run: `cargo test -p rara-channels telegram::adapter::tests -- --nocapture`

Run: `cd web && npm test -- Chat`

Expected: PASS

**Step 5: Commit**

```bash
git add crates/channels/src/telegram/adapter.rs crates/channels/src/web.rs web/src/pages/Chat.tsx
git commit -m "feat(channels): surface background tool progress separately from active replies"
```

### Task 6: Full verification

**Files:**
- Verify only

**Step 1: Run kernel tests**

Run: `cargo test -p rara-kernel --lib`

Expected: PASS

**Step 2: Run channel tests**

Run: `cargo test -p rara-channels telegram::adapter::tests -- --nocapture`

Expected: PASS

**Step 3: Run targeted app flow**

Run: `cargo test -p rara-app background_tool_completion_reenters_original_session -- --nocapture`

Expected: PASS

**Step 4: Manual smoke test**

Run a Telegram/Web session with a deliberately long detachable tool and verify:
- user can send a second message while the tool is still running
- the session accepts the new input immediately
- the background tool completion later re-enters the same session

**Step 5: Commit**

```bash
git status
```

Review the final diff before merge/PR work.
