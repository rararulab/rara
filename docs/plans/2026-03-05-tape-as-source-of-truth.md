# Tape as Source of Truth — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make `TapeService` the single source of truth for conversation history, eliminating the in-memory `Vec<ChatMessage>` from `Session`. Also reorganize `process/` into meaningful modules.

**Architecture:** Tape entries (JSONL) become the authoritative conversation store. The kernel reads from tape to build LLM context, writes to tape during agent turns. The `SlidingWindowCompaction` is removed; context management uses tape's anchor/handoff mechanism. The `process/` grab-bag is split into `session.rs`, `agent.rs`, `identity.rs`.

**Tech Stack:** Rust, rara-memory (tape), rara-kernel

---

## Phase 1: Reorganize `process/` (structural, no behavior change)

### Task 1: Split `process/` into `session.rs`, `agent.rs`, `identity.rs`

**Files:**
- Delete: `crates/kernel/src/process/mod.rs`
- Delete: `crates/kernel/src/process/agent_registry.rs`
- Delete: `crates/kernel/src/process/manifest_loader.rs`
- Delete: `crates/kernel/src/process/principal.rs`
- Delete: `crates/kernel/src/process/user.rs`
- Create: `crates/kernel/src/agent.rs`
- Create: `crates/kernel/src/identity.rs`
- Modify: `crates/kernel/src/lib.rs` — replace `pub mod process` with new modules

**Step 1: Create `crates/kernel/src/agent.rs`**

Move these types from `process/mod.rs` + `process/agent_registry.rs` + `process/manifest_loader.rs`:
- `AgentRole`
- `Priority`
- `SandboxConfig`
- `AgentManifest`
- `AgentEnv`
- `AgentRegistry` (from `agent_registry.rs`)
- `ManifestLoader` (from `manifest_loader.rs`)

**Step 2: Create `crates/kernel/src/identity.rs`**

Move these types from `process/principal.rs` + `process/user.rs`:
- `Principal` (from `principal.rs`)
- `KernelUser` (from `user.rs`)
- `Permission` (from `user.rs`)
- `UserStore` trait (from `user.rs`)

**Step 3: Keep session types in existing `crates/kernel/src/session.rs`** (or create if not exists)

Move from `process/mod.rs`:
- `Session`
- `SessionState`
- `SessionTable`
- `RuntimeMetrics`, `MetricsSnapshot`
- `SessionStats`, `SystemStats`
- `AgentRunLoopResult`
- `Signal`

**Step 4: Update `crates/kernel/src/lib.rs`**

Replace:
```rust
pub mod process;
```
With:
```rust
pub mod agent;
pub mod identity;
```
And ensure session types are exported from the existing `session` module.

**Step 5: Fix all `use crate::process::*` imports across the kernel crate**

Grep for `crate::process` and update to `crate::agent`, `crate::identity`, or `crate::session` as appropriate.

**Step 6: Fix external crate imports**

Grep for `rara_kernel::process` in the entire workspace and update references:
- `rara-boot` uses `AgentRegistry`, `ManifestLoader`, `UserStore`
- `rara-agents` uses `AgentManifest`, `AgentRole`, `Priority`
- `backend-admin` extension uses various types

**Step 7: Verify**

Run: `cargo check -p rara-kernel`
Then: `cargo check --workspace`

**Step 8: Commit**

```bash
git add -A && git commit -m "refactor(kernel): split process/ into agent, identity, session modules"
```

---

## Phase 2: Add tape-based LLM context builder

### Task 2: Add `build_llm_context()` to `TapeService`

**Files:**
- Modify: `crates/memory/src/tape/service.rs`

**Step 1: Add method**

```rust
/// Build LLM-ready messages from tape entries since the last anchor.
///
/// Reads entries from the most recent anchor, filters to
/// Message/ToolCall/ToolResult kinds, and assembles them into the
/// JSON message format expected by LLM APIs via `default_tape_context`.
pub async fn build_llm_context(&self) -> TapResult<Vec<Value>> {
    let entries = self.from_last_anchor(Some(&[
        TapEntryKind::Message,
        TapEntryKind::ToolCall,
        TapEntryKind::ToolResult,
    ])).await?;
    super::context::default_tape_context(&entries)
}
```

**Step 2: Verify**

Run: `cargo check -p rara-memory`

**Step 3: Commit**

```bash
git add crates/memory/src/tape/service.rs
git commit -m "feat(memory): add build_llm_context to TapeService"
```

---

### Task 3: Add `tape_context_to_llm_messages` converter in kernel

**Files:**
- Modify: `crates/kernel/src/agent_loop.rs`

**Step 1: Add conversion function**

Add alongside existing `build_llm_history`:

```rust
/// Convert tape context JSON messages (from `default_tape_context`) into
/// [`llm::Message`] format for the LLM driver.
pub(crate) fn tape_context_to_llm_messages(tape_messages: &[serde_json::Value]) -> Vec<llm::Message> {
    tape_messages
        .iter()
        .filter_map(|msg| {
            let role = msg.get("role")?.as_str()?;
            let content = msg.get("content").and_then(|c| c.as_str()).unwrap_or("");

            match role {
                "system" => Some(llm::Message::system(content)),
                "user" => Some(llm::Message::user(content)),
                "assistant" => {
                    if let Some(tool_calls) = msg.get("tool_calls").and_then(|tc| tc.as_array()) {
                        let calls: Vec<llm::ToolCallRequest> = tool_calls
                            .iter()
                            .filter_map(|tc| {
                                let id = tc.get("id")?.as_str()?.to_string();
                                let function = tc.get("function")?.as_object()?;
                                let name = function.get("name")?.as_str()?.to_string();
                                let arguments = function.get("arguments")?.as_str()?.to_string();
                                Some(llm::ToolCallRequest { id, name, arguments })
                            })
                            .collect();
                        if calls.is_empty() {
                            Some(llm::Message::assistant(content))
                        } else {
                            Some(llm::Message::assistant_with_tool_calls(content, calls))
                        }
                    } else {
                        Some(llm::Message::assistant(content))
                    }
                }
                "tool" => {
                    let tool_call_id = msg.get("tool_call_id")
                        .and_then(|id| id.as_str())
                        .unwrap_or("");
                    Some(llm::Message::tool_result(tool_call_id, content))
                }
                _ => None,
            }
        })
        .collect()
}
```

**Step 2: Verify**

Run: `cargo check -p rara-kernel`

**Step 3: Commit**

```bash
git add crates/kernel/src/agent_loop.rs
git commit -m "feat(kernel): add tape_context_to_llm_messages converter"
```

---

## Phase 3: Wire tape into the kernel (breaking changes)

### Task 4: Update `agent_loop` to persist to tape during turn

**Files:**
- Modify: `crates/kernel/src/agent_loop.rs`

**Step 1: Add `TapeService` parameter to `run_inline_agent_loop`**

Change signature:
```rust
pub(crate) async fn run_inline_agent_loop(
    handle: &KernelHandle,
    session_key: SessionKey,
    user_text: String,
    history: Option<Vec<llm::Message>>,
    stream_handle: &StreamHandle,
    turn_cancel: &CancellationToken,
    tape: &rara_memory::tape::TapeService,  // NEW
) -> crate::error::Result<AgentTurnResult> {
```

**Step 2: After assembling tool calls, persist assistant+tool_call to tape**

After the `messages.push(llm::Message::assistant_with_tool_calls(...))` block (~line 504), add:

```rust
// Persist assistant message with tool calls to tape
{
    let calls_json: Vec<serde_json::Value> = assistant_tool_calls.iter().map(|tc| {
        serde_json::json!({
            "id": tc.id,
            "function": { "name": tc.name, "arguments": tc.arguments }
        })
    }).collect();
    let _ = tape.append_tool_call(serde_json::json!({"calls": calls_json})).await;
}
```

**Step 3: After executing tool calls and collecting results, persist tool_results to tape**

After the tool results are appended to `messages` (~line 584), add:

```rust
// Persist tool results to tape
{
    let results_json: Vec<serde_json::Value> = valid_tool_calls.iter()
        .zip(results.iter())
        .map(|((_id, _name, _args), (_success, result, _err, _dur))| result.clone())
        .collect();
    let _ = tape.append_tool_result(serde_json::json!({"results": results_json})).await;
}
```

Note: The `results` from `join_all` are consumed by the zip below, so capture the results data before the existing zip loop.

**Step 4: After the turn completes (no tool calls), persist final assistant message to tape**

In the terminal response block (~line 450), add before the return:

```rust
// Persist final assistant message to tape
let _ = tape.append_message(serde_json::json!({
    "role": "assistant",
    "content": &accumulated_text
})).await;
```

**Step 5: Verify**

Run: `cargo check -p rara-kernel`

**Step 6: Commit**

```bash
git add crates/kernel/src/agent_loop.rs
git commit -m "feat(kernel): agent_loop persists messages to tape during turn"
```

---

### Task 5: Update `kernel.handle_user_message` to use tape

**Files:**
- Modify: `crates/kernel/src/kernel.rs`

**Step 1: Replace conversation-based flow with tape-based flow**

The current flow (~lines 1100-1200) does:
1. Take `conversation` from Session
2. Compact
3. `build_llm_history` from compacted
4. Put conversation back + push user message
5. Fire-and-forget tape persist
6. Call agent_loop with history

Replace with:
```rust
// 1. Append user message to tape (awaited, not fire-and-forget)
let tape = self.tape_for(&session_key);
tape.append_message(serde_json::json!({
    "role": "user",
    "content": &user_text
})).await.map_err(|e| {
    tracing::warn!(%e, "failed to persist user message to tape");
});

// 2. Build LLM context from tape
let tape_messages = tape.build_llm_context().await.unwrap_or_default();
let history = {
    let msgs = crate::agent_loop::tape_context_to_llm_messages(&tape_messages);
    if msgs.is_empty() { None } else { Some(msgs) }
};

// 3. Call agent loop (tape persists tool calls/results/assistant messages internally)
let result = crate::agent_loop::run_inline_agent_loop(
    &self.kernel_handle,
    session_key,
    user_text,
    history,
    &stream_handle,
    &turn_cancel,
    &tape,  // NEW
).await;
```

**Step 2: Remove post-turn conversation push**

Remove the code (~line 1376) that pushes `assistant_msg` to `rt.conversation` — agent_loop now handles tape persistence.

Also remove the fire-and-forget tape persist of assistant messages — already handled in agent_loop.

**Step 3: Verify**

Run: `cargo check -p rara-kernel`

**Step 4: Commit**

```bash
git add crates/kernel/src/kernel.rs
git commit -m "refactor(kernel): handle_user_message reads/writes via tape"
```

---

### Task 6: Update `handle_child_completed` to use tape

**Files:**
- Modify: `crates/kernel/src/kernel.rs`

**Step 1: Replace conversation push with tape append**

Current (~line 739-750):
```rust
rt.conversation.push(child_msg.clone());
```

Replace with:
```rust
let tape = self.tape_for(&parent_id);
let _ = tape.append_message(serde_json::json!({
    "role": "user",
    "content": &child_result_text
})).await;
```

Remove the `process_table.with_mut` that pushes to conversation.

**Step 2: Verify**

Run: `cargo check -p rara-kernel`

**Step 3: Commit**

```bash
git add crates/kernel/src/kernel.rs
git commit -m "refactor(kernel): handle_child_completed writes to tape"
```

---

### Task 7: Remove `conversation` from `Session` and delete compaction

**Files:**
- Modify: `crates/kernel/src/process/mod.rs` (or `session.rs` after Task 1)
- Delete: `crates/kernel/src/compaction.rs`
- Modify: `crates/kernel/src/lib.rs` — remove `pub mod compaction`

**Step 1: Remove `conversation` field from `Session`**

Remove:
```rust
pub conversation: Vec<ChatMessage>,
pub max_context_tokens: usize,
```

**Step 2: Remove conversation-related methods from `SessionTable`**

Any methods that only exist to manipulate conversation.

**Step 3: Update `spawn_agent` in kernel.rs**

Remove the `initial_messages` loading block (~lines 544-555) and the `conversation` field from `Session` construction (~line 595).

**Step 4: Delete `compaction.rs`**

Remove the file and `pub mod compaction` from `lib.rs`. Remove any `use crate::compaction::*` imports.

**Step 5: Fix all compile errors**

Grep for `conversation` and `compaction` across the kernel crate, fix remaining references.

**Step 6: Verify**

Run: `cargo check -p rara-kernel`
Then: `cargo check --workspace`

**Step 7: Commit**

```bash
git add -A
git commit -m "refactor(kernel): remove conversation Vec and compaction module

Tape is now the single source of truth for conversation history.
Context management uses tape anchors instead of in-memory compaction."
```

---

## Phase 4: Cleanup & verify

### Task 8: Remove dead code and unused imports

**Step 1:** Run `cargo clippy --workspace` and fix warnings related to unused imports, dead code from the refactor.

**Step 2:** Remove `build_llm_history` function (no longer needed, replaced by `tape_context_to_llm_messages`).

**Step 3:** Remove `ChatMessage` imports where no longer used.

**Step 4:** Verify full workspace builds: `cargo check --workspace`

**Step 5: Commit**

```bash
git add -A && git commit -m "chore(kernel): remove dead code from tape migration"
```

---

## Dependency Graph

```
Task 1 (reorganize process/)
  ↓
Task 2 (TapeService.build_llm_context)  ──┐
Task 3 (tape_context_to_llm_messages)   ──┤
  ↓                                        │
Task 4 (agent_loop + tape param)        ──┤
  ↓                                        │
Task 5 (handle_user_message via tape)   ←─┘
  ↓
Task 6 (handle_child_completed via tape)
  ↓
Task 7 (remove conversation + compaction)
  ↓
Task 8 (cleanup)
```

## Risk Notes

- **No rollback path**: Once `conversation: Vec` is removed, the old flow is gone. This is intentional.
- **Tape I/O performance**: Tape reads on every turn instead of in-memory access. `FileTapeStore` has read caching (`_read_offset` incremental reads) so this should be acceptable.
- **Context size**: Without compaction, we rely on anchors. For now, the `from_last_anchor` window is the entire session since `session/start`. A future task should add automatic handoff when context exceeds a token budget.
- **Mid-turn crash recovery**: Tool calls/results are now persisted mid-turn, so on restart the tape has partial turn data. `default_tape_context` handles this correctly (incomplete tool call pairs are simply included).
