# Scheduled Tasks Design — 时间轮驱动的定时任务系统

> Date: 2026-03-07

## 概述

为 Rara 内核添加定时任务能力。定时任务作为 `KernelEvent` 集成到现有事件系统中——到期任务注入为 `UserMessage`，复用完整 agent loop（LLM 推理 + 工具调用）。

参考 bub 项目的 APScheduler 设计（三种触发器、session 绑定、工具 API），但采用 Rust 原生方案：timing-wheel crate + crossbeam 无锁风格 + 内核事件循环 tick。

## 核心决策

| 决策项 | 选择 | 理由 |
|--------|------|------|
| 执行方式 | 注入 UserMessage，走 agent loop | agent-first，复用现有 session 基础设施 |
| 触发器类型 | once / interval / cron 全支持 | interval 对 LLM 比 cron 更自然 |
| 持久化 | JSON 文件 | 任务数量少，不需要 SQLite |
| 调度机制 | 时间轮 + 内核 tick | 融入事件循环，不需额外线程 |
| 时间轮实现 | `timing-wheel` crate | 轻量 binary heap，API 匹配 tick+drain 模式 |
| tick 间隔 | 1 秒固定 | 定时任务不需要毫秒级精度 |
| 工具注册 | Syscall 路径 | 和 MemStore/GetToolRegistry 一致 |

## 数据模型

```rust
// crates/kernel/src/schedule.rs

base::define_id!(JobId);

/// 触发器类型
pub enum Trigger {
    /// 一次性延迟任务
    Once { run_at: Timestamp },
    /// 固定间隔周期任务
    Interval { every: Duration, next_at: Timestamp },
    /// Cron 表达式
    Cron { expr: String, next_at: Timestamp },
}

/// 时间轮中的一个条目
pub struct JobEntry {
    pub id: JobId,
    pub trigger: Trigger,
    pub message: String,          // 注入为 UserMessage 的文本
    pub session_key: SessionKey,  // 绑定到哪个 session
    pub principal: Principal,     // 谁创建的
    pub created_at: Timestamp,
}
```

- `JobEntry` 序列化为 JSON 存 `~/.config/rara/jobs.json`
- `Trigger` 每个变体都带 `next_at`，方便时间轮快速比较

## 时间轮

使用 `timing-wheel` crate（binary heap 实现）。

```rust
pub struct TimingWheel {
    wheel: timing_wheel::TimingWheel<JobEntry>,
    path: PathBuf,  // ~/.config/rara/jobs.json
}

impl TimingWheel {
    /// 启动时从 JSON 恢复
    pub fn load(path: PathBuf) -> Self;

    /// tick: 返回所有到期的 job
    /// Once → 移除；Interval/Cron → 计算下一个 next_at 放回
    pub fn drain_expired(&mut self) -> Vec<JobEntry>;

    pub fn add(&mut self, entry: JobEntry);
    pub fn remove(&mut self, id: &JobId) -> Option<JobEntry>;
    pub fn list(&self, session_key: Option<SessionKey>) -> Vec<&JobEntry>;

    /// 持久化到 JSON 文件
    fn persist(&self);
}
```

## Syscall 扩展

新增三个 Syscall 变体：

```rust
pub enum Syscall {
    // ... 现有变体

    RegisterJob {
        trigger: Trigger,
        message: String,
        reply_tx: oneshot::Sender<Result<JobId>>,
    },

    RemoveJob {
        job_id: JobId,
        reply_tx: oneshot::Sender<Result<()>>,
    },

    ListJobs {
        reply_tx: oneshot::Sender<Result<Vec<JobEntry>>>,
    },
}
```

`session_key` 和 `principal` 从 `SyscallEnvelope` 和 session 上下文中获取。

## 工具 API

三个 AgentTool，LLM 可直接调用：

### `schedule.add`

```json
{
  "after_seconds": 300,        // 三选一
  "interval_seconds": 3600,    // 三选一
  "cron": "0 9 * * *",         // 三选一
  "message": "检查 PR #123 的状态"
}
```

返回：`{ "job_id": "xxx", "next_run": "2026-03-07T15:00:00Z" }`

### `schedule.remove`

```json
{ "job_id": "xxx" }
```

### `schedule.list`

无参数。返回当前 session 绑定的所有定时任务。

## 内核事件循环改造

`Kernel` 新增字段：

```rust
pub struct Kernel {
    // ... 现有字段
    timing_wheel: TimingWheel,
}
```

Global processor select 循环增加 tick 分支：

```rust
async fn run_global_processor(kernel, queue, shutdown) {
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    loop {
        tokio::select! {
            _ = queue.wait() => {
                // 正常事件处理（不变）
            }
            _ = tick.tick() => {
                for job in kernel.timing_wheel.drain_expired() {
                    queue.push(KernelEventEnvelope::user_message(
                        InboundMessage::scheduled(
                            job.message,
                            job.session_key,
                            job.principal,
                        )
                    ));
                }
            }
            _ = shutdown.cancelled() => {
                kernel.timing_wheel.persist();
                break;
            }
        }
    }
}
```

Syscall 处理：

```rust
Syscall::RegisterJob { trigger, message, reply_tx } => {
    let entry = JobEntry::new(trigger, message, session_key, principal);
    let id = entry.id.clone();
    kernel.timing_wheel.add(entry);
    kernel.timing_wheel.persist();
    let _ = reply_tx.send(Ok(id));
}
Syscall::RemoveJob { job_id, reply_tx } => {
    let result = kernel.timing_wheel.remove(&job_id);
    kernel.timing_wheel.persist();
    let _ = reply_tx.send(result);
}
Syscall::ListJobs { reply_tx } => {
    let jobs = kernel.timing_wheel.list(Some(session_key));
    let _ = reply_tx.send(Ok(jobs));
}
```

## 数据流

```
LLM 调用 schedule.add 工具
  → ToolContext.event_queue.push(SessionCommand(Syscall::RegisterJob))
  → Kernel handle_event → SyscallDispatcher
  → timing_wheel.add(entry) + persist()
  → reply_tx.send(Ok(job_id))

每 1 秒 tick:
  → timing_wheel.drain_expired()
  → 到期 job → KernelEventEnvelope::user_message(InboundMessage::scheduled(...))
  → push 到 event queue
  → 正常 agent loop: session 查找/创建 → LLM 推理 → 工具调用 → 回复

  Once 任务: drain 后不放回
  Interval 任务: 计算 next_at = now + every, 放回时间轮
  Cron 任务: 根据 cron 表达式计算下一个触发时间, 放回时间轮
```

## InboundMessage 扩展

新增 `scheduled` 构造器，标记消息来源为定时任务：

```rust
impl InboundMessage {
    pub fn scheduled(message: String, session_key: SessionKey, principal: Principal) -> Self {
        Self {
            session_key: Some(session_key),
            text: message,
            sender: principal.user_id(),
            source: MessageSource::Scheduled,  // 新增枚举值
            ..
        }
    }
}
```

## 文件清单

| 文件 | 变更 |
|------|------|
| `crates/kernel/src/schedule.rs` | 新增：JobId, Trigger, JobEntry, TimingWheel |
| `crates/kernel/src/event.rs` | 新增 Syscall 变体：RegisterJob, RemoveJob, ListJobs |
| `crates/kernel/src/kernel.rs` | Kernel 新增 timing_wheel 字段；global processor 增加 tick 分支 |
| `crates/kernel/src/syscall.rs` | SyscallDispatcher 处理新 syscall |
| `crates/kernel/src/tool/schedule.rs` | 新增：schedule.add / remove / list 三个 AgentTool |
| `crates/kernel/src/io/message.rs` | InboundMessage 新增 scheduled 构造器 + MessageSource::Scheduled |
| `crates/kernel/Cargo.toml` | 新增依赖：timing-wheel, cron (解析 cron 表达式) |
| `~/.config/rara/jobs.json` | 运行时：定时任务持久化存储 |
