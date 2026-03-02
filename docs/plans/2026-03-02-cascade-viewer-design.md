# Cascade Viewer — Agent 执行追踪可视化

**Date**: 2026-03-02
**Location**: KernelTop 页面，替换/增强现有 TurnTraceTree

## 概述

在 KernelTop 进程监控面板中，展开进程后以 Cascade（瀑布/级联）树形视图展示 Agent 的 ReAct 执行链：
User Input → Thought → Action → Observation → Response，支持历史轮询 + 活跃进程实时流式。

## 数据模型

### 后端扩展

**IterationTrace 新增字段：**
- `reasoning_text: Option<String>` — 完整推理文本（用于 Thought 节点）

**TurnTrace 新增字段：**
- `input_text: Option<String>` — 触发该 turn 的用户消息

### Cascade 节点类型

| 类型 | 数据来源 | 图标 | 展开内容 |
|------|---------|------|---------|
| User Input | `TurnTrace.input_text` | MessageSquare | 用户原始文本 |
| Thought | `IterationTrace.reasoning_text` | Brain | 完整推理 Markdown |
| Action | `ToolCallTrace` | Wrench | 工具名 + 参数 JSON |
| Observation | `ToolCallTrace.result_preview` | Eye | 结果 + 耗时 + 状态 |
| Response | `IterationTrace.text_preview` | Bot | 回复 Markdown |

## 前端组件结构

```
KernelTop.tsx
└── ProcessRow（展开后）
    └── <CascadeViewer turns={turnTraces} streamEvents={...} />
        ├── TurnGroup (TICK 1)
        │   ├── CascadeNode type="input"
        │   ├── CascadeNode type="thought"
        │   ├── CascadeNode type="action"
        │   ├── CascadeNode type="observation"
        │   └── CascadeNode type="response"
        └── TurnGroup (TICK N, 活跃 — 流式)
            ├── CascadeNode type="thought" streaming
            └── CascadeNode type="action" streaming
```

### CascadeNode 统一结构

- **折叠头部**：类型图标 + 标签 + 摘要（单行截断）+ 耗时 Badge
- **展开体**：Thought/Response → Markdown；Action → JSON pre；Observation → 结果 pre

### 样式

- 左侧竖线（`border-l-2`）树状层级
- 类型颜色：Thought 紫色、Action 蓝色、Observation 绿色、Response 灰色
- shadcn `Collapsible` + `Badge` + `ScrollArea`
- 流式节点：脉冲动画边框（`animate-pulse`）

## 数据流

### 历史数据（轮询）

`GET /api/v1/kernel/processes/{agent_id}/turns` → `Vec<TurnTrace>`
- 已有端点，新增 `reasoning_text` 和 `input_text` 字段

### 活跃进程（流式）

新增 WebSocket 端点：
```
GET /api/v1/kernel/processes/{agent_id}/stream  (WebSocket upgrade)
```
- 查 ProcessTable 获取 session_id → 订阅 StreamHub
- 转发 StreamEvent（TextDelta, ReasoningDelta, ToolCallStart/End, TurnMetrics, Done）

### 前端混合策略

```
展开进程行
  ├── fetch /turns → 历史 TurnTrace[] → 渲染 TurnGroups
  └── if state ∈ {Running, Idle}
      └── ws /processes/{id}/stream
          → StreamEvent → 追加节点到最新 TurnGroup
          → done → 断开 WS，重拉 /turns 保证完整
```

## 实现范围

### 后端
1. `IterationTrace` + `TurnTrace` 新增字段
2. `agent_turn.rs` 捕获 reasoning_text 和 input_text
3. 新增 `/processes/{agent_id}/stream` WebSocket 端点

### 前端
4. 新建 `CascadeViewer` + `CascadeNode` + `TurnGroup` 组件
5. 替换 KernelTop 中的 TurnTraceTree
6. WebSocket 流式集成
