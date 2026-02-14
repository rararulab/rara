# Proactive Agent (Agent Soul)

Proactive Agent 是一个 cron 驱动的后台 worker，周期性回顾用户最近的聊天活动，通过 LLM 判断是否需要主动发送 Telegram 消息。

**核心理念：** 让 AI 代理拥有可配置的"灵魂"（personality prompt），在用户不主动提问时也能提供帮助。

**示例场景：** 用户白天分享了几个 JD 并提了一些问题，晚上代理主动发送："嘿，我注意到你之前在看 Stripe 那个 Senior Backend 岗位，还感兴趣吗？我可以帮你准备简历。"

## Architecture

```
Cron trigger (default: 9am, 6pm, 9pm daily)
  │
  ▼
ProactiveAgentWorker.work()
  │
  ├── settings_svc.current() ─── agent.proactive_enabled? ──► false → skip
  ├── ai.openrouter_api_key? ──► None → skip
  ├── telegram.chat_id? ────────► None → skip
  │
  ├── chat_service.list_sessions(20)
  │     └── filter: updated_at > now - 24h
  │
  ├── for each session:
  │     └── chat_service.get_messages(last 20)
  │           └── build activity summary text
  │
  ├── AgentRunner (no tools, max_iterations=1)
  │     ├── system_prompt: agent.soul (or DEFAULT_SOUL_PROMPT)
  │     └── user_content: activity summary + instructions
  │
  └── LLM response
        ├── "SKIP" or empty → done
        └── message text → notify_client.send_telegram()
```

### Components

| Component | Role |
|-----------|------|
| `ProactiveAgentWorker` | Cron-triggered `FallibleWorker<AppState>` |
| `AgentSettings` | soul prompt、enabled 开关、cron 表达式 |
| `SettingsSvc` | 运行时设置热读取 |
| `ChatService` | 读取最近 sessions 和 messages |
| `AgentRunner` | 单轮 LLM 调用（无 tool-calling） |
| `NotifyClient` | 通过 PGMQ 队列发送 Telegram 通知 |

## Settings

### AgentSettings

```rust
pub struct AgentSettings {
    pub soul: Option<String>,         // LLM 人格 prompt，None 使用内置默认
    pub proactive_enabled: bool,      // 是否启用，支持运行时热切换
    pub proactive_cron: Option<String>, // Cron 表达式，修改需重启
}
```

`AgentSettings` 作为 `Settings.agent` 嵌入，使用 `#[serde(default)]` 向后兼容。

### API

通过 `POST /api/v1/settings` 更新：

```json
{
  "agent": {
    "soul": "You are a friendly career coach...",
    "proactive_enabled": true,
    "proactive_cron": "0 9,21 * * *"
  }
}
```

所有字段可选，partial update 语义。`soul` 和 `proactive_cron` 发送空字符串表示清空（恢复默认）。

### Hot Reload

| Setting | Hot Reload | Notes |
|---------|-----------|-------|
| `proactive_enabled` | Yes | 每次 tick 检查 |
| `soul` | Yes | 每次 tick 读取 |
| `proactive_cron` | No | 启动时固定，修改需重启 |

## Worker Flow

### Guard Checks

每次 tick 依次检查，任一不满足则 early return：

1. `agent.proactive_enabled` — 运行时热读取
2. `ai.openrouter_api_key` — 必须配置 AI 服务
3. `telegram.chat_id` — 必须配置 Telegram 接收方

### Activity Collection

1. 列出最近 20 个 sessions（按 `updated_at` 降序）
2. 过滤 `updated_at > now - 24h`
3. 对每个 session 读取最后 20 条消息
4. 构建文本摘要，每条消息截断到 200 字符（UTF-8 char boundary safe）

### LLM Reflection

使用 `AgentRunner` 单轮调用：

- **System prompt:** 用户自定义 soul 或内置默认
- **User content:** 活动摘要 + 指令
- **Tools:** 空（纯文本生成）
- **Max iterations:** 1
- **Model:** `settings.ai.model_for(ModelScenario::Chat)`

### Message Delivery

LLM 回复不是 "SKIP" 时，通过 `NotifyClient.send_telegram()` 入队 PGMQ，由 Telegram bot 异步发送。

## Default Soul Prompt

```
You are a proactive job search companion. You're encouraging, data-driven, and concise.
When reviewing recent user activity, consider:
- Did they share JDs but not follow up?
- Are they stuck on applications without progress?
- Did they ask questions that suggest uncertainty?
If you spot something worth mentioning, craft a brief, warm Telegram message (max 300 chars).
If nothing stands out, respond with exactly "SKIP".
```

## Cron Scheduling

默认 `0 9,18,21 * * *`（每天 9:00、18:00、21:00）。

无效 cron 表达式在启动时**自动降级**到默认值并输出 warn 日志，不会 panic。

## Frontend

Settings 页面 "Agent Personality" 卡片：

- **Soul Prompt** — 多行文本域，placeholder 显示默认 prompt
- **Proactive Messaging** — Switch 开关
- **Cron Schedule** — 文本输入，附常用值说明

## Source Files

| File | Description |
|------|-------------|
| `crates/domain/shared/src/settings/model.rs` | `AgentSettings` + patch + normalize |
| `crates/domain/shared/src/settings/router.rs` | `AgentSettingsView` + API |
| `crates/workers/src/proactive.rs` | `ProactiveAgentWorker` |
| `crates/workers/src/worker_state.rs` | `llm_provider` in `AppState` |
| `crates/app/src/lib.rs` | Worker 注册 + cron 降级 |
| `web/src/pages/Settings.tsx` | Agent Personality UI |
