# KernelHandle 去重：让 Handle 持有 Arc<Kernel>

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 消除 `KernelHandle` 与 `Kernel` 之间的 14 个重复字段和 5 个重复方法，让 `KernelHandle` 变成 `Arc<Kernel>` 的薄包装。

**Architecture:** `KernelHandle` 从持有 14 个独立 `Arc` 字段改为持有单一 `Arc<Kernel>`。所有只读访问器委托给 `Kernel`，写操作通过 `self.kernel.event_queue()` 路由。`Kernel::start()` 返回值从 `(Arc<Kernel>, KernelHandle)` 简化为 `KernelHandle`。

**Tech Stack:** Rust, tokio, Arc

---

## 影响范围

| 文件 | 改动类型 |
|------|----------|
| `crates/kernel/src/handle/kernel_handle.rs` | **重写** — 14 字段 → 1 字段 |
| `crates/kernel/src/kernel.rs` | **修改** — `handle()` 简化，`start()` 返回值变化 |
| `crates/app/src/lib.rs` | **适配** — `start()` 返回值解构 |
| `crates/kernel/src/kernel.rs` (tests) | **适配** — 测试中的 `start()` 调用 |
| `crates/kernel/src/event_loop/lifecycle.rs` (tests) | **适配** — 测试中的解构 |

**不需要改动的文件**（API 不变）：
- `crates/channels/src/telegram/adapter.rs` — 只用 `KernelHandle` 的方法
- `crates/channels/src/web.rs` — 同上
- `crates/extensions/backend-admin/` — 同上
- `crates/cmd/src/main.rs` — 同上
- `crates/boot/src/kernel.rs` — 同上

---

### Task 1: 重写 KernelHandle 结构体

**Files:**
- Modify: `crates/kernel/src/handle/kernel_handle.rs`

**Step 1: 替换结构体定义**

把 14 字段的结构体替换为：

```rust
#[derive(Clone)]
pub struct KernelHandle {
    kernel: Arc<Kernel>,
}
```

**Step 2: 重写 `new()` 构造函数**

```rust
impl KernelHandle {
    pub(crate) fn new(kernel: Arc<Kernel>) -> Self {
        Self { kernel }
    }
```

**Step 3: 添加 `kernel()` 访问器**

```rust
    /// 访问底层 Kernel 引用。
    pub fn kernel(&self) -> &Kernel {
        &self.kernel
    }
```

**Step 4: 重写所有只读访问器为委托**

将 12 个只读访问器改为委托调用，例如：

```rust
    pub fn process_table(&self) -> &Arc<ProcessTable> {
        // Kernel::process_table() 返回 &ProcessTable，这里需要返回 &Arc<ProcessTable>
        // 但由于 Kernel 内部字段是 Arc<ProcessTable>，直接暴露引用即可
        self.kernel.process_table_arc()
    }

    pub fn agent_registry(&self) -> &AgentRegistryRef { self.kernel.agent_registry() }
    pub fn tool_registry(&self) -> &ToolRegistryRef { self.kernel.tool_registry() }
    pub fn stream_hub(&self) -> &StreamHubRef { self.kernel.stream_hub() }
    pub fn endpoint_registry(&self) -> &EndpointRegistryRef { self.kernel.endpoint_registry() }
    pub fn ingress_pipeline(&self) -> &IngressPipelineRef { self.kernel.ingress_pipeline() }
    pub fn audit(&self) -> &AuditRef { self.kernel.audit() }
    pub fn settings(&self) -> &SettingsRef { self.kernel.settings() }
    pub fn security(&self) -> &SecurityRef { self.kernel.security() }
    pub fn config(&self) -> &KernelConfig { self.kernel.config() }
    pub fn event_queue(&self) -> &EventQueueRef { self.kernel.event_queue() }
    pub fn device_registry(&self) -> &DeviceRegistryRef { self.kernel.device_registry() }
```

注意：`KernelHandle::process_table()` 当前返回 `&Arc<ProcessTable>`，而 `Kernel::process_table()` 返回 `&ProcessTable`。需要在 `Kernel` 上新增一个 `process_table_arc()` 方法返回 `&Arc<ProcessTable>`，或者将 `KernelHandle::process_table()` 的返回类型改为 `&ProcessTable`。由于外部调用者（`backend-admin`）用 `.process_table().get(id)` 和 `.process_table().list()`，`ProcessTable` 本身的方法不需要 `Arc`，所以改返回 `&ProcessTable` 更干净。但需要验证没有调用者依赖 `Arc::clone(handle.process_table())`。

**决策**：在 `Kernel` 上新增 `pub fn process_table_arc(&self) -> &Arc<ProcessTable>` 以保持 API 兼容。

**Step 5: 删除 5 个重复的查询方法，改为委托**

```rust
    pub async fn process_stats(&self, agent_id: &AgentId) -> Option<ProcessStats> {
        self.kernel.process_stats(agent_id).await
    }

    pub async fn list_processes(&self) -> Vec<ProcessStats> {
        self.kernel.list_processes().await
    }

    pub fn system_stats(&self) -> SystemStats {
        self.kernel.system_stats()
    }

    pub fn get_process_turns(&self, agent_id: AgentId) -> Vec<TurnTrace> {
        self.kernel.get_process_turns(agent_id)
    }

    pub async fn audit_query(&self, filter: AuditFilter) -> Vec<AuditEvent> {
        self.kernel.audit_query(filter).await
    }
```

**Step 6: 写操作方法保持不变（仅改字段引用）**

`spawn_with_input`, `spawn_named`, `send_signal`, `ingest`, `submit_message`, `shutdown` 中的 `self.event_queue` 改为 `self.kernel.event_queue()`，`self.agent_registry` 改为 `self.kernel.agent_registry()` 等。

**Step 7: 更新 Debug 实现**

```rust
impl std::fmt::Debug for KernelHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KernelHandle")
            .field("event_queue_pending", &self.kernel.event_queue().pending_count())
            .finish()
    }
}
```

**Step 8: 验证编译**

Run: `cargo check -p rara-kernel`
Expected: 可能有编译错误需要在 Task 2 中修复

---

### Task 2: 修改 Kernel 结构体适配

**Files:**
- Modify: `crates/kernel/src/kernel.rs`

**Step 1: 新增 `process_table_arc()` 方法**

在 `Kernel` 的 impl 块中添加：

```rust
    /// Access the process table as Arc (for KernelHandle API compatibility).
    pub fn process_table_arc(&self) -> &Arc<ProcessTable> { &self.process_table }
```

**Step 2: 简化 `handle()` 方法**

由于 `handle()` 需要 `Arc<Kernel>` 但 `&self` 不提供，有两种选择：
- 删除 `handle()` 方法（只在 `start()` 中构造）
- 保留但签名改为需要 Arc

选择删除，因为 `handle()` 只在 `start()` 和测试中调用：

删除 `Kernel::handle()` 方法。

**Step 3: 修改 `start()` 返回值**

```rust
    pub fn start(self, cancel_token: CancellationToken) -> KernelHandle {
        let kernel = Arc::new(self);
        let handle = KernelHandle::new(kernel.clone());

        tokio::spawn({
            let k = kernel;
            let token = cancel_token;
            async move {
                Kernel::run_event_loop_arc(k, token).await;
            }
        });

        info!("kernel event loop started");
        handle
    }
```

**Step 4: 验证 kernel crate 内部编译**

Run: `cargo check -p rara-kernel`
Expected: PASS（可能测试需要在 Task 3 修复）

---

### Task 3: 修复 kernel crate 内部测试

**Files:**
- Modify: `crates/kernel/src/kernel.rs` (tests module)
- Modify: `crates/kernel/src/event_loop/lifecycle.rs` (tests module)

**Step 1: 修复 `kernel.rs` 中的 `start_test_kernel()`**

当前：
```rust
fn start_test_kernel(...) -> (KernelHandle, CancellationToken) {
    let kernel = make_test_kernel(max_concurrency, child_limit);
    let cancel = CancellationToken::new();
    let (_arc, handle) = kernel.start(cancel.clone());
    (handle, cancel)
}
```

改为：
```rust
fn start_test_kernel(...) -> (KernelHandle, CancellationToken) {
    let kernel = make_test_kernel(max_concurrency, child_limit);
    let cancel = CancellationToken::new();
    let handle = kernel.start(cancel.clone());
    (handle, cancel)
}
```

**Step 2: 修复 `kernel.rs` 中 `make_guarded_kernel` 测试**

当前：
```rust
let (_kernel, handle) = kernel.start(cancel.clone());
```

改为：
```rust
let handle = kernel.start(cancel.clone());
```

**Step 3: 验证 kernel 测试通过**

Run: `cargo test -p rara-kernel`
Expected: PASS

**Step 4: Commit**

```bash
git add crates/kernel/
git commit -m "refactor(kernel): KernelHandle holds Arc<Kernel> instead of 14 separate Arc fields"
```

---

### Task 4: 适配外部调用者

**Files:**
- Modify: `crates/app/src/lib.rs`

**Step 1: 修复 `app/src/lib.rs` 中 `start()` 调用**

当前（约第 313 行）：
```rust
let (_kernel_arc, kernel_handle) = kernel.start(cancellation_token.clone());
```

改为：
```rust
let kernel_handle = kernel.start(cancellation_token.clone());
```

**Step 2: 验证整个工程编译**

Run: `cargo check`
Expected: PASS

**Step 3: Commit**

```bash
git add crates/app/src/lib.rs
git commit -m "refactor(app): adapt to simplified Kernel::start() return type"
```

---

### Task 5: 最终验证

**Step 1: 运行完整测试套件**

Run: `cargo test -p rara-kernel`
Expected: All tests PASS

**Step 2: 运行工程级编译检查**

Run: `cargo check`
Expected: PASS

**Step 3: 验证前端编译（如果有前端改动）**

不需要 — 本次改动纯后端。
