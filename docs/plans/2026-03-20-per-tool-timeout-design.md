# Per-Tool Execution Timeout — Design Doc

**Issue**: #778
**Date**: 2026-03-20

## Problem

Currently the agent loop uses a single global `tool_execution_timeout` (180s) that
wraps the entire `join_all` of all parallel tool futures. If any single tool
exceeds this, the entire wave is killed with `AgentExecution` error — aborting
the whole agent turn instead of just the offending tool.

## Approach — Trait Method + Macro Attribute

### 1. `AgentTool` trait extension

Add a default method to the `AgentTool` trait:

```rust
fn execution_timeout(&self) -> Option<Duration> { None }
```

`None` means "use the kernel default timeout". Tools with internal timeout
management (bash, http-fetch) return `None` and handle their own timeouts.

### 2. `KernelConfig` addition

Add `default_tool_timeout: Duration` (default 120s) — the per-tool timeout
applied when `execution_timeout()` returns `None`.

### 3. `ToolDef` macro attribute

Add optional `timeout_secs` attribute:

```rust
#[tool(name = "...", description = "...", timeout_secs = 30)]
```

Generates: `fn execution_timeout(&self) -> Option<Duration> { Some(Duration::from_secs(30)) }`

### 4. Agent loop changes

In the tool execution closure (agent/mod.rs ~line 2000), wrap each individual
`tool.execute()` with `tokio::time::timeout` inside `tokio::select!` (for
cancellation support):

```rust
let per_tool_timeout = tool.execution_timeout().unwrap_or(default_tool_timeout);
let tool_result = tokio::select! {
    result = tokio::time::timeout(per_tool_timeout, tool.execute(args, &tc)) => {
        match result {
            Ok(inner) => inner,
            Err(_elapsed) => Err(anyhow::anyhow!(
                "tool execution timed out after {}s", per_tool_timeout.as_secs()
            )),
        }
    }
    _ = tool_cancel.cancelled() => { /* existing interrupt handling */ }
};
```

Tool futures are spawned into a `JoinSet` (not `join_all`) so that completed
results are preserved if the global wave timeout fires.

### 5. Timeout behavior

- Per-tool timeout: returns `{"error": "tool execution timed out after Ns"}`
  for that tool only; other tools in the wave continue normally
- Global wave timeout (180s): safety net using `JoinSet` — completed tools
  keep their real results, only incomplete tools get synthetic timeout errors
  (no `Err(AgentExecution)` — the agent turn continues gracefully)

### 6. Tools with internal cleanup

Tools like `bash` and `http-fetch` that manage their own timeouts declare
`timeout_secs` in the macro with a value larger than their internal timeout
(e.g. bash: internal 120s, external 150s). This ensures the internal cleanup
mechanism fires first while the external timeout acts as a safety net.

## Files to modify

1. `crates/kernel/src/tool/mod.rs` — add `execution_timeout()` to `AgentTool` trait
2. `crates/kernel/src/kernel.rs` — add `default_tool_timeout` to `KernelConfig`
3. `crates/common/tool-macro/src/lib.rs` — add `timeout_secs` attribute
4. `crates/kernel/src/agent/mod.rs` — per-tool timeout wrapping + graceful global timeout
5. `crates/app/src/tools/bash.rs` — override `execution_timeout` for bash
