# rara-kernel — Agent Guidelines

## Critical: StreamDelta Event Ordering in `openai.rs`

### The Invariant

In `StreamAccumulator::process_chunk()`, **`ToolCallStart` MUST be sent before `ToolCallArgumentsDelta`** for the same tool call index.

The receiver in `agent.rs` uses a `HashMap<u32, PendingToolCall>` keyed by index. The entry is only created when `ToolCallStart` arrives. If `ToolCallArgumentsDelta` arrives first, `get_mut(&index)` returns `None` and **the arguments are silently dropped**.

### Why This Matters

Some LLM providers (notably OpenRouter) deliver the tool call name and arguments in a **single SSE chunk**. If the code emits `ToolCallArgumentsDelta` before `ToolCallStart`, the arguments are lost. This causes:

- Tool calls with empty `{}` arguments
- Bash tool fails with `missing required parameter: command`
- Agent enters a retry loop (67+ failed calls observed in production)

### The Pattern (DO NOT CHANGE)

```
1. Set entry.id    (from tc.id)
2. Set entry.name  (from tc.function.name)
3. Collect args into local variable (DO NOT send yet)
4. Emit ToolCallStart  (if !started && id + name are set)
5. Emit ToolCallArgumentsDelta  (now the receiver entry exists)
```

### What NOT To Do

- Do NOT move `ToolCallArgumentsDelta` emission before `ToolCallStart`
- Do NOT inline the argument send back into the `if let Some(ref func)` block before the start check
- Do NOT assume providers send tool call parts in separate chunks — single-chunk delivery is common
