# Tool Progress Reporting for Telegram

**Issue**: #87
**Date**: 2026-03-06

## Problem

Rara executes multi-step tool tasks silently. Users see no feedback until the final reply, making it feel stuck.

## Solution

Handle `ToolCallStart`/`ToolCallEnd` stream events in the Telegram stream forwarder to send/edit a progress message showing tool execution status.

## Design

### Message Lifecycle

```
ToolCallStart("shell")       → sendMessage:  "🔧 shell..."
ToolCallStart("read_file")   → editMessage:  "🔧 shell...\n🔧 read_file..."
ToolCallEnd("shell", ok)     → editMessage:  "✅ shell\n🔧 read_file..."
ToolCallEnd("read_file", ok) → editMessage:  "✅ shell\n✅ read_file"
(next iteration tools)       → continue appending to same message
Final text reply arrives     → progress message stays (all ✅)
```

### Data Structures

```rust
struct ToolProgress {
    name:     String,
    finished: bool,
    success:  bool,
}

struct ProgressMessage {
    message_id: Option<MessageId>,
    tools:      Vec<ToolProgress>,
}
```

Added as a field on `StreamingMessage` (or alongside it in `active_streams`).

### Render Function

```rust
fn render_progress(tools: &[ToolProgress]) -> String {
    tools.iter().map(|t| {
        if t.finished {
            if t.success { format!("✅ {}", t.name) }
            else { format!("❌ {}", t.name) }
        } else {
            format!("🔧 {}...", t.name)
        }
    }).collect::<Vec<_>>().join("\n")
}
```

### Changes

**Single file**: `crates/channels/src/telegram/adapter.rs`

1. Add `ToolProgress` + `ProgressMessage` structs
2. Add `render_progress()` function
3. In `spawn_stream_forwarder`, replace `Ok(_) => {}` with handlers for:
   - `ToolCallStart { name, .. }` → push tool, send/edit progress message
   - `ToolCallEnd { id, success, .. }` → mark tool done, edit progress message
4. Throttle edits using existing `MIN_EDIT_INTERVAL`
5. Send `send_chat_action(typing)` before first progress message

### Edge Cases

- **Fast tools (<1s)**: Still send progress, shown as immediately ✅
- **Send failure**: Silently ignore, progress is best-effort
- **Tool name display**: Simple mapping for common tools, fallback to raw name
