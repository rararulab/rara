# Plan-Execute Architecture Design

**Date**: 2026-03-13
**Status**: Draft
**Phase 1 Prerequisites**: #242, #243, #244, #245, #246 (all completed)

## 背景

rara 当前是纯 reactive 架构：每轮迭代从 tape 重建全部消息 → 调 LLM → 执行 tool → 结果写 tape → 下一轮。复杂任务中 tool 结果不断堆积，上下文快速膨胀。

Plan-Execute 架构将复杂任务分解为：**Plan（规划）→ Execute（逐步执行）→ Replan（条件修正）**，每步执行有独立的精简上下文。

## 设计决策

| 决策点 | 选择 | 理由 |
|--------|------|------|
| Step 结构 | 顺序列表，无 DAG | 业界主流（LangGraph 等）；replan 比 DAG 更有价值 |
| Replan 触发 | 条件触发（失败/意外） | 多数步骤按计划走，不需要每步消耗 planner call |
| PlanState 存储 | Tape（TapEntryKind::Plan） | 复用现有基础设施，plan 本身是对话上下文一部分 |
| 触发方式 | LLM 自主判断 + 用户 /plan | 零摩擦 + 显式控制两条路 |
| Inline/Worker 判断 | Planner 决定 | Planner 有全局视角，实现简单 |
| v1/v2 共存 | 消息 tag 路由，不改 reactive loop | 零风险，独立迭代，灰度切换 |

## 核心数据结构

```rust
/// Plan 中间表示，存入 tape
struct Plan {
    goal: String,
    steps: Vec<PlanStep>,
    past_steps: Vec<PastStep>,
    status: PlanStatus,  // Active | Completed | Failed | Replanned
}

struct PlanStep {
    index: usize,
    task: String,            // 自然语言描述
    mode: ExecutionMode,     // Inline | Worker
    acceptance: String,      // 完成条件
}

struct PastStep {
    index: usize,
    task: String,
    summary: String,         // executor 摘要（不是完整输出）
    outcome: StepOutcome,    // Success | Failed { reason } | NeedsReplan { reason }
}

enum ExecutionMode {
    Inline,                  // 主 agent loop 中执行
    Worker,                  // spawn 独立 session
}
```

## 可观测性：新增 StreamEvent

```rust
enum StreamEvent {
    // ... 现有的 TextDelta, ToolCallStart/End 等不变 ...

    PlanCreated { goal: String, steps: Vec<String> },
    PlanStepStart { index: usize, task: String, mode: String },
    PlanStepEnd { index: usize, outcome: String, summary: String },
    PlanReplan { reason: String, new_steps: Vec<String> },
    PlanCompleted { summary: String },
}
```

## Telegram 渲染

复用现有 progress message 模式 — 一条独立的 Plan 进度消息持续 editMessageText：

```
📋 Plan: 帮用户重构 auth 模块 (3/5)

✅ 1. 分析现有 auth 代码结构 (8.2s)
✅ 2. 设计新的 middleware 接口 (12.1s)
🔧 3. 实现 JWT 验证层... 23.4s
⬚ 4. 迁移现有路由
⬚ 5. 添加集成测试
```

与现有 tool progress 消息并行显示，遵循 1.5s 节流和 Telegram rate limit 规则。

## v1/v2 路由机制

v1（reactive）和 v2（plan-execute）完全共存，通过消息 tag 在 kernel 层路由：

```
用户消息到达 kernel.start_llm_turn()
    │
    ├─ v1 (默认) → run_agent_loop()      // 现有 reactive，一行不改
    │
    └─ v2 (/plan 或 tag) → run_plan_loop()  // 新的 plan-execute 循环
```

### 路由规则

| 条件 | 路由 |
|------|------|
| 用户发送 `/plan ...` 指令 | v2（本次消息） |
| 用户通过 `/msg_version 2` 切换 | v2（会话级持久切换） |
| InboundMessage 携带 `execution_mode: "plan"` tag | v2 |
| AgentManifest 配置 `default_execution_mode: "plan"` | v2 |
| 其他所有情况 | v1 |

### /msg_version 命令

```
/msg_version 1    — 切回 reactive 模式
/msg_version 2    — 切到 plan-execute 模式
/msg_version      — 查看当前版本
```

- **作用域**: 会话级（session），存在 Session 元数据中
- **持久性**: 切换后该 session 内后续所有消息都走对应版本，直到再次切换
- **优先级**: `/plan` 指令 > session msg_version > manifest 默认值

### 共享基础设施

v1 和 v2 共享所有底层原语，不重复实现：

- **TapeService** — 同一个 tape，v2 多一个 `TapEntryKind::Plan`
- **ToolRegistry** — 同一套 tool，v2 额外注册 `create_plan`
- **StreamHub** — 同一套 stream，v2 多 emit PlanXxx 事件
- **Guard pipeline** — 同一套安全检查
- **KernelHandle::spawn_child** — v2 的 worker 步骤直接复用

### 灰度策略

1. **初期**: 仅 `/plan` 显式触发 v2
2. **验证期**: 特定 agent（如 worker）默认 v2
3. **收敛期**: rara 主 agent 支持 LLM 自主判断触发 v2
4. **最终**: 评估是否用 v2 完全替代 v1

## 执行流程（v2: run_plan_loop）

```
用户消息到达（v2 路由）
    │
    ▼
    LLM 调用 create_plan tool（提供完整 goal + steps）
         │
         ▼
    ┌─ Plan 阶段 ─────────────────────┐
    │  LLM 输出结构化 Plan JSON        │
    │  写入 tape (TapEntryKind::Plan)  │
    │  emit PlanCreated event          │
    └──────────┬───────────────────────┘
               │
               ▼
    ┌─ Execute Loop ──────────────────────────────────┐
    │  for step in plan.steps:                         │
    │    emit PlanStepStart                            │
    │                                                  │
    │    if step.mode == Inline:                        │
    │      主 agent 执行（普通 tool call 循环）          │
    │      结果写入 tape，提取 summary                  │
    │      上下文仅含: system prompt + plan概览          │
    │               + 当前step goal + acceptance        │
    │                                                  │
    │    if step.mode == Worker:                        │
    │      spawn worker (独立 tape)                     │
    │      等待完成，收 summary（截断逻辑复用 #244）      │
    │                                                  │
    │    emit PlanStepEnd                               │
    │    past_steps.push(summary)                       │
    │                                                  │
    │    ── Replan 检查 ──                              │
    │    if outcome == Failed || NeedsReplan:           │
    │      用 past_steps + 剩余 steps 调 LLM replan    │
    │      emit PlanReplan                              │
    │      替换 plan.steps，继续循环                     │
    └──────────┬──────────────────────────────────────┘
               │
               ▼
    emit PlanCompleted
    LLM 生成最终总结回复用户
```

## Tool 接口

新增 2 个 tool 注册到 ToolRegistry：

### create_plan

- **触发**: LLM 自主调用或 `/plan` 指令转换
- **输入**: `{ goal: String, steps: Vec<{ task: String, mode: "inline" | "worker", acceptance: String }> }` — LLM 在 tool arguments 中提供完整 plan 结构，tool 只做验证和存储
- **输出**: Plan JSON（含自动分配的 index）
- **副作用**: 写入 tape, emit PlanCreated, agent loop 进入 plan-execute 子循环

### replan

- **触发**: 仅在 plan-execute 流程内部，条件触发
- **输入**: `{ past_steps, failure_reason, remaining_steps }`
- **输出**: 新的 steps 列表
- **副作用**: 更新 tape 中的 Plan, emit PlanReplan

不需要 `execute_step` tool — step 执行由 kernel 驱动，LLM 只负责 plan 和 replan。

## 改动点清单

| 文件 | 改动 |
|------|------|
| `kernel/src/memory/mod.rs` | 新增 `TapEntryKind::Plan` |
| `kernel/src/io.rs` | 新增 5 个 PlanXxx StreamEvent |
| `kernel/src/plan.rs` | **新文件** — `run_plan_loop()` + `PlanExecutor`：驱动 plan 生成、step 执行、replan 判断、event emit |
| `kernel/src/kernel.rs` | `start_llm_turn` 加 v1/v2 路由分支 |
| `kernel/src/tool.rs` | 注册 `create_plan` tool |
| `channels/src/telegram/adapter.rs` | 渲染 PlanXxx 事件为进度消息 |
| `channels/src/web.rs` | 转发 PlanXxx 事件到前端 |
| `agents/src/lib.rs` | rara manifest tools 加入 `create_plan` |

## 不动的部分

- **`run_agent_loop()` 一行不改** — v2 是独立的 `run_plan_loop()`，通过 kernel 路由
- TapeService / TapeStore — 只加新 entry kind
- Worker spawn 机制 — 直接复用 `KernelHandle::spawn_child`
- Guard pipeline — plan tool 走正常 guard 流程
- 所有 v1 路径完全不受影响
