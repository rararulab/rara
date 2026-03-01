# Kernel Observability — HTTP API + TUI (`rara top`)

**Date**: 2026-03-01
**Status**: Approved

## Goal

让 kernel 的运行状态可观测。暴露 HTTP API 端点，并在 `rara-cmd` 中实现 `rara top` 子命令，以 TUI 形式实时展示 kernel 状态。

## Design Decisions

| Decision | Choice |
|----------|--------|
| TUI 位置 | 嵌入 `rara-cmd` 作为 `rara top` 子命令 |
| 展示面板 | 5 个：系统概览、进程列表、已注册 Agent、审批队列、审计日志 |
| 数据获取 | HTTP 轮询（1 秒间隔） |
| 交互模式 | 只读观测，无操作 |
| 布局风格 | Tab 切换，系统概览常驻顶部 |
| 错误格式 | RFC 9457 Problem Details |

## HTTP API

### 端点

```
GET /api/v1/kernel/stats          → SystemStats
GET /api/v1/kernel/processes      → Vec<ProcessStats>
GET /api/v1/kernel/approvals      → Vec<ApprovalRequest>
GET /api/v1/kernel/audit?limit=50 → Vec<AuditEvent>
GET /api/v1/kernel/agents         → 已有，复用
```

### 成功响应

直接返回 `T`，HTTP 200：

```json
{
  "active_processes": 3,
  "total_spawned": 12,
  "total_tokens_consumed": 45000,
  "uptime_ms": 360000
}
```

### 错误响应

RFC 9457 Problem Details，HTTP 4xx/5xx：

```json
{
  "type": "https://rara.dev/problems/not-found",
  "title": "Process not found",
  "status": 404,
  "detail": "No process with agent_id 550e8400-..."
}
```

### ProblemDetails 结构体

```rust
#[derive(Serialize)]
pub struct ProblemDetails {
    #[serde(rename = "type")]
    pub problem_type: String,
    pub title: String,
    pub status: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}
```

Content-Type: `application/problem+json`

## TUI 布局

```
┌─────────────────────────────────────────────┐
│ rara-top  Processes:3  Tokens:45k  Up:6m    │  ← 系统概览（常驻 1 行）
├─────────────────────────────────────────────┤
│ [1 Processes] [2 Agents] [3 Approvals] [4 Audit] │  ← Tab bar
├─────────────────────────────────────────────┤
│                                             │
│  （根据 tab 切换内容）                        │
│                                             │
├─────────────────────────────────────────────┤
│ q:Quit  1-4:Tab  ↑↓:Scroll  r:Refresh      │  ← 帮助栏
└─────────────────────────────────────────────┘
```

### Tab 内容

- **Processes** — 表格：ID(短)、Name、State、Uptime、LLM Calls、Tokens、Tool Calls
- **Agents** — 表格：Name、Role、Builtin/Custom、Tools 数量
- **Approvals** — 表格：ID、Agent、Tool、Risk Level、等待时间
- **Audit** — 日志流：Timestamp、Agent、Event Type、Details（截断）

### 按键

| Key | Action |
|-----|--------|
| `q` | 退出 |
| `1-4` | 切换 Tab |
| `↑↓` | 滚动列表 |
| `r` | 立即刷新 |

### 启动方式

```bash
rara top --url http://localhost:25555
```

默认连接 `http://localhost:25555`。

## Crate 结构

### backend-admin（修改）

```
src/kernel/
  mod.rs          # pub mod router, problem;
  router.rs       # 5 个 GET 端点
  problem.rs      # ProblemDetails + IntoResponse
```

`lib.rs` 中 nest kernel router 到 `/api/v1/kernel`。

### rara-cmd（修改）

```
src/top/
  mod.rs        # TopCmd::run() 入口
  app.rs        # App 状态 + 轮询逻辑
  ui.rs         # ratatui 渲染
  client.rs     # HTTP client（reqwest）
  types.rs      # API 响应反序列化类型
```

新增依赖：`ratatui`、`crossterm`、`reqwest`、`tokio`。

TUI types.rs 自行定义反序列化结构，不直接依赖 kernel crate，通过 HTTP JSON 解耦。

### kernel（可能修改）

给 `ProcessStats`、`SystemStats`、`ApprovalRequest`、`AuditEvent` 加 `#[derive(Serialize)]`（如果缺少）。

## 错误处理

### HTTP API

- kernel 方法返回 `None` → `404 ProblemDetails`
- 内部错误 → `500 ProblemDetails`

### TUI

- 连接失败 → 顶部状态栏 `DISCONNECTED`（红色），持续重试
- 部分端点失败 → 对应 tab 显示错误，其他 tab 正常
- 不 panic，不退出

## 测试策略

- HTTP 端点：集成测试，`axum::test` 验证 JSON 结构
- TUI 渲染：不测
- client.rs：不测（薄层 reqwest）

## Issue 拆分

1. **Issue A — Kernel HTTP API**: backend-admin 中的 kernel 路由 + ProblemDetails + kernel 类型加 Serialize
2. **Issue B — TUI `rara top`**: rara-cmd 中的 top 子命令 + HTTP client + ratatui 渲染

可并行开发。Issue B 端到端测试依赖 Issue A 完成。
