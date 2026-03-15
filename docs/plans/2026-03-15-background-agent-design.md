# Background Agent Design

## Summary

Agent 可以在 turn 执行过程中，通过 `spawn_background` tool spawn 一个后台 agent 执行耗时任务。Parent agent 的 turn 正常结束并回复用户。Background agent 完成后，kernel 触发 parent 的 proactive turn 来组织结果并推送给用户。

参考 Claude Code subagent 模型：background agent 运行在隔离 context 中，不能反向提问 parent，只有最终结果返回。

## Design Decisions

| 决策 | 选项 | 理由 |
|------|------|------|
| 触发方式 | Builtin tool | Agent 自己最清楚什么任务需要后台化 |
| 结果通知 | Parent proactive turn | Parent 有机会总结、格式化结果再推送 |
| Agent 身份 | Parent 动态构造 manifest | Agent manifest 支持动态生成 |
| Child→Parent 通信 | 不支持 | Claude 模型：background agent 自行决策，不确定的在结果中标注 |
| 状态感知 | Context 注入 | Parent turn 开始时注入 active background tasks 状态 |

## Architecture

```
User Message → Parent Turn
  → agent 调用 spawn_background(manifest, input, description)
  → SpawnBackgroundTool::execute():
      1. KernelHandle::spawn_child() 创建 child session
      2. 注册 BackgroundTaskEntry 到 parent session
      3. 不 await result_rx，交给 kernel background watcher
      4. 返回 { task_id, agent_name } 给 agent
  → agent 继续当前 turn，回复用户 "已在后台处理"
  → turn 正常结束

... 用户可以继续发消息，parent 正常响应 ...
... (context 注入: "你有 1 个后台任务正在运行: [task_id: description]") ...

Background child 完成 → cleanup_process()
  → ChildSessionDone 到达 parent
  → handle_child_completed() 检测到是 background task
  → 更新 BackgroundTaskEntry 状态为 Completed/Failed
  → 触发 parent proactive turn (类似 MitaDirective 机制)
  → parent agent 看到 child result，组织回复
  → 通过 origin_endpoint 推送给用户
```

## Components

### 1. BackgroundTaskEntry

在 `Session` struct 上新增 background task 追踪。不需要独立 registry — 生命周期绑定 parent session。

```rust
// crates/kernel/src/session/mod.rs

/// Tracks a background child agent spawned by this session.
#[derive(Debug, Clone)]
pub struct BackgroundTaskEntry {
    /// Child session key (doubles as task_id).
    pub child_key: SessionKey,
    /// Human-readable name from the spawned manifest.
    pub agent_name: String,
    /// Description provided by the parent agent.
    pub description: String,
    /// When the task was spawned.
    pub created_at: jiff::Timestamp,
}
```

在 `Session` 中新增:
```rust
/// Active background tasks spawned by this session.
pub background_tasks: Vec<BackgroundTaskEntry>,
```

### 2. SpawnBackgroundTool

新增 builtin tool: `crates/kernel/src/tool/spawn_background.rs`

```rust
pub struct SpawnBackgroundTool {
    kernel_handle: Arc<KernelHandle>,
}
```

**Tool schema:**
```json
{
  "name": "spawn_background",
  "description": "Spawn a background agent to handle a long-running task. The agent runs independently and results are delivered when complete. You cannot interact with the background agent after spawning.",
  "parameters": {
    "type": "object",
    "required": ["manifest", "input", "description"],
    "properties": {
      "manifest": {
        "type": "object",
        "description": "Agent manifest for the background agent (name, system_prompt, model, tools, etc.)",
        "properties": {
          "name": { "type": "string" },
          "system_prompt": { "type": "string" },
          "model": { "type": "string" },
          "tools": { "type": "array", "items": { "type": "string" } },
          "max_iterations": { "type": "integer" }
        },
        "required": ["name", "system_prompt"]
      },
      "input": {
        "type": "string",
        "description": "The task instruction to send to the background agent"
      },
      "description": {
        "type": "string",
        "description": "Short human-readable description of the task (shown to user in status)"
      }
    }
  }
}
```

**Execute 流程:**

```rust
async fn execute(&self, params: Value, context: &ToolContext) -> Result<ToolOutput> {
    let session_key = context.session_key.unwrap();
    let principal = /* resolve from context */;

    // 1. Parse manifest from params
    let manifest = AgentManifest::from_json(params["manifest"])?;

    // 2. Spawn child (reuse existing mechanism)
    let handle = self.kernel_handle
        .spawn_child(&session_key, &principal, manifest.clone(), input)
        .await?;

    // 3. Register as background task on parent session
    let entry = BackgroundTaskEntry {
        child_key: handle.session_key,
        agent_name: manifest.name.clone(),
        description: description.clone(),
        created_at: jiff::Timestamp::now(),
    };
    self.kernel_handle.register_background_task(&session_key, entry);

    // 4. Spawn watcher — drain result_rx (fire-and-forget)
    tokio::spawn(async move {
        let mut rx = handle.result_rx;
        while let Some(event) = rx.recv().await {
            if matches!(event, AgentEvent::Done(_)) {
                break;
            }
        }
        // ChildSessionDone is already emitted by cleanup_process.
    });

    // 5. Return immediately
    Ok(serde_json::json!({
        "task_id": child_key.to_string(),
        "agent_name": manifest.name,
        "status": "spawned",
        "message": "Background agent is now running. Results will be delivered when complete."
    }).into())
}
```

### 3. handle_child_completed 改造

当 `ChildSessionDone` 到达时，检测是否为 background task。如果是，触发 parent proactive turn。

```rust
// crates/kernel/src/kernel.rs — handle_child_completed()

async fn handle_child_completed(
    &self,
    parent_id: SessionKey,
    child_id: SessionKey,
    result: AgentRunLoopResult,
) {
    // ... existing: persist child result to parent tape ...

    // Check if this is a background task
    let is_background = self.process_table.with(&parent_id, |p| {
        p.background_tasks.iter().any(|t| t.child_key == child_id)
    }).unwrap_or(false);

    if is_background {
        // Remove from active list
        self.handle.remove_background_task(&parent_id, &child_id);

        // Emit BackgroundTaskDone so clients remove the status indicator.
        self.handle.stream_hub().emit_to_session(
            &parent_id,
            StreamEvent::BackgroundTaskDone {
                task_id: child_id.to_string(),
                status,
            },
        );

        // Build directive with full result context for the proactive turn.
        // On failure, include the TurnTrace so parent can diagnose without
        // needing to read the child tape manually.
        //
        // NOTE: Failure detection is fragile — AgentRunLoopResult has no
        // status/error field, so we rely on output prefix heuristics.
        // TODO: Add explicit status field to AgentRunLoopResult.
        let status = if result.output.starts_with("error:") {
            BackgroundTaskStatus::Failed
        } else {
            BackgroundTaskStatus::Completed
        };
        let trace_section = if status == BackgroundTaskStatus::Failed {
            // Serialize the last TurnTrace from the child session for debugging.
            // The trace contains iteration details, tool calls, and error messages.
            let trace_json = self.process_table
                .with(&child_id, |p| {
                    p.turn_traces.back().map(|t| serde_json::to_string_pretty(t).ok())
                })
                .flatten().flatten()
                .unwrap_or_default();
            format!("\n\n[Debug Trace]\n{trace_json}")
        } else {
            String::new()
        };

        let directive = format!(
            "[Background Task {status}]\n\
             task_id={child_id}\n\
             iterations={}, tool_calls={}\n\n\
             Result:\n{truncated_output}{trace_section}\n\n\
             Proactively inform the user of the outcome. Be concise. \
             If the task failed, explain what went wrong.",
            result.iterations, result.tool_calls,
        );

        let system_user = crate::identity::UserId("system".to_string());
        let mut msg = InboundMessage::synthetic(
            directive,
            system_user,
            parent_id,
        );
        msg.metadata.insert(
            "background_task_done".to_string(),
            serde_json::json!(child_id.to_string()),
        );
        self.deliver_to_session(parent_id, msg).await;
    }
}
```

### 4. Context 注入

在 `run_agent_loop` 的 **第一次 iteration**（`iteration == 0`）时注入 active background tasks 列表。仅在首次注入，避免每次 iteration 重复添加相同信息。

```rust
// crates/kernel/src/agent.rs — run_agent_loop() iteration 0

if iteration == 0 {
    let background_tasks: Vec<BackgroundTaskEntry> = handle
        .background_tasks(&session_key);

    if !background_tasks.is_empty() {
    let task_list: String = background_tasks
        .iter()
        .enumerate()
        .map(|(i, t)| format!(
            "  {}. [{}] {} (started {})",
            i + 1, t.child_key, t.description,
            humanize_duration(t.created_at)
        ))
        .collect::<Vec<_>>()
        .join("\n");

    reminders.push(format!(
        "[Active Background Tasks]\n\
         You have {} background task(s) running:\n{task_list}\n\
         Results will be delivered automatically when complete. \
         Use cancel_background(task_id) to cancel if needed.",
        background_tasks.len()
    ));
}
```

### 5. ToolContext 扩展

`SpawnBackgroundTool` 需要 `KernelHandle` 来调用 `spawn_child`。当前 `ToolContext` 只暴露 `event_queue`。

**方案**: 在 `ToolContext` 中新增 `kernel_handle: Option<Arc<KernelHandle>>`。这与 schedule tools 已有的模式一致（schedule tools 通过 event_queue 推事件，但 spawn_child 需要 await oneshot reply）。

```rust
pub struct ToolContext {
    pub user_id:         Option<String>,
    pub session_key:     Option<SessionKey>,
    pub origin_endpoint: Option<Endpoint>,
    pub event_queue:     Option<EventQueueRef>,
    pub kernel_handle:   Option<Arc<KernelHandle>>,  // NEW
}
```

## File Changes Summary

| File | Change |
|------|--------|
| `crates/kernel/src/session/mod.rs` | Add `BackgroundTaskEntry`, add `background_tasks` field to `Session` |
| `crates/kernel/src/tool/spawn_background.rs` | **New** — `SpawnBackgroundTool` impl |
| `crates/kernel/src/tool/mod.rs` | Add `pub(crate) mod spawn_background;` export |
| `crates/kernel/src/kernel.rs` | Modify `handle_child_completed()` to detect background tasks and trigger proactive turn |
| `crates/kernel/src/agent.rs` | Add background task status injection in context building |
| `crates/kernel/src/tool/mod.rs` | Add `kernel_handle` to `ToolContext` |
| `crates/kernel/src/handle.rs` | Add `register_background_task()` method |

### 7. Client-Side Progress Display (StreamEvent)

参考 Claude Code 的 subagent 进度条设计，通过 `StreamEvent` 实时推送 background task 状态变化，客户端渲染进度指示器（含 elapsed timer）。

```rust
// crates/kernel/src/io.rs

/// Terminal status of a background agent task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackgroundTaskStatus {
    Completed,
    Failed,
    Cancelled,
}

// StreamEvent 新增 variants:
BackgroundTaskStarted {
    task_id:     String,
    agent_name:  String,
    description: String,
},
BackgroundTaskDone {
    task_id: String,
    status:  BackgroundTaskStatus,
},
```

**事件流:**
- `SpawnBackgroundTool::execute()` 成功后 emit `BackgroundTaskStarted`
- `handle_child_completed()` 检测到 background task 完成后 emit `BackgroundTaskDone`
- `CancelBackgroundTool::execute()` 取消后 emit `BackgroundTaskDone { status: Cancelled }`

**客户端行为:**
- 收到 `BackgroundTaskStarted` → 显示进度指示器（agent_name + description + elapsed timer）
- 收到 `BackgroundTaskDone` → 移除进度指示器，可选显示完成/失败状态

`StreamHub::emit_to_session()` 方法用于向特定 session 的客户端推送事件。
<!-- TODO: emit_to_session 当前使用线性扫描 session，高并发时需优化 -->

## Security Considerations

- Background agent 继承 parent 的 `Principal` — 权限不会超过 parent
- Child semaphore 限制并发 background tasks 数量（已有机制）
- Guard pipeline 对 background agent 的 tool 调用同样生效
- Background agent 不能 spawn 子 agent（与 Claude 模型一致，通过 manifest 中不包含 `spawn_background` tool 实现）

### 6. CancelBackgroundTool

新增 builtin tool: `crates/kernel/src/tool/cancel_background.rs`

Parent agent 可以主动取消正在运行的 background agent。复用已有的 `SendSignal` 机制。

```rust
pub struct CancelBackgroundTool {
    kernel_handle: Arc<KernelHandle>,
}
```

**Tool schema:**
```json
{
  "name": "cancel_background",
  "description": "Cancel a running background task by task_id.",
  "parameters": {
    "type": "object",
    "required": ["task_id"],
    "properties": {
      "task_id": {
        "type": "string",
        "description": "The task_id returned by spawn_background"
      },
      "reason": {
        "type": "string",
        "description": "Optional reason for cancellation"
      }
    }
  }
}
```

**Execute 流程:**

```rust
async fn execute(&self, params: Value, context: &ToolContext) -> Result<ToolOutput> {
    let session_key = context.session_key.unwrap();
    let task_id: SessionKey = params["task_id"].as_str().unwrap().parse()?;

    // 1. Verify task_id belongs to this session's background tasks
    let found = self.kernel_handle.process_table().with(&session_key, |p| {
        p.background_tasks.iter().any(|t| t.child_key == task_id)
    }).unwrap_or(false);

    if !found {
        return Ok(serde_json::json!({
            "error": "task not found or not a background task of this session"
        }).into());
    }

    // 2. Send Terminate signal to child session
    self.kernel_handle.send_signal(task_id, Signal::Terminate).await?;

    // 3. Remove from active list
    self.kernel_handle.process_table().with_mut(&session_key, |p| {
        p.background_tasks.retain(|t| t.child_key != task_id);
    });

    Ok(serde_json::json!({
        "task_id": task_id.to_string(),
        "status": "cancelled"
    }).into())
}
```

取消后 `cleanup_process()` 仍然会触发 `ChildSessionDone`，但由于 task 已从 `background_tasks` 中移除，`handle_child_completed` 不会触发 proactive turn。

## File Changes Summary (updated)

| File | Change |
|------|--------|
| `crates/kernel/src/session/mod.rs` | Add `BackgroundTaskEntry`, add `background_tasks` field to `Session` |
| `crates/kernel/src/tool/spawn_background.rs` | **New** — `SpawnBackgroundTool` impl |
| `crates/kernel/src/tool/cancel_background.rs` | **New** — `CancelBackgroundTool` impl |
| `crates/kernel/src/tool/mod.rs` | Add module exports, add `kernel_handle` to `ToolContext` |
| `crates/kernel/src/kernel.rs` | Modify `handle_child_completed()` to detect background tasks and trigger proactive turn |
| `crates/kernel/src/agent.rs` | Add background task status injection in context building |
| `crates/kernel/src/handle.rs` | Add `register_background_task()` method |

## Observability

Background agent 的可观测性分三层：

**1. Parent Agent 视角（LLM 可见）**
- **运行中**: context 注入 active task 列表（task_id + description + started_at）
- **完成**: proactive turn directive 包含完整 result output
- **失败**: proactive turn directive 额外包含 `TurnTrace`（最后一次 iteration 的 tool calls、error messages）

**2. Child Tape（持久化）**
- Child 有独立的 tape（keyed by child session_key = task_id）
- 完整记录每个 iteration、tool call input/output、LLM response
- 即使 child session 被 cleanup，tape 文件保留（JSONL 持久化）
- 运维或事后分析可直接读取 `~/.config/rara/tapes/{task_id}.jsonl`

**3. Metrics（Prometheus）**
- 复用已有 `SESSION_ACTIVE` / `SESSION_SUSPENDED` gauges
- Background child 的 `manifest_name` label 区分于普通 session
- `RuntimeMetrics`（llm_calls, tool_calls, tokens_consumed）在 child session 上独立计数

**失败诊断流程：**
```
background agent failed
  → ChildSessionDone 携带 AgentRunLoopResult (output 含 error)
  → handle_child_completed 提取 child 的最后 TurnTrace
  → proactive turn directive 包含 result + trace
  → parent agent 看到错误信息 + trace，向用户解释失败原因
  → 如需深入分析：child tape 保留完整历史
```

## Future Extensions

- **进度上报**: Background agent 通过 pipe 或 event 上报进度，parent context 注入实时进度
- **并发限制**: 可配置的 per-session background task 上限（当前由 child_semaphore 控制）
