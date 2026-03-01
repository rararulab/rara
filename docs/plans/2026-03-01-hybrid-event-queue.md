# Hybrid Event Queue Design

**Date:** 2026-03-01
**Status:** Approved

## Overview

将 kernel 的纯内存 EventQueue 升级为 hybrid 队列，兼顾性能（内存快速路径）和持久化（文件系统 WAL），系统重启后未完成的事件能继续处理。

## 设计决策

| 决策 | 选择 | 理由 |
|------|------|------|
| 持久化后端 | 文件系统 WAL | 顺序 I/O 性能高，不增加 PG 负载 |
| 序列化格式 | JSON lines | 开发阶段友好，可人工阅读调试 |
| Segment 策略 | 单文件 + truncate | 单进程内核，事件量不大，实现简单 |
| 持久化范围 | 选择性 | 只持久化 UserMessage / SpawnAgent / Timer |
| 完成标记 | Completion marker | 支持乱序完成，单文件自包含 |

## Crate 结构

```
crates/core/queue/              # rara-queue crate
  src/
    lib.rs                      # pub mod memory, wal, hybrid
    memory.rs                   # MemoryQueue (现有 EventQueue 逻辑搬过来)
    wal.rs                      # WalQueue — append-only log + completion marker
    hybrid.rs                   # HybridQueue — MemoryQueue + WalQueue 组合

crates/core/kernel/
  src/
    event_queue.rs              # EventQueue trait 定义 (不再有具体实现)
```

**依赖方向：**
```
kernel  ← 定义 EventQueue trait + KernelEvent 类型
queue   → 实现 trait (依赖 kernel)
boot    → 组装: 创建 HybridQueue 注入 kernel
```

## 核心类型

### EventQueue Trait (kernel 侧)

```rust
// kernel/src/event_queue.rs

#[async_trait]
pub trait EventQueue: Send + Sync + 'static {
    async fn push(&self, event: KernelEvent) -> Result<(), BusError>;
    fn try_push(&self, event: KernelEvent) -> Result<(), BusError>;
    async fn drain(&self, max: usize) -> Vec<KernelEvent>;
    async fn wait(&self);
    fn pending_count(&self) -> usize;
    fn mark_completed(&self, wal_id: u64);
}
```

### WAL Entry

```rust
struct WalEntry {
    id: u64,
    ts: String,              // ISO 8601
    kind: WalEntryKind,
}

enum WalEntryKind {
    Event(KernelEvent),
    Completed { event_id: u64 },
}
```

### WalQueue

```rust
pub struct WalQueue {
    path: PathBuf,
    next_id: AtomicU64,
    file: Mutex<BufWriter<File>>,
    pending: Mutex<BTreeMap<u64, KernelEvent>>,
}
```

**操作：**
- `append(event)` — 分配 ID → JSON 序列化 → 追加写入 → 插入 pending → 返回 ID
- `mark_completed(id)` — 追加 Completed 记录 → 从 pending 移除
- `recover()` — 逐行读取 → 重建 pending map → 返回未完成事件列表
- `truncate()` — 读取 pending → 重写新文件 → 原子 rename

### HybridQueue

```rust
pub struct HybridQueue {
    memory: MemoryQueue,
    wal: WalQueue,
    truncate_interval: Duration,
}
```

**写入流程：**
1. 判断事件是否需要持久化（UserMessage / SpawnAgent / Timer）
2. 需持久化 → 先写 WAL 拿到 `wal_id` → 推入 MemoryQueue
3. 不需持久化 → 直接推入 MemoryQueue

**读取流程：** 完全委托 MemoryQueue

**完成标记：** event_loop 处理完事件后检查 wal_id，有值调用 mark_completed()

**启动恢复：** WalQueue::recover() → 未完成事件推入 MemoryQueue → 正常启动

## WAL 文件格式

路径：`{data_dir}/wal/events.jsonl`

```jsonl
{"id":1,"ts":"2026-03-01T10:00:00Z","kind":{"Event":{"UserMessage":{...}}}}
{"id":2,"ts":"2026-03-01T10:00:01Z","kind":{"Completed":{"event_id":1}}}
```

## Truncate 策略

- 周期性（每 5 分钟）或文件超过阈值（10MB）
- 后台 tokio::spawn，CancellationToken 优雅关闭
- 实现：读取 pending map → 写入 temp 文件 → 原子 rename

## 崩溃安全

- 追加写入天然安全，最坏丢失最后一条不完整行
- recover() 遇到不完整 JSON 行跳过
- truncate 使用 write-to-temp + rename 原子替换

## Kernel 侧改动

1. `EventQueue` 从 struct 变为 trait
2. `KernelEvent` 需要持久化的变体添加 `wal_id: Option<u64>`（或用 wrapper）
3. `event_loop.rs` 处理完事件后调用 `mark_completed()`
4. `Kernel::new()` 接受 `Arc<dyn EventQueue>` 而非具体类型
5. 需要持久化的类型添加 Serialize/Deserialize：InboundMessage、AgentManifest、Principal

## 测试计划

**MemoryQueue：** 搬迁现有全部测试

**WalQueue（tempdir，无需 testcontainers）：**
- append + recover
- completed events excluded from recovery
- truncate compaction
- corrupt line skipped

**HybridQueue：**
- non-persistent event skips WAL
- persistent event writes WAL
- restart recovery
- truncate background task
