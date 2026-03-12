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
- Parsing uses serde deserialization — adding a new variant to the enum automatically works in config

### What NOT To Do

- Do NOT change the `#[default]` attribute away from `MentionOrSmallGroup` — existing deployments have no `group_policy` in their config. Changing the default silently changes their bot behavior (e.g. switching to `All` would make the bot reply to every group message, spamming the chat).
- Do NOT add manual string parsing for GroupPolicy — the enum already has `#[serde(rename_all = "snake_case")]`. Manual match-arms duplicate the variant list and will silently ignore new variants when someone adds one to the enum. Serde handles both directions automatically.
- Do NOT put group policy logic in the kernel — the `MentionOrSmallGroup` variant calls `bot.get_chat_member_count()`, which is a Telegram Bot API call. The kernel has no Telegram dependency and must not acquire one. Policy evaluation requires platform-specific context that only the adapter has.
