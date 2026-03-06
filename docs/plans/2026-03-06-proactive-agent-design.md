# Proactive Agent Design — Mita + Rara 分层架构

## 背景

Rara 当前是纯 reactive 模式：用户发消息 → 回复。需要实现 proactive 能力，使 Rara 能主动发起对话、follow up、在群聊中自主决定是否发言。

主要场景为 Telegram：私聊（多 session）+ 群聊（单 session）。

## 架构总览

```
用户 (Telegram)
  私聊 session A, B, C...  │  群聊 session G
         │                         │
   ┌─────▼─────────────────────────▼─────┐
   │           Rara (reactive)            │
   │  · 用户消息 → 加载 tape → 回复        │
   │  · 群聊：轻量 LLM 判断要不要回复       │
   │  · 不回复的消息也记 tape              │
   │  · 实时提取关键信息写入 user tape      │
   └─────────────────────────────────────┘

   ┌─────────────────────────────────────┐
   │         Mita (proactive)             │
   │  · 心跳定时器驱动（默认 30 分钟）      │
   │  · 有自己的 tape                     │
   │  · 只做决策，不做执行                  │
   │  · 通过 dispatch_rara() 下达指令      │
   │  · 对用户完全不可见                    │
   └─────────────────────────────────────┘
```

## Agent 分层

### Rara（用户可见）

- 触发方式：用户消息
- 职责：对话、工具执行、实时信息提取
- Tape 访问：当前 session tape + 相关 user tape(s)
- 群聊中有轻量 "要不要回复" 决策能力

### Mita（后台隐藏）

- 触发方式：心跳定时器（默认 30 分钟，可配置）
- 职责：跨 session 观察、判断是否需要主动行动、给 Rara 下指令
- Tape 访问：自己的 tape + 可读所有 session tape 和 user tape
- 对用户完全不可见，Rara 是唯一人格

两者在 AgentRegistry 中共存：

```
AgentRegistry
  ├─ rara  (用户可见, reactive)
  └─ mita  (后台, proactive, 心跳驱动)
```

## Tape 分层

### Session Tape

- 粒度：per session
- 内容：对话消息流水（用户消息 + agent 回复）
- 群聊中所有消息都记录（包括 Rara 未回复的）
- 私聊中用户可切换 session

### User Tape

- 粒度：per user
- 内容：Rara 对该用户的认知、偏好、待办事项、观察
- 跨 session 持久化
- 写入来源：
  - Rara 实时提取（对话中明显的信息）
  - Mita 心跳补充（深层观察、跨 session 关联）

### Mita Tape

- 粒度：单一（Mita 只有一个实例）
- 内容：每次心跳的思考过程、决策记录
- 用途：避免重复行动、积累跨心跳观察

## 消息触发型 Proactive（群聊主动回复）

属于 Rara 自身的能力，不经过 Mita。

```
群消息到达
→ 记入群聊 session tape
→ 第一步：轻量 LLM 调用（最近几条消息 + 短上下文）
   → "需要回复吗？"
   → 不需要 → 结束
   → 需要 → 第二步
→ 第二步：完整 LLM 调用（session tape + user tapes）
   → 生成回复 → 发送
→ Rara 实时提取关键信息写入相关 user tape
```

## 时间触发型 Proactive（Mita 心跳）

```
30 分钟定时器触发
→ 加载 Mita tape（知道上次做了什么）
→ LLM + 工具循环：
   1. list_sessions()    — 查看活跃 session 元数据
   2. read_tape()        — 深入读感兴趣的 session tape
   3. 判断是否需要行动
   4. dispatch_rara(session_id, instruction)  — 给 Rara 下达指令
→ Mita 的思考和决策记入 Mita tape
→ Rara 收到指令 → 加载对应 tape → LLM 生成具体消息 → 发送
```

### Mita 工具集

| 工具 | 用途 |
|------|------|
| `list_sessions()` | 获取所有活跃 session 的元数据 |
| `read_tape(session_id, recent_n)` | 读取指定 session/user 的 tape |
| `dispatch_rara(session_id, instruction)` | 给 Rara 下达行动指令 |

Mita 没有通用工具（http、bash 等），只做判断和调度。

## 上下文加载策略

Rara 每次对话加载：

1. 当前 session tape（对话历史）
2. 相关 user tape(s)（对用户的认知）

群聊中可能涉及多个用户，加载所有参与者的 user tape。

## 信息回写

| 来源 | 时机 | 写入目标 | 内容 |
|------|------|----------|------|
| Rara | 实时（每次对话后） | User tape | 明显信息：偏好、指令、事实 |
| Mita | 心跳（30 分钟） | User tape | 深层观察：跨 session 关联、行为模式 |

## 分阶段实现建议

### Phase 1：User Tape + 上下文加载

- 实现 user tape 的存储和读写
- Rara 对话时加载 session tape + user tape
- Rara 实时提取信息写入 user tape

### Phase 2：群聊主动回复

- 群聊消息全量记入 tape
- 实现两步 LLM 判断（轻量判断 + 完整回复）

### Phase 3：Mita Agent

- 实现 Mita agent + 心跳定时器
- 实现 Mita 工具集（list_sessions、read_tape、dispatch_rara）
- 实现 dispatch_rara 指令投递机制

### Phase 4：信息回写闭环

- Mita 心跳时深层分析 + 回写 user tape
- Tape 增长管理（摘要/压缩策略）
