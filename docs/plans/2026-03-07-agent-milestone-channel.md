# Agent Milestone Channel Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace the oneshot result channel with an mpsc channel that carries milestone events during child agent execution, so parent agents receive structured progress and can decide what to tell the user.

**Architecture:** `AgentEvent` enum with `Milestone` and `Done` variants flows through an `mpsc` channel from child to parent. The child's `run_agent_loop` emits milestones at key points (tool call start/end, iteration start). `exec_spawn` collects milestones and returns them alongside the final output in the tool result JSON. Parent agent LLM sees milestones and decides what to communicate.

**Tech Stack:** Rust, tokio mpsc, serde

---

### Task 1: Define `AgentEvent` enum and update `AgentHandle`

**Files:**
- Modify: `crates/kernel/src/io.rs:220-228` (AgentRunLoopResult import area)
- Modify: `crates/kernel/src/io.rs:963-973` (AgentHandle struct)

**Step 1: Write the failing test**

```rust
// crates/kernel/src/io.rs — add at bottom of file in #[cfg(test)] mod
#[cfg(test)]
mod agent_event_tests {
    use super::*;
    use crate::session::AgentRunLoopResult;

    #[test]
    fn milestone_serializes_to_json() {
        let event = AgentEvent::Milestone {
            stage: "tool_call_start".to_string(),
            detail: Some("mobile_screenshot".to_string()),
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "milestone");
        assert_eq!(json["stage"], "tool_call_start");
        assert_eq!(json["detail"], "mobile_screenshot");
    }

    #[test]
    fn done_wraps_result() {
        let result = AgentRunLoopResult {
            output: "done".to_string(),
            iterations: 3,
            tool_calls: 5,
        };
        let event = AgentEvent::Done(result.clone());
        match event {
            AgentEvent::Done(r) => assert_eq!(r.output, "done"),
            _ => panic!("expected Done"),
        }
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p rara-kernel agent_event_tests -- --nocapture`
Expected: FAIL — `AgentEvent` not found

**Step 3: Define `AgentEvent` and update `AgentHandle`**

In `crates/kernel/src/io.rs`, add the enum near the `AgentHandle` struct:

```rust
/// Events sent from a child agent to its parent during execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    /// A key execution milestone (tool call, iteration boundary, etc.).
    Milestone {
        stage:  String,
        detail: Option<String>,
    },
    /// Agent execution completed.
    Done(AgentRunLoopResult),
}
```

Update `AgentHandle`:

```rust
pub struct AgentHandle {
    pub session_key: SessionKey,
    pub result_rx:   mpsc::Receiver<AgentEvent>,
}
```

Add `AgentEvent` to the module's public exports.

**Step 4: Run test to verify it passes**

Run: `cargo test -p rara-kernel agent_event_tests -- --nocapture`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/kernel/src/io.rs
git commit -m "feat(kernel): define AgentEvent enum with Milestone and Done variants (#127)"
```

---

### Task 2: Update `Session.result_tx` to mpsc

**Files:**
- Modify: `crates/kernel/src/session.rs:272` (result_tx field type)

**Step 1: Change the type**

In `crates/kernel/src/session.rs`, change:

```rust
// Before:
pub result_tx: Option<tokio::sync::oneshot::Sender<AgentRunLoopResult>>,

// After:
pub result_tx: Option<tokio::sync::mpsc::Sender<crate::io::AgentEvent>>,
```

**Step 2: Run cargo check to find all compile errors**

Run: `cargo check -p rara-kernel 2>&1 | head -60`
Expected: Errors in `handle.rs` and `kernel.rs` where `oneshot::channel()` and `tx.send()` are used. These are fixed in Tasks 3 and 4.

**Step 3: Commit (will not compile yet — that's ok, we fix consumers next)**

```bash
git add crates/kernel/src/session.rs
git commit -m "refactor(kernel): change Session.result_tx from oneshot to mpsc (#127)"
```

---

### Task 3: Update `spawn_child` in `handle.rs`

**Files:**
- Modify: `crates/kernel/src/handle.rs:347-379` (spawn_child method)

**Step 1: Update channel creation**

```rust
// Before (handle.rs:368):
let (result_tx, result_rx) = tokio::sync::oneshot::channel();

// After:
let (result_tx, result_rx) = tokio::sync::mpsc::channel(64);
```

The rest of the method stays the same — `result_tx` goes into the session, `result_rx` goes into `AgentHandle`.

**Step 2: Run cargo check**

Run: `cargo check -p rara-kernel 2>&1 | head -60`
Expected: Still errors in `kernel.rs` (Task 4) and `syscall.rs` (Task 5). `handle.rs` should be clean now.

**Step 3: Commit**

```bash
git add crates/kernel/src/handle.rs
git commit -m "refactor(kernel): spawn_child uses mpsc channel for AgentEvent (#127)"
```

---

### Task 4: Update kernel cleanup to send `AgentEvent::Done`

**Files:**
- Modify: `crates/kernel/src/kernel.rs:736-744` (cleanup_process result sending)

**Step 1: Update the send call**

In `cleanup_process` (kernel.rs around line 743), change:

```rust
// Before:
if let Some(tx) = rt.result_tx {
    let _ = tx.send(result.clone());
}

// After:
if let Some(tx) = rt.result_tx {
    let _ = tx.send(crate::io::AgentEvent::Done(result.clone())).await;
}
```

Note: `mpsc::Sender::send` is async, so ensure the enclosing function is async (it already is: `cleanup_process` is `async fn`). If `cleanup_process` takes ownership of `rt.result_tx` by destructuring, this should work directly. If it's inside a non-async closure (e.g. `with_mut`), you may need to extract `result_tx` first and `.await` outside the closure.

Check for the pattern — the `result_tx` is likely extracted via `with_mut` or similar. Look at the exact code and ensure `send().await` happens outside any synchronous closure.

**Step 2: Run cargo check**

Run: `cargo check -p rara-kernel 2>&1 | head -60`
Expected: Errors only in `syscall.rs` now (Task 5).

**Step 3: Commit**

```bash
git add crates/kernel/src/kernel.rs
git commit -m "refactor(kernel): cleanup_process sends AgentEvent::Done via mpsc (#127)"
```

---

### Task 5: Emit milestones from `run_agent_loop`

**Files:**
- Modify: `crates/kernel/src/agent.rs:474-484` (run_agent_loop signature)
- Modify: `crates/kernel/src/agent.rs:831-836` (ToolCallStart emission)
- Modify: `crates/kernel/src/agent.rs:963-968` (ToolCallEnd emission)
- Modify: `crates/kernel/src/kernel.rs` (call site that passes the sender)

**Step 1: Add `milestone_tx` parameter to `run_agent_loop`**

```rust
pub(crate) async fn run_agent_loop(
    handle: &KernelHandle,
    session_key: SessionKey,
    user_text: String,
    history: Option<Vec<llm::Message>>,
    stream_handle: &StreamHandle,
    turn_cancel: &CancellationToken,
    tape: crate::memory::TapeService,
    tape_name: &str,
    tool_context: crate::tool::ToolContext,
    milestone_tx: Option<tokio::sync::mpsc::Sender<crate::io::AgentEvent>>,  // NEW
) -> crate::error::Result<AgentTurnResult> {
```

**Step 2: Emit milestones at ToolCallStart**

After the existing `stream_handle.emit(StreamEvent::ToolCallStart { ... })` (around line 831-835), add:

```rust
if let Some(ref mtx) = milestone_tx {
    let _ = mtx.send(crate::io::AgentEvent::Milestone {
        stage: "tool_call_start".to_string(),
        detail: Some(tool_call.name.clone()),
    }).await;
}
```

**Step 3: Emit milestones at ToolCallEnd**

After the existing `stream_handle.emit(StreamEvent::ToolCallEnd { ... })` (around line 963-968), add:

```rust
if let Some(ref mtx) = milestone_tx {
    let _ = mtx.send(crate::io::AgentEvent::Milestone {
        stage: "tool_call_end".to_string(),
        detail: Some(format!("{}: {}", name, if *success { "ok" } else { "error" })),
    }).await;
}
```

**Step 4: Update call site in kernel.rs**

Find where `run_agent_loop` is called in `kernel.rs` (around line 1484). Pass the session's `result_tx` clone:

```rust
// Extract milestone_tx from session before entering the loop
let milestone_tx = self.process_table.with(&session_key, |p| p.result_tx.clone());

// ... existing code ...

let turn = crate::agent::run_agent_loop(
    &kernel_handle,
    session_key,
    user_text,
    history,
    &stream_handle,
    &turn_cancel,
    tape,
    &tape_name,
    tool_context,
    milestone_tx,  // NEW
).await;
```

**Step 5: Run cargo check**

Run: `cargo check -p rara-kernel 2>&1 | head -60`
Expected: Errors only in `syscall.rs` (Task 6).

**Step 6: Commit**

```bash
git add crates/kernel/src/agent.rs crates/kernel/src/kernel.rs
git commit -m "feat(kernel): run_agent_loop emits milestones via mpsc channel (#127)"
```

---

### Task 6: Update `exec_spawn` to collect milestones

**Files:**
- Modify: `crates/kernel/src/syscall.rs:454-486` (exec_spawn)
- Modify: `crates/kernel/src/syscall.rs:488-546` (exec_spawn_parallel)

**Step 1: Update `exec_spawn`**

```rust
async fn exec_spawn(
    &self,
    agent_name: &str,
    task: &str,
) -> Result<serde_json::Value, anyhow::Error> {
    let manifest = self.resolve_manifest(agent_name)?;
    let principal = self.principal()?;

    info!(agent = agent_name, task = task, "kernel: spawning single agent");

    let agent_handle = self
        .handle
        .spawn_child(&self.session_key, &principal, manifest, task.to_string())
        .await
        .map_err(|e| anyhow::anyhow!("spawn failed: {e}"))?;

    let child_key = agent_handle.session_key;
    let mut rx = agent_handle.result_rx;
    let mut milestones = Vec::new();

    while let Some(event) = rx.recv().await {
        match event {
            crate::io::AgentEvent::Milestone { stage, detail } => {
                milestones.push(serde_json::json!({
                    "stage": stage,
                    "detail": detail,
                }));
            }
            crate::io::AgentEvent::Done(result) => {
                return Ok(serde_json::json!({
                    "milestones": milestones,
                    "output": result.output,
                    "iterations": result.iterations,
                    "tool_calls": result.tool_calls,
                }));
            }
        }
    }

    Err(anyhow::anyhow!(
        "agent {} was dropped without producing a result",
        child_key
    ))
}
```

**Step 2: Update `exec_spawn_parallel`**

Same pattern — collect milestones per agent. Replace the `handle.result_rx.await` with a loop:

```rust
let mut rx = handle.result_rx;
let mut milestones = Vec::new();
let mut final_result = None;

while let Some(event) = rx.recv().await {
    match event {
        crate::io::AgentEvent::Milestone { stage, detail } => {
            milestones.push(serde_json::json!({
                "stage": stage,
                "detail": detail,
            }));
        }
        crate::io::AgentEvent::Done(result) => {
            final_result = Some(result);
            break;
        }
    }
}

match final_result {
    Some(result) => {
        results.push(serde_json::json!({
            "agent": agent_name,
            "milestones": milestones,
            "output": result.output,
            "iterations": result.iterations,
            "tool_calls": result.tool_calls,
        }));
    }
    None => {
        results.push(serde_json::json!({
            "agent": agent_name,
            "error": "agent was dropped without producing a result",
        }));
    }
}
```

**Step 3: Run cargo check**

Run: `cargo check -p rara-kernel`
Expected: PASS — all compile errors resolved.

**Step 4: Commit**

```bash
git add crates/kernel/src/syscall.rs
git commit -m "feat(kernel): exec_spawn collects milestones into tool result (#127)"
```

---

### Task 7: Full build + integration smoke test

**Step 1: Run full cargo check**

Run: `cargo check`
Expected: PASS

**Step 2: Run existing tests**

Run: `cargo test -p rara-kernel`
Expected: All existing tests pass, plus the new `agent_event_tests`.

**Step 3: Run cargo clippy**

Run: `cargo clippy -p rara-kernel -- -D warnings`
Expected: No new warnings.

**Step 4: Final commit if any fixups needed**

```bash
git add -A
git commit -m "test(kernel): agent milestone channel integration (#127)"
```
