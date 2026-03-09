# Tool Result Multimodal 通道实现计划

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 让工具产生的图片（如截图）能以 multimodal 方式回灌到 LLM，而不是只能作为 JSON 文本。

**Architecture:** 新增 `ToolOutput` 类型替代 `Value` 作为工具返回值，图片通过 `ResourceStore` 持久化到磁盘，tape 只存引用。Agent loop 和 context 重建时读取 resource 文件构建 multimodal 消息。

**Tech Stack:** Rust, serde_json, uuid, base64, image crate（已有）

---

### Task 1: `rara_paths::resources_dir()`

**Files:**
- Modify: `crates/paths/src/lib.rs`

**Step 1: 添加 `resources_dir()` 函数**

在 `skills_dir()` 后面添加：

```rust
/// Returns the path to the resources directory for tool-produced artifacts.
pub fn resources_dir() -> &'static PathBuf {
    static RESOURCES_DIR: OnceLock<PathBuf> = OnceLock::new();
    RESOURCES_DIR.get_or_init(|| data_dir().join("resources"))
}
```

**Step 2: 验证编译**

Run: `cargo check -p rara-paths`
Expected: PASS

**Step 3: Commit**

```bash
git add crates/paths/src/lib.rs
git commit -m "feat(paths): add resources_dir() for tool-produced artifacts"
```

---

### Task 2: `ToolOutput` + `ResourceAttachment` 类型

**Files:**
- Modify: `crates/kernel/src/tool.rs`

**Step 1: 添加新类型**

在 `use` 块后、`AgentToolRef` 定义之前添加：

```rust
/// A binary resource produced by a tool (e.g. a compressed screenshot).
#[derive(Debug, Clone)]
pub struct ResourceAttachment {
    /// MIME type of the resource (e.g. `"image/jpeg"`).
    pub media_type: String,
    /// Raw bytes of the resource (already compressed if applicable).
    pub data: Vec<u8>,
}

/// Output of a tool execution — a JSON result plus optional resource
/// attachments (images, files) that should be persisted separately and
/// fed to the LLM as multimodal content blocks.
#[derive(Debug, Clone)]
pub struct ToolOutput {
    /// JSON payload visible to the LLM as text.
    pub json: serde_json::Value,
    /// Binary resources to persist and inject as multimodal content.
    pub resources: Vec<ResourceAttachment>,
}

impl From<serde_json::Value> for ToolOutput {
    fn from(json: serde_json::Value) -> Self {
        Self {
            json,
            resources: vec![],
        }
    }
}
```

**Step 2: 改 `AgentTool::execute()` 返回类型**

将 trait 定义中的：

```rust
async fn execute(
    &self,
    params: serde_json::Value,
    context: &ToolContext,
) -> anyhow::Result<serde_json::Value>;
```

改为：

```rust
async fn execute(
    &self,
    params: serde_json::Value,
    context: &ToolContext,
) -> anyhow::Result<ToolOutput>;
```

**Step 3: 更新测试中的 `DummyTool`**

同文件底部 `mod tests` 里的 `DummyTool::execute` 返回值改为：

```rust
async fn execute(
    &self,
    _params: serde_json::Value,
    _context: &ToolContext,
) -> anyhow::Result<ToolOutput> {
    Ok(serde_json::json!({"ok": true}).into())
}
```

**Step 4: 验证 kernel crate 本身编译**

Run: `cargo check -p rara-kernel 2>&1 | head -50`
Expected: 大量编译错误来自其他 crate 的 `impl AgentTool`（预期行为，Task 4 修复）

**Step 5: Commit**

```bash
git add crates/kernel/src/tool.rs
git commit -m "feat(kernel): add ToolOutput type and update AgentTool::execute() signature"
```

---

### Task 3: `Message::tool_result_multimodal()`

**Files:**
- Modify: `crates/kernel/src/llm/types.rs`

**Step 1: 添加构造函数**

在现有 `tool_result()` 方法后面添加：

```rust
    pub fn tool_result_multimodal(
        tool_call_id: impl Into<String>,
        blocks: Vec<ContentBlock>,
    ) -> Self {
        Self {
            role:         Role::Tool,
            content:      MessageContent::Multimodal(blocks),
            tool_calls:   Vec::new(),
            tool_call_id: Some(tool_call_id.into()),
        }
    }
```

**Step 2: 验证编译**

Run: `cargo check -p rara-kernel`
Expected: 编译错误仍存在（来自 trait 签名变更），但新方法本身无错

**Step 3: Commit**

```bash
git add crates/kernel/src/llm/types.rs
git commit -m "feat(llm): add Message::tool_result_multimodal() constructor"
```

---

### Task 4: 批量更新所有 `impl AgentTool`（机械替换）

**Files:**
- Modify: 所有包含 `impl AgentTool for` 的文件

全部 33 个实现的 `execute` 方法需要两处改动：
1. 返回类型 `anyhow::Result<serde_json::Value>` → `anyhow::Result<rara_kernel::tool::ToolOutput>`（或 `anyhow::Result<ToolOutput>` 取决于 import）
2. 所有 `Ok(json!(...))` → `Ok(json!(...).into())`
3. 所有 `Ok(serde_json::json!(...))` → `Ok(serde_json::json!(...).into())`
4. 部分工具通过变量返回 `Ok(result)` — 改为 `Ok(result.into())`

**需要改动的文件清单：**

`crates/app/src/tools/` 下：
- `bash.rs` — `BashTool`
- `edit_file.rs` — `EditFileTool`
- `read_file.rs` — `ReadFileTool`
- `write_file.rs` — `WriteFileTool`
- `find_files.rs` — `FindFilesTool`
- `grep.rs` — `GrepTool`
- `http_fetch.rs` — `HttpFetchTool`
- `list_directory.rs` — `ListDirectoryTool`
- `send_email.rs` — `SendEmailTool`
- `composio.rs` — `ComposioTool`
- `screenshot.rs` — `ScreenshotTool`（Task 8 会进一步改造）
- `send_image.rs` — `SendImageTool`
- `settings.rs` — `SettingsTool`
- `skill_tools.rs` — `ListSkillsTool`, `CreateSkillTool`, `DeleteSkillTool`
- `tape_info.rs` — `TapeInfoTool`
- `user_note.rs` — `UserNoteTool`
- `tape_handoff.rs` — `TapeHandoffTool`
- `mita_write_user_note.rs` — `MitaWriteUserNoteTool`
- `mita_read_tape.rs` — `ReadTapeTool`
- `mita_list_sessions.rs` — `ListSessionsTool`
- `mita_dispatch_rara.rs` — `DispatchRaraTool`
- `mcp_tools.rs` — `InstallMcpServerTool`, `ListMcpServersTool`, `RemoveMcpServerTool`

`crates/kernel/src/` 下：
- `schedule_tool.rs` — 5 个 schedule 工具
- `memory/knowledge/tool.rs` — `MemoryTool`

`crates/integrations/mcp/src/` 下：
- `tool_bridge.rs` — `McpToolBridge`

**每个文件的改动模式相同：**

1. 如果文件没有导入 `ToolOutput`，在已有的 `use rara_kernel::tool::` 或同 crate use 中添加
2. `execute` 签名：`-> anyhow::Result<serde_json::Value>` → `-> anyhow::Result<ToolOutput>`
3. 所有 return 点：`Ok(json!(...))` → `Ok(json!(...).into())`，`Ok(value_var)` → `Ok(value_var.into())`

**Step 1: 批量修改所有文件**

逐文件修改，保持每个文件的改动尽量小。

**Step 2: 全量编译验证**

Run: `cargo check 2>&1 | tail -20`
Expected: PASS（或只剩 agent.rs 的编译错误，因为 agent.rs 的消费侧还没改）

**Step 3: Commit**

```bash
git add -A
git commit -m "refactor(tools): migrate all AgentTool impls to ToolOutput return type"
```

---

### Task 5: `ResourceStore` 模块

**Files:**
- Create: `crates/kernel/src/memory/resource.rs`
- Modify: `crates/kernel/src/memory/mod.rs`

**Step 1: 创建 `resource.rs`**

```rust
//! Persistent storage for tool-produced binary resources (images, files).
//!
//! Resources are stored as individual files under a base directory and
//! referenced by [`ResourceRef`] in tape entries.  This keeps the tape
//! JSONL lightweight while preserving full-fidelity binary data.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A reference to a persisted resource file, stored in tape entries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceRef {
    /// Unique identifier for this resource.
    pub id: String,
    /// MIME type (e.g. `"image/jpeg"`).
    pub media_type: String,
    /// Path relative to the resource store base directory.
    pub rel_path: String,
}

/// Persistent file-backed store for binary resources.
#[derive(Debug, Clone)]
pub struct ResourceStore {
    base_dir: PathBuf,
}

impl ResourceStore {
    /// Create a new store rooted at the given directory.
    ///
    /// The directory is created if it does not exist.
    pub async fn new(base_dir: PathBuf) -> std::io::Result<Self> {
        tokio::fs::create_dir_all(&base_dir).await?;
        Ok(Self { base_dir })
    }

    /// Persist a binary resource and return its reference.
    pub async fn store(
        &self,
        media_type: &str,
        data: &[u8],
    ) -> std::io::Result<ResourceRef> {
        let id = Uuid::new_v4().to_string();
        let ext = extension_for_media_type(media_type);
        let filename = format!("{id}.{ext}");
        let path = self.base_dir.join(&filename);
        tokio::fs::write(&path, data).await?;
        Ok(ResourceRef {
            id,
            media_type: media_type.to_owned(),
            rel_path: filename,
        })
    }

    /// Load raw bytes for a previously stored resource.
    pub async fn load(&self, ref_: &ResourceRef) -> std::io::Result<Vec<u8>> {
        tokio::fs::read(self.base_dir.join(&ref_.rel_path)).await
    }

    /// Return the base directory of this store.
    pub fn base_dir(&self) -> &Path {
        &self.base_dir
    }
}

fn extension_for_media_type(media_type: &str) -> &str {
    match media_type {
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        _ => "bin",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn store_and_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = ResourceStore::new(dir.path().to_path_buf()).await.unwrap();

        let data = b"fake jpeg bytes";
        let ref_ = store.store("image/jpeg", data).await.unwrap();

        assert_eq!(ref_.media_type, "image/jpeg");
        assert!(ref_.rel_path.ends_with(".jpg"));

        let loaded = store.load(&ref_).await.unwrap();
        assert_eq!(loaded, data);
    }

    #[test]
    fn extension_mapping() {
        assert_eq!(extension_for_media_type("image/jpeg"), "jpg");
        assert_eq!(extension_for_media_type("image/png"), "png");
        assert_eq!(extension_for_media_type("application/octet-stream"), "bin");
    }
}
```

**Step 2: 注册模块到 `memory/mod.rs`**

在 `mod store;` 后添加：

```rust
pub mod resource;
```

在 `pub use store::FileTapeStore;` 后添加：

```rust
pub use resource::{ResourceRef, ResourceStore};
```

**Step 3: 确认 kernel Cargo.toml 已有 uuid 和 tempfile 依赖**

Run: `grep -E "^(uuid|tempfile)" crates/kernel/Cargo.toml`

如果缺少 `uuid`，需要添加。`tempfile` 只用于 dev-dependencies。

**Step 4: 编译验证**

Run: `cargo check -p rara-kernel`
Expected: PASS

**Step 5: 运行测试**

Run: `cargo test -p rara-kernel -- resource`
Expected: 2 tests PASS

**Step 6: Commit**

```bash
git add crates/kernel/src/memory/resource.rs crates/kernel/src/memory/mod.rs
git commit -m "feat(memory): add ResourceStore for tool-produced binary resources"
```

---

### Task 6: 将 `ResourceStore` 接入 Kernel 和 TapeService

**Files:**
- Modify: `crates/kernel/src/memory/service.rs`
- Modify: `crates/kernel/src/memory/context.rs`
- Modify: `crates/kernel/src/kernel.rs`
- Modify: `crates/app/src/boot.rs`

**Step 1: `TapeService` 持有 `ResourceStore`**

`crates/kernel/src/memory/service.rs`：

在 `TapeService` struct 中添加 `resource_store` 字段：

```rust
pub struct TapeService {
    store: FileTapeStore,
    resource_store: super::resource::ResourceStore,
}
```

更新 `new()` 构造函数：

```rust
pub fn new(store: FileTapeStore, resource_store: super::resource::ResourceStore) -> Self {
    Self { store, resource_store }
}
```

添加 accessor：

```rust
pub fn resource_store(&self) -> &super::resource::ResourceStore {
    &self.resource_store
}
```

**Step 2: `build_llm_context` 传递 `ResourceStore` 到 `default_tape_context`**

`crates/kernel/src/memory/service.rs` 中 `build_llm_context` 的调用改为：

```rust
let mut messages = super::context::default_tape_context(&conv_entries, &self.resource_store)?;
```

同理 `build_llm_context_with_user` 中的调用也要改。

**Step 3: `default_tape_context()` 签名增加 `resource_store` 参数**

`crates/kernel/src/memory/context.rs`：

```rust
pub fn default_tape_context(
    entries: &[TapEntry],
    resource_store: &super::resource::ResourceStore,
) -> TapResult<Vec<Message>> {
```

暂时在函数体内只加 `let _ = resource_store;` 忽略参数（Task 7 会真正使用）。

同时更新 `append_tool_result_entry` 签名接收 `resource_store`，同样暂时忽略。

更新 `mod.rs` 的 re-export 和测试中的调用。

**Step 4: `boot.rs` 初始化 `ResourceStore`**

在创建 `TapeService` 时：

```rust
let resource_store = rara_kernel::memory::ResourceStore::new(
    rara_paths::resources_dir().clone(),
).await.whatever_context("Failed to initialize ResourceStore")?;

let tape_service = rara_kernel::memory::TapeService::new(
    rara_kernel::memory::FileTapeStore::new(rara_paths::memory_dir(), &workspace_path)
        .await
        .whatever_context("Failed to initialize FileTapeStore")?,
    resource_store,
);
```

**Step 5: 编译验证**

Run: `cargo check`
Expected: PASS

**Step 6: 运行现有测试**

Run: `cargo test -p rara-kernel -- tape`
Expected: 所有现有测试 PASS（可能需要更新测试中的 `TapeService::new()` 调用）

**Step 7: Commit**

```bash
git add crates/kernel/src/memory/service.rs crates/kernel/src/memory/context.rs \
       crates/kernel/src/kernel.rs crates/app/src/boot.rs crates/kernel/src/memory/mod.rs
git commit -m "feat(kernel): wire ResourceStore through TapeService and boot"
```

---

### Task 7: Agent loop — resource 持久化 + multimodal message 构建

**Files:**
- Modify: `crates/kernel/src/agent.rs`

**Step 1: 更新工具执行结果类型**

~line 949 的 `tool_futures` 闭包中，`tool.execute()` 现在返回 `ToolOutput`。

更新元组类型从 `(bool, Value, Option<String>, u64)` 到 `(bool, ToolOutput, Option<String>, u64)`：

```rust
// Ok path:
(true, result, None::<String>, dur)  // result is already ToolOutput

// Error paths:
(false, serde_json::json!({"error": e.to_string()}).into(), Some(e.to_string()), dur)
```

**Step 2: Resource 持久化 + tape 写入**

替换 ~line 1010-1023 的 tape 持久化逻辑：

```rust
// Persist resources and build tape payload.
if !results.is_empty() {
    let resource_store = tape.resource_store();
    let mut all_refs: Vec<serde_json::Value> = Vec::new();
    let mut results_json: Vec<serde_json::Value> = Vec::new();
    let mut has_resources = false;

    for (_success, output, _err, _dur) in &results {
        results_json.push(output.json.clone());

        let mut refs = Vec::new();
        for attachment in &output.resources {
            if let Ok(r) = resource_store.store(&attachment.media_type, &attachment.data).await {
                refs.push(serde_json::to_value(&r).unwrap_or_default());
                has_resources = true;
            }
        }
        all_refs.push(serde_json::Value::Array(refs));
    }

    let mut payload = serde_json::json!({ "results": results_json });
    if has_resources {
        payload["__resources"] = serde_json::Value::Array(all_refs);
    }
    let _ = tape.append_tool_result(tape_name, payload, None).await;
}
```

**Step 3: Multimodal message 构建**

替换 ~line 1064 的 `messages.push(llm::Message::tool_result(id, result_str))` 逻辑：

```rust
for ((id, name, args), (idx, (success, output, err, duration_ms))) in
    valid_tool_calls.iter().zip(results.iter().enumerate())
{
    let result_str = output.json.to_string();
    let result_preview = truncate_preview(&result_str, RESULT_PREVIEW_MAX_BYTES);

    // ... existing stream_handle.emit and tool_call_traces logic ...

    // Build LLM message — multimodal if resources exist.
    if output.resources.is_empty() {
        messages.push(llm::Message::tool_result(id, result_str));
    } else {
        let mut blocks = vec![llm::ContentBlock::Text { text: result_str }];
        for attachment in &output.resources {
            let b64 = base64::engine::general_purpose::STANDARD.encode(&attachment.data);
            blocks.push(llm::ContentBlock::ImageBase64 {
                media_type: attachment.media_type.clone(),
                data: b64,
            });
        }
        messages.push(llm::Message::tool_result_multimodal(id, blocks));
    }
}
```

注意：这里用 `attachment.data`（内存中的原始字节）做 base64，不从磁盘重读——resource 持久化是为了 tape 重建时使用。

**Step 4: 确保 `base64` crate 在 kernel 的依赖中**

Run: `grep base64 crates/kernel/Cargo.toml`

如果没有，添加到 `[dependencies]`。

**Step 5: 编译验证**

Run: `cargo check`
Expected: PASS

**Step 6: Commit**

```bash
git add crates/kernel/src/agent.rs crates/kernel/Cargo.toml
git commit -m "feat(agent): persist tool resources and build multimodal messages"
```

---

### Task 8: Context 重建 — 支持 `__resources`

**Files:**
- Modify: `crates/kernel/src/memory/context.rs`

**Step 1: 更新 `append_tool_result_entry` 实现**

```rust
fn append_tool_result_entry(
    messages: &mut Vec<Message>,
    pending_calls: &[PendingCall],
    entry: &TapEntry,
    resource_store: &super::resource::ResourceStore,
) -> TapResult<()> {
    let Some(results) = entry.payload.get("results").and_then(Value::as_array) else {
        return Ok(());
    };

    // Parse resource references if present.
    let resource_refs: Option<Vec<Vec<super::resource::ResourceRef>>> = entry
        .payload
        .get("__resources")
        .and_then(|v| serde_json::from_value(v.clone()).ok());

    for (index, result) in results.iter().enumerate() {
        let content = render_tool_result(result)?;
        let call_id = pending_calls
            .get(index)
            .map(|c| c.id.as_str())
            .unwrap_or("");

        let refs = resource_refs
            .as_ref()
            .and_then(|r| r.get(index))
            .cloned()
            .unwrap_or_default();

        if refs.is_empty() {
            messages.push(Message::tool_result(call_id, content));
        } else {
            let mut blocks = vec![crate::llm::ContentBlock::Text { text: content }];
            for ref_ in &refs {
                // Best-effort: if the resource file is missing, fall back to text-only.
                if let Ok(bytes) = load_resource_sync(resource_store, ref_) {
                    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
                    blocks.push(crate::llm::ContentBlock::ImageBase64 {
                        media_type: ref_.media_type.clone(),
                        data: b64,
                    });
                }
            }
            messages.push(Message::tool_result_multimodal(call_id, blocks));
        }
    }

    Ok(())
}
```

**Step 2: 同步读取 resource 的辅助函数**

`default_tape_context` 目前是同步函数。有两个选择：

A) 改为 async — 但会影响 `build_llm_context` 调用链
B) 用 `std::fs::read` 做同步读取 — 简单但阻塞

推荐 B，因为 resource 文件很小（压缩后 <200KB），同步读取耗时微秒级：

```rust
fn load_resource_sync(
    resource_store: &super::resource::ResourceStore,
    ref_: &super::resource::ResourceRef,
) -> std::io::Result<Vec<u8>> {
    std::fs::read(resource_store.base_dir().join(&ref_.rel_path))
}
```

**Step 3: 更新 `default_tape_context` 中的调用**

在 `TapEntryKind::ToolResult` 分支传递 `resource_store`：

```rust
TapEntryKind::ToolResult => {
    append_tool_result_entry(&mut messages, &pending_calls, entry, resource_store)?;
    pending_calls.clear();
}
```

**Step 4: 更新测试**

`context.rs` 的 `default_tape_context` 测试需要传入 `ResourceStore`。在测试中创建临时目录的 `ResourceStore`：

```rust
fn test_resource_store() -> super::super::resource::ResourceStore {
    // Use a blocking create for test convenience.
    let dir = std::env::temp_dir().join(format!("rara-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    // ResourceStore::new is async, but for tests we can use a workaround:
    // Since we only need the base_dir, and tests don't actually load resources,
    // we can construct it differently. Consider adding a sync constructor or
    // using tokio::test.
    // For now, convert relevant tests to #[tokio::test].
    tokio::runtime::Handle::current().block_on(
        super::super::resource::ResourceStore::new(dir)
    ).unwrap()
}
```

或者更简单：给 `ResourceStore` 加一个 `pub fn new_sync(base_dir: PathBuf) -> std::io::Result<Self>` 供测试用：

```rust
/// Create a new store synchronously. Intended for tests and non-async contexts.
pub fn new_sync(base_dir: PathBuf) -> std::io::Result<Self> {
    std::fs::create_dir_all(&base_dir)?;
    Ok(Self { base_dir })
}
```

**Step 5: 编译验证**

Run: `cargo check -p rara-kernel`
Expected: PASS

**Step 6: 运行测试**

Run: `cargo test -p rara-kernel -- context`
Expected: PASS

**Step 7: Commit**

```bash
git add crates/kernel/src/memory/context.rs crates/kernel/src/memory/resource.rs
git commit -m "feat(context): reconstruct multimodal tool results from resource refs"
```

---

### Task 9: Screenshot 工具改造

**Files:**
- Modify: `crates/app/src/tools/screenshot.rs`

**Step 1: 更新 execute 方法返回 `ToolOutput` with image attachment**

```rust
use rara_kernel::tool::{ToolOutput, ResourceAttachment};

async fn execute(
    &self,
    params: serde_json::Value,
    _context: &rara_kernel::tool::ToolContext,
) -> anyhow::Result<ToolOutput> {
    // ... existing url/selector/width/height/full_page parsing unchanged ...
    // ... existing Playwright invocation unchanged ...

    // Read and compress the screenshot.
    let raw_bytes = tokio::fs::read(&output_path).await
        .map_err(|e| anyhow::anyhow!("failed to read screenshot file: {e}"))?;

    let (compressed, media_type) = rara_kernel::llm::image::compress_image(
        &raw_bytes,
        rara_kernel::llm::image::DEFAULT_MAX_EDGE,
        rara_kernel::llm::image::DEFAULT_QUALITY,
    )?;

    // Clean up temporary file.
    let _ = tokio::fs::remove_file(&output_path).await;

    Ok(ToolOutput {
        json: json!({
            "success": true,
            "description": format!("screenshot of {url}"),
        }),
        resources: vec![ResourceAttachment {
            media_type,
            data: compressed,
        }],
    })
}
```

**Step 2: 移除不再需要的 `uuid` import**（如果 uuid 只用于生成文件名）

检查 — 如果 `Uuid` 只用于 output_path 的临时文件名，仍然需要保留（临时文件名生成还在）。

**Step 3: 编译验证**

Run: `cargo check`
Expected: PASS

**Step 4: Commit**

```bash
git add crates/app/src/tools/screenshot.rs
git commit -m "feat(screenshot): return compressed image as ToolOutput resource"
```

---

### Task 10: 全量验证

**Step 1: 完整编译**

Run: `cargo check`
Expected: PASS

**Step 2: 运行所有测试**

Run: `cargo test`
Expected: PASS

**Step 3: 手动验证链路（如果有运行环境）**

1. 启动 rara
2. 触发一个截图任务
3. 检查 `~/.local/share/rara/resources/` 下是否生成了 `.jpg` 文件
4. 检查 tape JSONL 中是否包含 `__resources` 字段
5. 确认 LLM 收到了 multimodal 消息（通过 trace log）

**Step 4: Final commit（如果有遗漏修复）**

```bash
git add -A
git commit -m "chore: final adjustments for tool-result multimodal pipeline"
```
