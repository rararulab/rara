# Context Budget Management Design

Date: 2026-03-07

## Problem

1. 图片发给 LLM 时没有压缩，浪费大量 token。
2. Agent 不会主动 handoff，导致上下文溢出。
3. 上下文溢出后用户无感知（fork discard 保证 tape 不污染，但用户看到没有回复）。

## Design Principles

- 参考 bub 的策略：agent 自主决策 handoff，不做程序化自动 compact。
- Token 用量使用模型返回的真实 `usage`，不做本地预估。
- 图片在 channel 层初始压缩，LLM driver 层做格式适配。

## Changes

### 1. Usage 收集（agent.rs）

`StreamDelta::Done { stop_reason, .. }` 当前丢弃了 `usage`。

改为提取 usage，写入 tape event：

```rust
StreamDelta::Done { stop_reason, usage } => {
    has_tool_calls = stop_reason == llm::StopReason::ToolCalls;
    if let Some(u) = usage {
        tape.append_event("llm.run", json!({
            "usage": {
                "prompt_tokens": u.prompt_tokens,
                "completion_tokens": u.completion_tokens,
                "total_tokens": u.total_tokens
            }
        })).await;
    }
    break;
}
```

### 2. TapeInfo 增加 last_token_usage（service.rs）

`TapeInfo` 增加 `last_token_usage: Option<u32>` 字段。

`info()` 倒序遍历 entries，找到最近的 `llm.run` event，读取 `usage.total_tokens`。与 bub 的 `TapeInfo` 对齐。

### 3. tape.info Tool（新增）

输出格式：

```
tape={name}
entries={N}
anchors={N}
last_anchor={name}
entries_since_last_anchor={N}
last_token_usage={N|unknown}
```

### 4. tape.handoff Tool（新增）

参数：

```rust
struct HandoffInput {
    name: Option<String>,       // anchor 名称，默认 "handoff"
    summary: Option<String>,    // 对之前内容的总结
    next_steps: Option<String>, // 后续待做事项
}
```

执行：调用 `TapeService::handoff()`，写入 anchor，返回 `"handoff created: {name}"`。

### 5. System Prompt 增加 context_contract

在 system prompt 追加：

```xml
<context_contract>
Excessively long context may cause model call failures.
In this case, you SHOULD first use tape.handoff tool to
shorten the length of the retrieved history.
</context_contract>
```

### 6. ContentBlock 增加 ImageBase64 变体（llm/types.rs）

```rust
pub enum ContentBlock {
    Text { text: String },
    ImageUrl { url: String },
    ImageBase64 { media_type: String, data: String },  // 新增
}
```

### 7. Wire 序列化支持 base64（openai.rs）

`WireContentPart` 增加 `ImageBase64` 变体，序列化为 OpenAI 格式：

```json
{
  "type": "image_url",
  "image_url": { "url": "data:image/jpeg;base64,..." }
}
```

OpenAI 和 Claude API 都支持 data URI，wire 层统一走 `image_url` 字段。

### 8. 图片压缩函数（新增 kernel/src/llm/image.rs）

```rust
/// 压缩图片：等比缩放到 max_edge，转 JPEG quality 85。
pub fn compress_image(bytes: &[u8], max_edge: u32, quality: u8) -> Result<(Vec<u8>, String)>
```

- 使用 `image` crate 做 resize + jpeg 编码
- `max_edge` 默认 1568（Anthropic 推荐上限）
- 返回 `(jpeg_bytes, "image/jpeg")`

### 9. Telegram 收图时压缩（telegram/adapter.rs）

收到用户图片时：
1. 下载原图
2. `compress_image(bytes, 1568, 85)`
3. base64 编码
4. 构造 `ContentBlock::ImageBase64 { media_type, data }`

### 10. ContextWindow 错误通知用户（kernel.rs）

当 agent loop 因 `KernelError::ContextWindow` 失败时，fork discard 后向用户发送通知：

```
上下文已超出模型限制，本轮对话未完成。
请发送 /handoff 或等待下次对话自动截断。
```

## Dependencies

- 新增 crate 依赖：`image`（图片 resize + jpeg 编码）

## Change Summary

| # | 改动 | 文件 | 类型 |
|---|------|------|------|
| 1 | agent loop 提取 usage 写入 tape event | `kernel/src/agent.rs` | 修改 |
| 2 | TapeInfo 增加 last_token_usage | `kernel/src/memory/service.rs` | 修改 |
| 3 | 增加 tape.info tool | `app/src/tools/` | 新增 |
| 4 | 增加 tape.handoff tool | `app/src/tools/` | 新增 |
| 5 | system prompt 增加 context_contract | `kernel/src/agent.rs` 或 prompt 模板 | 修改 |
| 6 | ContentBlock 增加 ImageBase64 变体 | `kernel/src/llm/types.rs` | 修改 |
| 7 | WireContentPart 支持 base64 序列化 | `kernel/src/llm/openai.rs` | 修改 |
| 8 | 图片压缩函数 | `kernel/src/llm/image.rs` | 新增 |
| 9 | Telegram 收图时压缩 | `channels/src/telegram/adapter.rs` | 修改 |
| 10 | ContextWindow 错误通知用户 | `kernel/src/kernel.rs` | 修改 |

## Dependency Order

- 1 → 2 → 3/4（usage 收集 → TapeInfo → tools）
- 6 → 7 → 8 → 9（类型 → 序列化 → 压缩 → channel 集成）
- 5, 10 独立

## What We Don't Do

- 不做程序化自动 compact
- 不做 token 本地预估（用模型返回的真实 usage）
- 不在 system prompt 注入 token 用量数字
