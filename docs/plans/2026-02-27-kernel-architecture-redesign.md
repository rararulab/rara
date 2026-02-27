# Kernel Architecture Redesign

## Overview

将现有的多 crate agent 架构（agent-core + rara-agents + rara-kernel）统一为单一 `rara-kernel` crate。Agent 从代码（struct + impl）变为纯数据（manifest 配置），Kernel 成为唯一的执行和编排中心。

## Design Decisions

1. **Agent = Manifest（纯数据）** — 没有 `ChatAgent`/`ProactiveAgent` struct，区别仅在配置（不同 system_prompt、tools 列表）
2. **Kernel = God Object** — 持有 7 个组件 + agent 注册表，驱动 LLM ↔ Tool 执行循环
3. **AgentContext = Struct（非 trait）** — Kernel 为每次调用实例化的"组件包"，agent 不需要多态 context
4. **移除 AgentRunner** — 循环逻辑内联到 Kernel::run_loop
5. **移除 agent-core** — 全部内容搬入 kernel
6. **移除 rara-agents** — Agent struct 消失

## Kernel 的 7 个组件

| 组件 | Trait | 职责 |
|------|-------|------|
| LLM | `LlmProvider` | Chat completion 请求/流式 + 模型能力检测 |
| Tool | `ToolRegistry` | 工具注册 + 按名分发 + 按 agent 过滤 |
| Memory | `Memory` (= State + Knowledge + Learning) | 三层记忆 CRUD |
| Session | `SessionStore` | 会话历史持久化 + 压缩 |
| Prompt | `PromptRepo` | 系统提示模板加载 |
| Guard | `Guard` | Tool 执行前拦截 + 输出审核 (HITL/RBAC) |
| Event Bus | `EventBus` | 组件间事件广播 |

## Kernel Struct

```rust
pub struct Kernel {
    // 7 组件插槽
    llm:      Arc<dyn LlmProvider>,
    tools:    Arc<ToolRegistry>,
    memory:   Arc<dyn Memory>,
    sessions: Arc<dyn SessionStore>,
    prompts:  Arc<dyn PromptRepo>,
    guard:    Arc<dyn Guard>,
    bus:      Arc<dyn EventBus>,

    // 运行时状态
    registry: AgentRegistry,
    config:   KernelConfig,
}
```

## AgentContext Struct

Kernel 为每次 agent 调用创建的组件包：

```rust
pub struct AgentContext {
    // 身份
    pub agent_id:   Uuid,
    pub session_id: Uuid,
    pub user_id:    Uuid,

    // 7 组件引用（Arc clone from Kernel）
    pub llm:      Arc<dyn LlmProvider>,
    pub tools:    Arc<ToolRegistry>,
    pub memory:   Arc<dyn Memory>,
    pub sessions: Arc<dyn SessionStore>,
    pub prompts:  Arc<dyn PromptRepo>,
    pub guard:    Arc<dyn Guard>,
    pub bus:      Arc<dyn EventBus>,

    // Agent 专属配置
    pub model:          String,
    pub system_prompt:  String,
    pub max_iterations: usize,
}
```

## AgentManifest

Agent 的全部定义 = 一份配置清单：

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentManifest {
    pub name:           String,
    pub model:          String,
    pub system_prompt:  String,
    pub provider_hint:  Option<String>,
    pub max_iterations: usize,
    pub tools:          Vec<String>,
    pub memory_scope:   Scope,
    pub guard_policy:   GuardPolicy,
    pub metadata:       serde_json::Value,
}
```

## 新增 Trait 定义

### LlmProvider

```rust
#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse>;
    async fn chat_stream(
        &self,
        request: ChatRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<ChatStreamDelta>> + Send>>>;
    fn capabilities(&self, model: &str) -> ModelCapabilities;
}

pub struct ChatRequest {
    pub model:         String,
    pub system_prompt: String,
    pub messages:      Vec<Message>,
    pub tools:         Option<Vec<ToolDefinition>>,
    pub temperature:   Option<f32>,
}

pub struct ChatResponse {
    pub content:       Option<String>,
    pub tool_calls:    Vec<ToolCall>,
    pub finish_reason: FinishReason,
    pub usage:         Option<Usage>,
}
```

### SessionStore

```rust
#[async_trait]
pub trait SessionStore: Send + Sync {
    async fn load_history(&self, session_id: Uuid) -> Result<Vec<Message>>;
    async fn append(&self, session_id: Uuid, exchange: Exchange) -> Result<()>;
    async fn get_or_create(&self, session_id: Uuid) -> Result<SessionMeta>;
    async fn compact(&self, session_id: Uuid, summary: String) -> Result<()>;
}
```

### Guard

```rust
#[async_trait]
pub trait Guard: Send + Sync {
    async fn check_tool(&self, ctx: &GuardContext, tool_name: &str, args: &Value) -> Verdict;
    async fn check_output(&self, ctx: &GuardContext, content: &str) -> Verdict;
}

pub enum Verdict {
    Allow,
    Deny { reason: String },
    NeedApproval { prompt: String },
}
```

### EventBus

```rust
#[async_trait]
pub trait EventBus: Send + Sync {
    async fn publish(&self, event: KernelEvent);
    async fn subscribe(&self, filter: EventFilter) -> EventStream;
}
```

## Kernel 执行流程

```rust
impl Kernel {
    pub async fn send(&self, agent_id: Uuid, input: UserInput) -> Result<Response> {
        let entry = self.registry.get(agent_id)?;
        let ctx = self.create_context(&entry);

        // 1. 加载历史
        let history = ctx.sessions.load_history(ctx.session_id).await?;

        // 2. 组装 system prompt
        let system = self.build_system_prompt(&ctx, &entry).await;

        // 3. LLM <-> Tool 循环
        let response = self.run_loop(&ctx, system, history, input).await?;

        // 4. 保存历史
        ctx.sessions.append(ctx.session_id, &response).await?;

        Ok(response)
    }

    async fn run_loop(&self, ctx: &AgentContext, ...) -> Result<Response> {
        for _ in 0..ctx.max_iterations {
            let resp = ctx.llm.chat(request).await?;

            if resp.tool_calls.is_empty() {
                return Ok(resp);
            }

            for call in &resp.tool_calls {
                let verdict = ctx.guard.check_tool(&guard_ctx, &call.name, &call.args).await;
                if verdict.is_allow() {
                    let result = ctx.tools.execute(&call).await?;
                    ctx.bus.publish(ToolExecuted { .. }).await;
                }
            }
        }
    }
}
```

## Crate 结构

```
rara-kernel（唯一核心 crate）
├── lib.rs
├── kernel.rs        # Kernel struct + run_loop + send/send_stream
├── context.rs       # AgentContext struct
├── registry.rs      # AgentRegistry + AgentManifest + AgentEntry
├── error.rs         # KernelError
├── llm.rs           # LlmProvider trait + ChatRequest/ChatResponse
├── memory/          # Memory traits (3-layer)
│   ├── mod.rs
│   ├── types.rs
│   ├── error.rs
│   ├── state.rs
│   ├── knowledge.rs
│   └── learning.rs
├── session.rs       # SessionStore trait
├── prompt.rs        # PromptRepo trait
├── guard.rs         # Guard trait + Verdict
├── event.rs         # EventBus trait + KernelEvent
├── tool.rs          # ToolRegistry (from agent-core)
├── model.rs         # ModelCapabilities (from agent-core)
└── defaults/        # 默认实现
    ├── noop_guard.rs
    ├── broadcast_bus.rs
    └── in_memory_session.rs
```

## 迁移步骤

### Phase 1 — Kernel trait 定义
1. 把 `agent-core::memory/` 搬入 `kernel::memory/`
2. 把 `agent-core::tool_registry.rs` 搬入 `kernel::tool.rs`
3. 把 `agent-core::model.rs` 搬入 `kernel::model.rs`
4. 把 `agent-core::prompt.rs` 搬入 `kernel::prompt.rs`
5. 新建 `kernel::llm.rs` — LlmProvider trait + ChatRequest/ChatResponse
6. 新建 `kernel::session.rs` — SessionStore trait
7. 新建 `kernel::guard.rs` — Guard trait + Verdict
8. 新建 `kernel::event.rs` — EventBus trait + KernelEvent
9. 新建 `kernel::context.rs` — AgentContext struct

### Phase 2 — Kernel 核心实现
10. 重写 `kernel::kernel.rs` — 7 组件 + run_loop + send/send_stream
11. 扩展 `kernel::registry.rs` — AgentManifest 加 memory_scope/guard_policy
12. 新建 `kernel::defaults/` — NoopGuard, BroadcastBus, InMemorySessionStore

### Phase 3 — LlmProvider 实现
13. 实现 OpenAiLlmProvider（包装 async-openai，含 fallback + 重试）

### Phase 4 — 上游消费者迁移
14. rara-agents 中 ChatAgent/ProactiveAgent/ScheduledAgent → manifest 配置
15. rara-chat::ChatService → 改用 Kernel::send/send_stream
16. rara-workers → 改用 Kernel
17. rara-app 组合根 → 构造 Kernel

### Phase 5 — 清理
18. 删除 agent-core crate
19. 删除 rara-agents crate
20. 更新 workspace Cargo.toml
