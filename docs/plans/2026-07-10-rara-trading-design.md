# rara Trading — 产品设计

> Status: 产品设计已定稿，待拆解实现计划。
> Author: Ryan + Claude
> Date: 2026-07-10

## Summary

把量化交易做成 **rara 的一项能力**，而不是一个外挂的独立系统。

用户不是在用一个交易软件——用户是在跟自己的 rara 说「帮我盯盘 / 研究 / 下单」。
rara 有多年记忆、会主动提醒、跨 Telegram / TUI / Web 陪着用户。底层交易引擎
**自主运行**，rara 作为副驾替用户看着、研究、必要时代为操作（改状态需用户确认一次）。

这跟 rara 自身的 North Star 同频：`goal.md` 里「注意到用户每周一看股票，自己建个
定时任务」——交易本就是 rara 要主动承担的一类生活/工作事务。

前身是独立的 Go 项目 `trading-agent`（signal-driven 自主交易基建）。本设计将其
**核心交易能力用 Rust 重写进 rara**，Python 策略/回测原样保留（经 sandbox 驱动），
Go 的编排层由 rara kernel 覆盖，Go 的半成品 agent 层删除。

## Design Decisions

| 决策 | 选项 | 理由 |
|------|------|------|
| Agent 归属 | rara 是唯一 agent | 不在交易引擎里再造一个 agent；rara 就是个人 agent |
| 集成方式 | Rust 原生 extension，**不走 MCP/进程边界** | 用户要 tool 直接可执行；kernel Tool 直调 |
| 交易引擎哲学 | 自主交易（沿用 trading-agent 现状） | LLM 是监控/副驾层，不是每单审批的 gatekeeper |
| 操作审批 | confirm-on-mutation | 引擎自主成交不打扰；**rara 主动发起的改状态操作**要用户确认一次（走 Guard） |
| 主表面 | 对话优先（多通道） | rara 价值在 agent + 记忆 + 主动 + 多通道，不是重型交易终端 |
| 可视化 | Web UI 承载（K线/回测曲线/持仓） | 交易有天生视觉的东西；rara 已有 web channel + browser driver |
| 策略/回测 | 保留 Python + vectorbt，rara-sandbox 驱动 | 研究闭环是最值钱资产，语言无关，零重写 |
| 新写代码 | crates/extensions/rara-trading | 交易所执行 + OMS + 信号/仓位；调研 barter-rs 复用 |

## Product Shape

### 身份
rara = 用户的私人量化操盘副驾。交易是 rara 的能力，不是独立软件。

### 表面（surfaces）
| 通道 | 角色 |
|------|------|
| 对话（Telegram / TUI / Web-chat） | **主入口**：研究 / 盯盘 / 下单 / 策略进化全靠说 |
| Web UI | 承载天生视觉的东西：K线、回测资金曲线、持仓/盈亏。看一眼用，非重型终端 |
| 主动推送 | rara heartbeat：异动、成交、新闻、策略表现，主动找用户 |

### 能力地图（研究 / 盯盘 / 运维 / 情报，四项全要）
| 能力 | 用户怎么用 | 底层 |
|------|-----------|------|
| 研究台 | "回测 rsi2 在 15m，跟 v2 比" | Python 策略 + vectorbt，跑在 rara-sandbox |
| 盯盘 Copilot | "现在什么情况" "为啥开这单" | rara memory + 引擎持仓/信号 |
| 运维操作员 | "上 testnet" "平一半 ETH" "halt" | 新 Rust 执行/OMS，**改状态需确认** |
| 情报分析 | "有影响我持仓的新闻吗" | rara data_feed + LLM |

### 控制模型
- 引擎**自主交易**，成交不打扰用户。
- rara **主动发起的改状态操作**（部署/下架策略、手动下单、调仓位上限、halt）
  → 走 rara 的 **Guard**，用户**确认一次**。
- 不是 OpenAlice 的每单审批，是「agent 动手前点头」。

## Architecture

```
┌──────────────────────────────────────────────────────────┐
│  rara kernel (既有)                                        │
│  LLM · Tape 记忆 · Guard · Proactive · Scheduler          │
│  Channels: Telegram / TUI / Web        data_feed · sandbox │
└───────────────┬──────────────────────────────────────────┘
                │  kernel Tool 直调（原生 Rust，无进程边界）
┌───────────────▼──────────────────────────────────────────┐
│  crates/extensions/rara-trading (新写 Rust)               │
│  · 交易所执行 / OMS / 下单        (调研 barter-rs)         │
│  · 信号 / Fusion / 仓位·组合                               │
│  · 策略运行时 & 回测编排 ──► rara-sandbox 跑 Python+vectorbt│
└──────────────────────────────────────────────────────────┘
```

### rara 里 新建 vs 复用 vs 删
- **新建 Rust**：crates/extensions/rara-trading = 交易所执行 + OMS + 信号/仓位
- **保留 Python**：策略 + vectorbt 回测，经 rara-sandbox 驱动
- **复用 rara**：LLM · Tape 记忆 · Guard · Proactive · Scheduler · Channels · data_feed
- **删除**：trading-agent 的 Go internal/agent/，及 rara 已覆盖的 Go 编排层

## 从 trading-agent 迁移映射

| trading-agent (Go) | 去向 |
|--------------------|------|
| Python 策略 + vectorbt 回测 | 保留，rara-sandbox 跑 |
| 调度 / 记忆 / guard / 通道 / LLM | 删，用 rara kernel |
| 市场数据摄取（CCXT / TimescaleDB） | 移植进 data_feed + rara-trading |
| 交易所执行 / OMS / 下单 | Rust 重写（barter-rs 调研） |
| 信号 / Fusion / 仓位组合 | Rust 重写（逻辑量不大） |
| internal/agent/（半成品 agent 框架） | 删 |

## 参考

- OpenAlice — file-driven personal trading agent，trading-as-git 审批模型（我们只借鉴
  工作台 UX，不借鉴每单审批）。
- Hermes Agent — rara 自身 North Star 参照的个人 agent 品类。
- 前身 trading-agent Go 项目（同 org），已实现完整自主交易 pipeline + 策略进化闭环。

## Open Questions（下一步实现计划要解决）

1. **落地顺序**：增量（先研究台：sandbox 跑 Python 回测作为原生 tool，最低风险）
   vs 一次性 port 执行引擎？—— rara 信奉 "Slow is Fast"，倾向增量。
2. **Rust 执行层选型**：barter-rs 能覆盖多少 CCXT 交易所？testnet（Bybit/OKX）支持？
3. **策略运行时契约**：Go 现有 IStrategy（populate_indicators/entry/exit）如何在
   Rust↔Python(sandbox) 边界保留。
4. **市场数据存储**：复用 rara 现有 data_feed store，还是引入 TimescaleDB？
