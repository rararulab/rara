# OpenAI Wire Format Fix — Design Doc

**Issue:** #743
**Date:** 2026-03-20

## Goal

Align `OpenAiDriver` wire format with the OpenAI Chat Completions API specification.

## Approach

Direct in-place fixes in `crates/kernel/src/llm/openai.rs`. No architectural
changes — all fixes are localized to `ChatRequest::from_completion()` and the
streaming loop in `LlmDriver::stream()`.

## Fixes

### P0 — Wire format correctness

1. **`ToolChoice::None` → `"none"`**: Was incorrectly mapped to `"auto"`,
   allowing tool calls when the caller explicitly prohibited them. Active
   call-sites in `agent/fold.rs` and `agent/mod.rs`.

2. **`ToolChoice::Specific` nested object**: Was sending
   `{"type": "function", "name": N}` instead of the spec-required
   `{"type": "function", "function": {"name": N}}`.

### P1 — Behavioral correctness

3. **Explicit `parallel_tool_calls: false`**: Was only sending `Some(true)` or
   omitting entirely. Now sends `Some(request.parallel_tool_calls)` so
   providers respect the caller's intent.

### P2 — Robustness

4. **SSE parse failure logging**: Silent `continue` replaced with
   `tracing::debug!` including truncated raw data.

5. **Stream early exit on channel close**: Check `tx.is_closed()` at the top
   of each loop iteration; break and return accumulated partial response.

## Key decisions

- `debug!` level for SSE parse failures (not `warn!`) to avoid noise from
  benign non-standard provider events.
- On channel close, return partial accumulated response rather than erroring —
  the data has value even if the downstream consumer disconnected.
- Always send `parallel_tool_calls` when tools are present. Minor risk that
  some providers (Ollama) may not recognize the field, but most silently
  ignore unknown fields.

## Affected crates

- `kernel` only (`crates/kernel/src/llm/openai.rs`)
