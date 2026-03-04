# TUI Session View — 设计文档

## 概述

将 `rara-top` TUI 的 Events tab 替换为 Sessions tab，从平铺 event 流重构为以 session 为中心的三栏布局：左侧 session 列表、右上甘特图时间轴、右下 agent 进程树。

## 布局

```
┌─────────────────────────────────────────────────────────────┐
│ rara-top  CONNECTED  Processes:3  Tokens:12.5k  Up:2h 15m  │
│ 1 Processes  2 Agents  3 Approvals  4 Audit  5 Sessions     │
├──────────────┬──────────────────────────────────────────────┤
│ Sessions     │  Gantt Timeline (上半)                        │
│              │  ─────────────────────────────────────────── │
│  ▶ abc123.. │  |███ main-agent ████████████████|            │
│    def456.. │  |    |██ child-1 ███|                        │
│    ghi789.. │  |    |     |██ child-2 █|                    │
│              │  0s        5s        10s        15s           │
│              ├──────────────────────────────────────────────┤
│              │  Agent Process Tree (下半)                    │
│              │  ─────────────────────────────────────────── │
│              │  ▼ main-agent  Running  12.5k tokens         │
│              │    ├─ child-1  Completed  3.2k tokens        │
│              │    │  └─ child-2  Completed  1.1k tokens     │
│              │    └─ child-3  Running  2.0k tokens          │
└──────────────┴──────────────────────────────────────────────┘
│ q:Quit  5:Tab  ↑↓:Select  Tab:Panel  Enter:Detail  r:Refresh│
└─────────────────────────────────────────────────────────────┘
```

## 数据模型

### 新增类型 (`types.rs`)

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelFocus {
    SessionList,
    Gantt,
    ProcessTree,
}

pub struct SessionState {
    pub sessions: IndexMap<String, SessionView>,
    pub selected_session: usize,
    pub focus: PanelFocus,
    pub gantt_selected: usize,
    pub tree_selected: usize,
}

pub struct SessionView {
    pub session_id: String,
    pub agents: IndexMap<String, AgentTimeline>,
    pub first_seen: Instant,
    pub last_event: Instant,
}

pub struct AgentTimeline {
    pub agent_id: String,
    pub name: String,
    pub parent_id: Option<String>,
    pub start: Instant,
    pub end: Option<Instant>,
    pub state: String,
    pub metrics: MetricsSnapshot,
    pub events: Vec<KernelEventEnvelope>,
}
```

### KernelEventEnvelope 变更

`KernelEventCommonFields` 已有 `agent_id: Option<String>`。需额外解析 `session_id`（从 event JSON payload 的 `common.session_id` 提取，或从 EventBase 传递）。

## 数据流

### 数据源（复用现有，不新增 API）

1. **Event SSE stream** (`/api/v1/kernel/events/stream`) — 实时事件，按 `session_id` 分组归入 `SessionView`
2. **Processes API** (`/api/v1/kernel/processes`) — 每秒 poll，更新 `AgentTimeline` 的 state/metrics/parent_id

### 事件处理逻辑

- 收到 event 时，按 `session_id` 分组归入对应 `SessionView`
- 没有 `session_id` 的事件按 `agent_id` 查找所属 session
- `ProcessStats` poll 数据更新 `AgentTimeline` 的 `state`/`metrics`/`parent_id`/`name`
- session 列表按 `last_event` 倒序排列

## 甘特图渲染

- 横轴 = session 起始到当前时间，按终端宽度等比缩放
- 纵轴 = 每行一个 agent，按进程树深度优先排列（parent 在上，child 缩进在下）
- bar 颜色：Running=Green, Completed=Gray, Failed=Red, Idle=Cyan
- 用 Unicode block 字符（`█`, `▓`, `░`）绘制时间条
- 左侧标签列显示 agent 短名

## 进程树渲染

- 树状缩进结构，用 `├─`, `└─`, `│` 连接线
- 每个节点显示：agent 名称 + 状态（带颜色）+ token 数
- 展开/折叠由选中 + Enter 控制

## 交互

### 焦点系统

| 快捷键 | 作用 |
|--------|------|
| `Tab` / `Shift+Tab` | 焦点在 SessionList → Gantt → ProcessTree 循环 |
| `↑`/`↓`/`j`/`k` | 当前焦点面板内上下移动 |
| `Enter` | SessionList: 选中并跳到 Gantt；Gantt/Tree: 弹出详情 popup |
| `1-5` | 全局 tab 切换 |
| `q` | 退出 |
| `r` | 刷新 |

### 视觉反馈

- 焦点面板 border = Cyan
- 非焦点面板 border = DarkGray
- 选中行 = DarkGray 背景 + Bold

## 文件变更清单

| 文件 | 变更 |
|------|------|
| `crates/cmd/src/top/types.rs` | 新增 SessionView, AgentTimeline, PanelFocus, SessionState |
| `crates/cmd/src/top/app.rs` | Tab::Events → Tab::Sessions；App 状态改为 SessionState；事件分组逻辑；焦点切换；面板内导航 |
| `crates/cmd/src/top/ui.rs` | 删除 render_events_table；新增 render_sessions_tab（三栏）、render_session_list、render_gantt、render_process_tree |
| `crates/cmd/src/top/client.rs` | 不变 |
| `crates/cmd/src/top/mod.rs` | 不变 |
| `crates/cmd/Cargo.toml` | 可能新增 indexmap 依赖 |

## 不做的事情

- 不新增 API endpoint
- 不新增 crate
- 不改 kernel 端代码
- 不做事件持久化或历史回放
