# Tool Result Multimodal 通道设计

## 问题

Rara 有"用户发图 → 压缩 → multimodal 输入"的链路（Telegram adapter），但没有"工具产图 → 压缩/引用 → multimodal 输入"的链路。

工具（如 `screenshot`）返回的图片只能作为 JSON 文本回灌到 LLM，导致：
- 模型无法真正"看到"截图
- OCR/元素树等大文本积进上下文
- 容易 context overflow

### 根因

| 阶段 | 瓶颈 |
|------|------|
| `AgentTool::execute()` | 返回 `Result<Value>`，无法携带二进制 |
| `agent.rs:1032` | `result.to_string()` 丢失所有结构 |
| `Message::tool_result()` | 只接受 `impl Into<String>`，强制 `MessageContent::Text` |
| `context.rs:render_tool_result()` | 所有 Value 序列化为字符串 |

而 `openai.rs` LLM driver 层已经完整支持 `MessageContent::Multimodal`。

## 方案

### 设计原则

- **content vs artifact 分离**：LLM 看到压缩图片 + 简短文本描述，tape 只存引用
- **Resource 外部存储**：图片文件存磁盘，tape JSONL 保持轻量
- **类型安全**：通过 `ToolOutput` 类型让工具显式声明"我返回图片"
- **源码兼容**：`From<Value> for ToolOutput` 保证现有工具零改动

### 业界参考

- **LangChain** `response_format="content_and_artifact"`：工具返回 `[content, artifact]` 元组，content 给 LLM，artifact 给下游
- **Claude API** `tool_result`：content 字段支持 content blocks 数组，可内联 image base64
- **共同模式**：原始资源存独立位置，通过引用关联

## 核心类型

### `ToolOutput` + `ResourceAttachment`（`kernel/src/tool.rs`）

```rust
pub struct ResourceAttachment {
    pub media_type: String,   // "image/jpeg"
    pub data: Vec<u8>,        // 压缩后的原始字节
}

pub struct ToolOutput {
    pub json: serde_json::Value,
    pub resources: Vec<ResourceAttachment>,
}

impl From<serde_json::Value> for ToolOutput {
    fn from(json: serde_json::Value) -> Self {
        Self { json, resources: vec![] }
    }
}
```

`AgentTool::execute()` 返回类型从 `Result<Value>` 改为 `Result<ToolOutput>`。

### `Message::tool_result_multimodal`（`llm/types.rs`）

```rust
pub fn tool_result_multimodal(tool_call_id: impl Into<String>, blocks: Vec<ContentBlock>) -> Self {
    Self {
        role:         Role::Tool,
        content:      MessageContent::Multimodal(blocks),
        tool_calls:   Vec::new(),
        tool_call_id: Some(tool_call_id.into()),
    }
}
```

## Resource 存储

### `ResourceStore`（`kernel/src/memory/resource.rs`）

```rust
pub struct ResourceStore {
    base_dir: PathBuf,  // ~/.local/share/rara/resources/
}

pub struct ResourceRef {
    pub id: String,           // uuid
    pub media_type: String,
    pub rel_path: String,     // "{uuid}.jpg"
}
```

- `store(attachment) -> ResourceRef`：写文件，返回引用
- `load(ref_) -> Vec<u8>`：按引用读取字节

### Tape 持久化格式

```json
{
  "results": [{"success": true, "description": "screenshot captured"}],
  "__resources": [[{"id": "abc-123", "media_type": "image/jpeg", "rel_path": "abc-123.jpg"}]]
}
```

`__resources` 是 `Vec<Vec<ResourceRef>>`，外层对应每个 tool call，内层对应该 call 的多个 resource。

## Agent Loop 改动（`agent.rs`）

1. **执行后**：遍历 `ToolOutput.resources`，调用 `ResourceStore::store()` 持久化
2. **写 tape**：payload 附加 `__resources` 字段
3. **构建 LLM 消息**：
   - 有 resources → `ResourceStore::load()` → base64 → `ContentBlock::ImageBase64` → `Message::tool_result_multimodal()`
   - 无 resources → 原有 `Message::tool_result()` 纯文本路径

## Context 重建（`context.rs`）

`append_tool_result_entry` 检测 payload 中的 `__resources`：
- 有 → 读取 resource 文件 → base64 → multimodal message
- 无 → 走原来的 `render_tool_result()` 纯文本路径

注意：`context.rs` 需要访问 `ResourceStore`，`default_tape_context()` 签名需要增加 `resource_store` 参数。

## Screenshot 工具改造

```rust
async fn execute(&self, params: Value, _context: &ToolContext) -> Result<ToolOutput> {
    // ...Playwright 截图...
    let raw_bytes = tokio::fs::read(&output_path).await?;
    let (compressed, media_type) =
        image::compress_image(&raw_bytes, DEFAULT_MAX_EDGE, DEFAULT_QUALITY)?;
    let _ = tokio::fs::remove_file(&output_path).await;

    Ok(ToolOutput {
        json: json!({ "success": true, "description": "screenshot captured" }),
        resources: vec![ResourceAttachment { media_type, data: compressed }],
    })
}
```

## 影响范围

| 文件 | 改动类型 |
|------|----------|
| `kernel/src/tool.rs` | 新增 `ToolOutput`, `ResourceAttachment`；trait 签名变更 |
| `kernel/src/llm/types.rs` | 新增 `tool_result_multimodal()` |
| `kernel/src/memory/resource.rs` | 新增模块 |
| `kernel/src/memory/context.rs` | 支持 `__resources` multimodal 重建 |
| `kernel/src/agent.rs` | resource 持久化 + multimodal message 构建 |
| `app/src/tools/screenshot.rs` | 返回 `ToolOutput` + attachment |
| 所有其他 `impl AgentTool` | `Ok(json!(...))` → `Ok(json!(...).into())` |
