# rara-kernel — Agent Guidelines

## Critical: StreamDelta Event Ordering in `openai.rs`

### The Invariant

In `StreamAccumulator::process_chunk()`, **`ToolCallStart` MUST be sent before `ToolCallArgumentsDelta`** for the same tool call index.

The receiver in `agent.rs` uses a `HashMap<u32, PendingToolCall>` keyed by index. The entry is only created when `ToolCallStart` arrives. If `ToolCallArgumentsDelta` arrives first, `get_mut(&index)` returns `None` and **the arguments are silently dropped**.

### Why This Matters

Some LLM providers (notably OpenRouter) deliver the tool call name and arguments in a **single SSE chunk**. If the code emits `ToolCallArgumentsDelta` before `ToolCallStart`, the arguments are lost. This causes:

- Tool calls with empty `{}` arguments
- Bash tool fails with `missing required parameter: command`
- Agent enters a retry loop (67+ failed calls observed in production)

### The Pattern (DO NOT CHANGE)

```
1. Set entry.id    (from tc.id)
2. Set entry.name  (from tc.function.name)
3. Collect args into local variable (DO NOT send yet)
4. Emit ToolCallStart  (if !started && id + name are set)
5. Emit ToolCallArgumentsDelta  (now the receiver entry exists)
```

### What NOT To Do

- Do NOT move `ToolCallArgumentsDelta` emission before `ToolCallStart`
- Do NOT inline the argument send back into the `if let Some(ref func)` block before the start check
- Do NOT assume providers send tool call parts in separate chunks — single-chunk delivery is common

---

## Critical: IngressRateLimiter in `io.rs`

### The Invariant

Rate limiting MUST happen in `IOSubsystem::resolve()` **before** identity resolution and session lookup. These are expensive operations (DB queries); rate limiting protects them.

### Design

- Per-key sliding window: `DashMap<String, Vec<Instant>>`, 60-second window
- Key format: `"{channel_type}:{platform_user_id}"`
- `max_per_minute` is configured via `AppConfig.max_ingress_per_minute` (YAML config, serde default = 30)
- `gc()` method removes expired empty entries — call periodically to prevent memory growth

### What NOT To Do

- Do NOT move the `check_rate()` call after identity resolution — identity resolution hits the database. If a malicious user floods messages, every message would trigger a DB query before being rejected. Rate limiting first means we reject spam with zero DB cost.
- Do NOT hardcode the rate limit value — the project principle is "no config defaults in Rust code" (see CLAUDE.md). The value lives in `AppConfig` so operators can tune it per deployment without recompiling.
- Do NOT remove the `gc()` method — `DashMap` never auto-evicts keys. Without `gc()`, every unique `{channel}:{user}` that ever sent a message stays in memory forever as an empty `Vec`. In group chats with many participants, this is unbounded growth.
- Do NOT apply rate limiting to `ChannelType::Internal` or `ChannelType::Proactive` messages — these are kernel-generated (scheduled jobs, Mita directives, agent-to-agent calls). They bypass `IOSubsystem::resolve()` entirely via `InboundMessage::synthetic()`. Rate limiting them would break internal orchestration.

---

## Critical: GroupPolicy in `channel/types.rs` + Telegram adapter

### The Invariant

The `GroupPolicy` enum controls group chat routing in the Telegram adapter. The default variant MUST be `MentionOrSmallGroup` to preserve backward-compatible behavior (small groups ≤ 3 members respond automatically, larger groups require @mention or rara keyword).

### Routing Logic (in `handle_update()`)

```
GroupPolicy::Ignore              → return early, drop message
GroupPolicy::MentionOnly         → respond only if @mentioned or rara keyword
GroupPolicy::MentionOrSmallGroup → small group: always; large group: need mention, else GroupMessage
GroupPolicy::ProactiveJudgment   → mentioned: respond; not mentioned: GroupMessage for LLM judgment
GroupPolicy::All                 → respond to everything
```

`is_group_proactive = true` routes via `submit_group_message()` → kernel `handle_group_message()` → LLM judgment → optional agent turn.

### Configuration

- YAML: `telegram.group_policy: mention_or_small_group`
- Settings key: `telegram.group_policy`
- Hot-reloadable via settings watcher in `try_build_telegram()`
- Parsing uses `strum::EnumString` (`str::parse::<GroupPolicy>()`) — adding a new variant to the enum automatically works in config

### What NOT To Do

- Do NOT change the `#[default]` attribute away from `MentionOrSmallGroup` — existing deployments have no `group_policy` in their config. Changing the default silently changes their bot behavior (e.g. switching to `All` would make the bot reply to every group message, spamming the chat).
- Do NOT add manual string parsing for GroupPolicy — the enum derives `strum::EnumString` with `#[strum(serialize_all = "snake_case")]`. Manual match-arms duplicate the variant list and will silently ignore new variants when someone adds one to the enum. `str::parse()` handles it automatically.
- Do NOT put group policy logic in the kernel — the `MentionOrSmallGroup` variant calls `bot.get_chat_member_count()`, which is a Telegram Bot API call. The kernel has no Telegram dependency and must not acquire one. Policy evaluation requires platform-specific context that only the adapter has.

---

## Critical: Tape-Driven Message Rebuild + Context Budget

### What

Agent loop 每次迭代从 tape 重建 LLM messages，而不是在内存中累积。配合两层 context budget 截断，控制发给 LLM 的 token 量。

### Why

之前 `run_agent_loop` 在 turn 开始时从 tape 加载 history 到内存 `messages` 向量，然后每次迭代同时 push 到 messages 和写入 tape（双写）。问题：

1. **messages 只增不减** — 第 N 次迭代发送前 N-1 次所有内容给 LLM，O(N²) token 增长
2. **无法在重建时做截断** — 内存 messages 是独立副本，截断它不影响 tape，但也无法利用 tape 做选择性重建
3. **警告消息跨迭代累积** — context pressure warning 本身也被 push 进 messages，越警告 context 越大

### How

```
tape (JSONL, append-only) ← 唯一真相源
         │
         ▼  每次迭代
rebuild_messages_for_llm()
  = [system prompt] + [anchor context] + [user memory] + [对话历史 since last anchor]
         │
         ▼  临时注入（不持久化）
  + [tape search reminder]      只在第一次迭代
  + [anchor reminder]           上次迭代有大 tool output 时
  + [context pressure warning]  超过阈值时
  + [LLM error recovery msg]   上次迭代 LLM 出错时
         │
         ▼  截断（只影响发给 LLM 的，tape 不变）
  Layer 1: truncate_tool_result()    单个 tool result > 30% context window
  Layer 2: apply_context_guard()     总 tool results > 75% headroom → 旧结果压缩到 2K
         │
         ▼
       LLM call
         │
         ▼  只写 tape
  tape.append_message(assistant)
  tape.append_tool_call(calls)
  tape.append_tool_result(results)
```

### The Invariant

- **Tape 是唯一真相源** — loop 内不 `messages.push`，下次迭代从 tape 重建
- **临时注入不持久化** — reminder/warning 只在本次迭代的 messages 中，下次重建时消失
- **截断只影响 LLM 视角** — `context_budget` 修改的是重建后的 messages 副本，tape 保留完整原始数据

### Key Files

| File | Role |
|------|------|
| `agent.rs` | Loop 内每次迭代调用 `rebuild_messages_for_llm`，临时注入 + context guard |
| `memory/service.rs` | `rebuild_messages_for_llm()` — system prompt + tape history 重建 |
| `memory/context.rs` | `default_tape_context()` — tape entries → LLM messages 转换 |
| `context_budget.rs` | Layer 1 (`truncate_tool_result`) + Layer 2 (`apply_context_guard`) |

### Context Budget Parameters

| Parameter | Value | Meaning |
|-----------|-------|---------|
| `TOOL_CHARS_PER_TOKEN` | 3 | Tool output density (code/JSON) |
| `per_result_cap` | 30% of context window | Layer 1: 单个 tool result 上限 |
| `single_result_max` | 50% of context window | Layer 2 Pass 1: 硬上限 |
| `total_tool_headroom` | 75% of context window | Layer 2 Pass 2: 总量阈值 |
| `COMPACT_TARGET_CHARS` | 2,000 | Layer 2 Pass 2: 旧结果压缩目标 |

### What NOT To Do

- Do NOT add `messages.push` in the agent loop — 所有新内容只写 tape，下次迭代重建。如果你 push 了，这条消息会在下次重建时**重复出现**（tape 有一份 + push 一份）。
- Do NOT persist reminder/warning messages to tape — 它们是临时的 LLM 指令，不应该成为对话历史的一部分。持久化会导致每次重建都注入旧的警告。
- Do NOT truncate tape entries — tape 是完整历史，截断只发生在 `rebuild_messages_for_llm()` 之后的 `context_budget` 阶段。
- Do NOT bypass `rebuild_messages_for_llm` for "performance" by caching messages across iterations — tape 是 append-only JSONL 本地文件，读取 <1ms。缓存引入双写一致性问题，正是我们刚消除的。
- Do NOT change `context_budget` thresholds without testing — 30%/50%/75% 是参考 OpenFang 的经验值，过低会截断有用信息，过高会 context overflow。

---

## TaskReport & Subscription Notification Bus

### What

Structured `TaskReport` publishing and tag-based `Subscription` routing so background/scheduled tasks can notify sessions of their results.

### Key Files

| File | Role |
|------|------|
| `task_report.rs` | `TaskReport`, `TaskReportStatus`, `PrReviewResult` and related types |
| `notification/` | `TaskNotification`, `TaskReportRef`, `NotifyAction`, `Subscription`, `SubscriptionRegistry` (file-backed) |
| `event.rs` | `Syscall::Subscribe`, `Syscall::Unsubscribe`, `Syscall::PublishTaskReport` variants |
| `syscall.rs` | Dispatch arms + `handle_publish_task_report` + `SyscallTool` exec methods and schema |
| `handle.rs` | `KernelHandle::deliver_internal()` for ProactiveTurn delivery |
| `kernel.rs` | `SubscriptionRegistry` instantiation and session cleanup |

### Three Syscalls

1. **Subscribe** — register tag-based subscription for a session, returns subscription UUID
2. **Unsubscribe** — remove subscription by ID
3. **PublishTaskReport** — persist result to `JobResultStore` (for scheduled jobs), match subscriptions, deliver via ProactiveTurn (synthetic message → LLM turn) or SilentAppend (tape entry only)

### Critical Invariants

- `task_type` is always auto-included in `tags` by `exec_publish_report` — callers don't need to duplicate it
- `source_session` is always overwritten to the calling session — agents cannot spoof the source
- Subscriptions are cleaned up on session end (`cleanup_process` calls `remove_session`)
- `SubscriptionRegistry` is file-backed (`subscriptions.json`) — subscriptions survive kernel restarts

### Delivery Modes

- `ProactiveTurn`: creates an `InboundMessage::synthetic()` with the subscription owner's identity, pushed to the event queue. This triggers an LLM turn on the subscriber session. **If the subscriber session is not in the process table** (e.g. after restart), delivery is automatically downgraded to `SilentAppend` to avoid restoring the session with an incorrect identity.
- `SilentAppend`: appends a `TapEntryKind::TaskReport` entry to the subscriber's tape. No LLM turn is triggered.

### What NOT To Do

---

## Tool Call Loop Breaker in `agent/loop_breaker.rs`

### What

Detects when the agent is stuck calling the same tool repeatedly without progress and intervenes with escalating strategies. Works alongside `RepetitionGuard` (text-level) — this handles tool-call-level repetition.

### Three Detection Patterns

| Pattern | Trigger | Intervention |
|---------|---------|--------------|
| Exact duplicate | Same `(tool, args)` ≥3 times consecutively | Disable tool |
| Same-tool flooding | Same tool ≥5 calls → warn; ≥8 calls → disable | Warn then disable |
| Ping-pong | A-B-A-B alternation ≥4 cycles | Disable both tools |

### Integration in Agent Loop

1. Initialized alongside other turn state (near `consecutive_silent_iters`)
2. Records tool calls after execution, checks for patterns
3. Injects warning via `loop_breaker_warning` (same pattern as `context_pressure_warning`)
4. Disables tools by removing from `tool_defs`

### What NOT To Do

- Do NOT weaken thresholds without understanding the failure mode — the defaults (warn=5, disable=8, exact_dup=3, pingpong=4) are based on real production incidents where agents burned 25 iterations on tape search loops
- Do NOT remove the `DisableTools` intervention — warning alone is insufficient; small models ignore warnings and continue looping
- Do NOT apply loop breaking to tool *results* — only track tool *calls* (name + args). Results are variable and should not trigger detection

---

## Background Task Delegation

Two tools for spawning background agents:

- **`task`** (Core tier) — high-level preset-based delegation. LLM picks a
  `task_type` (`general-purpose` or `bash`) and provides a prompt. System
  prompt, tools, and limits are resolved from presets in
  `tool/task/presets.rs`. This is the primary delegation interface.

- **`spawn-background`** (Deferred tier) — low-level escape hatch. LLM
  provides raw `system_prompt`, `tools`, `model`, and `max_iterations`.
  Use only when presets don't fit.

Both tools share the same underlying machinery: `spawn_child` +
`register_background_task` + fire-and-forget result delivery via proactive
turn.

**Anti-nesting invariant:** Task presets set `excluded_tools` on the child
`AgentManifest` to prevent recursive subagent spawning. The exclusion list
includes `task`, `spawn-background`, and `create-plan`.

### What NOT To Do

- Do NOT add new task presets without setting `excluded_tools` — omitting the exclusion list allows the child agent to spawn its own children, leading to unbounded recursion
- Do NOT bypass `presets.rs` by copying preset logic inline — all preset definitions must live in one place for auditability

---

- Do NOT publish TaskReport without going through the syscall — `exec_publish_report` enforces `source_session` and `tags` invariants
- Do NOT use `UserId("system")` in synthetic messages for ProactiveTurn — always use the subscription owner's identity to prevent privilege escalation on session restore
- Do NOT construct `TaskNotification` outside `handle_publish_task_report` — it builds the `TaskReportRef` and coordinates result persistence
