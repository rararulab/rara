# rara Trading — 产品设计

> Status: 产品设计已定稿；金融信息订阅 MVP 优先，研究台随后实施。
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
| 操作审批 | mandate + confirm-on-mutation | 引擎只能在已确认的交易授权内自主成交；rara 主动发起的改状态操作须由 Guard 确认 |
| 主表面 | 对话优先（多通道） | rara 价值在 agent + 记忆 + 主动 + 多通道，不是重型交易终端 |
| 可视化 | Web UI 承载（K线/回测曲线/持仓） | 交易有天生视觉的东西；rara 已有 web channel + browser driver |
| 策略/回测 | 保留 Python + vectorbt，受约束的独立 sandbox 驱动 | 研究闭环是最值钱资产，语言无关，零重写 |
| 新写代码 | crates/extensions/rara-trading | 交易所执行 + OMS + 信号/仓位；调研 barter-rs 复用 |

## Product Shape

### 身份
rara = 用户的私人量化操盘副驾。交易是 rara 的能力，不是独立软件。

### 表面（surfaces）
| 通道 | 角色 |
|------|------|
| 对话（Telegram / TUI / Web-chat） | **主入口**：研究 / 盯盘 / 下单 / 策略进化全靠说 |
| Web UI | 承载天生视觉的东西：K线、回测资金曲线、持仓/盈亏。看一眼用，非重型终端 |
| 主动推送 | rara heartbeat：异动、最新 K 线、成交、新闻、策略表现，主动找用户 |

### 能力地图（研究 / 盯盘 / 运维 / 信息，四项全要）
| 能力 | 用户怎么用 | 底层 |
|------|-----------|------|
| 研究台 | "回测 rsi2 在 15m，跟 v2 比" | Python 策略 + vectorbt，跑在受约束的独立 sandbox |
| 盯盘 Copilot | "现在什么情况" "为啥开这单" | rara memory + 引擎持仓/信号 |
| 运维操作员 | "上 testnet" "平一半 ETH" "halt" | 新 Rust 执行/OMS，**改状态需确认** |
| 信息订阅 | "订阅 BTC 15m K线和美联储新闻" | rara data_feed + 订阅匹配 + LLM 摘要 |

### 当前落地顺序

1. **金融信息订阅**：RSS/Atom 新闻源 + 最新已收盘 K 线 → 标的/主题/时间框订阅 → 限流的主动推送或静默入记忆；不涉及策略、账户或订单。
2. **研究台**：隔离 sandbox 跑 Python/vectorbt 回测，产出可复现 artifact；不涉及部署或下单。
3. **执行**：只在 mandate、OMS、reconciliation 和硬风控齐备后，从 paper/testnet 开始。

对应实现计划：

- `docs/plans/2026-07-10-rara-finance-subscriptions-implementation.md`
- `docs/plans/2026-07-10-rara-trading-research-mvp-implementation.md`

### 第一阶段：金融信息订阅（新闻 + 最新 K 线，无交易能力）

金融信息订阅复用 rara 的 `data_feed`、Tape、主动消息和多通道。运营者配置可信 RSS/Atom
新闻源和 market candle source；用户通过对话订阅来源、分类、watch term、symbol 与
timeframe（如 BTC 15m、NVDA 1d、美联储）。新闻条目会被规范化、去重并持久化为
`FeedEvent`；最新已收盘 K 线会同时写入 `FeedEvent` 和专用 TSDB-backed
`MarketDataRepository`。订阅匹配后：`immediate` 才启动一次主动 rara turn，`silent`
只写入会话记忆。两种模式都不做交易建议，更不触发任何订单。

第一版不让 LLM 任意抓取 URL，也不让 LLM 任意拉取 ticker 行情。来源 URL、market data
provider、symbol 白名单和 timeframe 白名单归运营者管理。新闻匹配使用确定性文本和分类
标签；K 线匹配使用 provider/source、symbol、venue 和 timeframe。主动消息用
cooldown/每小时预算控制噪音。

K 线订阅首批只推送 `market_candle_closed`，即最近一个已收盘 bar。未收盘 bar 的
`market_candle_update` 高频且会反复修订，后续作为显式高频模式增加；默认不进入对话主动
推送。K 线 payload 使用字符串化 Decimal 表达 open/high/low/close/volume，事件 ID 使用
`source + venue + symbol + timeframe + open_time` 的确定性键，重复轮询或重启不能重复通知。
TSDB 主键使用 `source + venue + symbol + timeframe + open_time`，允许 provider 修正同一根
bar 时按版本/ingested_at 记录可追溯更新。

### 控制模型
- 引擎**自主交易**，但只能在一个已确认、不可变的 `TradingMandate` 内成交；成交本身
  不打扰用户。
- `TradingMandate` 是部署时 Guard 确认的精确对象，至少包含：策略版本/hash、账户与
  `paper` / `testnet` / `live` 环境、交易对白名单、仓位和名义金额上限、杠杆、止损/日损
  阈值、有效期，以及撤销条件。修改任一字段就是新的改状态操作，必须重新确认。
- rara **主动发起的改状态操作**（部署/下架策略、改变 mandate、手动下单、调仓位上限）
  → 走 rara 的 **Guard**，用户**确认一次**。手动单确认的是精确的 `OrderIntent`
  （账户、方向、数量、价格保护、client-order-id），不复用某次部署授权。
- 风控阈值命中的 `halt` 是引擎本地、立即生效的硬保护，不能依赖 LLM、Guard 或消息通道。
  rara 提议的非紧急 halt 仍走 Guard；两类 halt 都必须写入审计记录。
- 不是 OpenAlice 的每单审批，而是「用户先批准边界，引擎只在边界内自动执行」。

### 第二阶段：研究台 MVP（无交易能力）

第二阶段只交付原生 `trading_backtest` tool，验证“与 rara 对话做量化研究”的体验。它
**没有**部署、下单、调仓、交易所连接或读取交易所凭据的能力；研究产物也不能直接变成
运行中的策略。

| 项目 | MVP 决定 |
|------|----------|
| Tool 输入 | `BacktestSpec + StrategyRef + DatasetRef`，而不是模型可任意拼接的 Python 命令 |
| 策略输入 | `StrategyRef` 指向版本化策略文件及内容 hash；先冻结一个只读、确定性的 Python 策略 ABI |
| 数据输入 | `DatasetRef` 指向已导入、带 schema/hash/时间范围/时区的行情数据集 |
| 执行隔离 | 每次回测使用独立 sandbox：禁网、无交易所凭据、策略与数据只读挂载；只允许写入本次 artifact 目录 |
| 输出 | `BacktestArtifact`：结果表、资金曲线/交易明细，以及完整可复现 manifest |

`BacktestArtifact` 的 manifest 必须记录策略 hash、数据集 hash、Python/vectorbt 镜像与依赖
锁、时间框与时区、warmup、手续费、滑点、撮合/填充模型和随机种子。相同输入必须可以重跑；
结果不满足该条件，就不是可用于讨论或演进的研究结论。

策略 ABI 首批只支持确定性回测所需的最小子集：输入 OHLCV schema、指标、entry/exit 信号
和 warmup。禁止策略自行取数或出网；跨时间框、动态参数搜索和实盘运行时语义在后续阶段
扩展，不能隐式沿用 Go 的行为。

### 市场数据边界

`data_feed` 继续承载新闻、最新 K 线、告警、异动和成交通知等通用事件，不作为 OHLCV
历史库。第一阶段一旦摄取 K 线，就同时引入专门的 `MarketDataRepository`，生产实现选
TimescaleDB/PostgreSQL hypertable 保存 OHLCV；`data_feed` 只保存通知事件、原始 payload
摘要和去重水位线。`MarketDataRepository` 负责 OHLCV 的分区、补洞、去重、provider 修正、
resample 与数据质量。第二阶段的 `DatasetRef` 可以从 TSDB 固化导出为带 hash 的
Parquet/CSV artifact；执行阶段也只读取该 repository，不从 `FeedEvent` 表拼历史。

TSDB 选型首发固定为 TimescaleDB/PostgreSQL，而不是 ClickHouse、QuestDB 或 InfluxDB。
原因是 MVP 更重视精确 upsert、修正审计、普通 SQL、事务、Rust `sqlx` 集成和后续与账户/
配置数据的关系查询。ClickHouse/QuestDB 留作后续高吞吐行情湖：当我们需要全市场 tick 级
摄取、跨资产大扫描或长期冷数据分层时，再通过 `MarketDataRepository` 增加实现，不改上层。

容量假设按上百个标的设计。100 个标的即使订阅 1m/5m/15m/1d，日新增 bar 量也仍是
TimescaleDB 的轻量级负载；真正要避免的是按用户订阅或按 symbol 启动独立抓取任务。行情
摄取必须按 provider/venue/timeframe 批量运行，一个 source 覆盖一组 symbol，订阅层只消费
已摄取的事件和 TSDB 数据，不触发额外 provider 请求。

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
│  · 金融订阅：RSS article + latest candle 匹配与投递          │
│  · 研究台：回测编排 ──► 独立、禁网 sandbox 跑 Python+vectorbt│
│  · 执行阶段：交易所执行 / OMS / 下单      (调研 barter-rs)   │
│  · 执行阶段：信号 / Fusion / 仓位·组合                     │
└──────────────────────────────────────────────────────────┘
```

### rara 里 新建 vs 复用 vs 删
- **新建 Rust**：crates/extensions/rara-trading = 金融信息订阅（第一阶段：新闻 + 最新 K 线）+ 研究台（第二阶段）；后续交易所执行 + OMS + 信号/仓位
- **保留 Python**：策略 + vectorbt 回测，经受约束的回测 runner 驱动
- **复用 rara**：LLM · Tape 记忆 · Guard · Proactive · Scheduler · Channels · data_feed
- **新增专用边界**：TSDB-backed MarketDataRepository、版本化 Dataset/Artifact store；后续 Trading ledger
- **删除**：trading-agent 的 Go internal/agent/，及 rara 已覆盖的 Go 编排层

## 从 trading-agent 迁移映射

| trading-agent (Go) | 去向 |
|--------------------|------|
| Python 策略 + vectorbt 回测 | 保留，经独立、禁网的回测 runner 跑 |
| 调度 / 记忆 / guard / 通道 / LLM | 删，用 rara kernel |
| 市场数据摄取（CCXT / TimescaleDB） | 最新 K 线/通知类事件进 data_feed；OHLCV 历史进入专用 MarketDataRepository |
| 交易所执行 / OMS / 下单 | Rust 重写（barter-rs 调研） |
| 信号 / Fusion / 仓位组合 | Rust 重写（逻辑量不大） |
| internal/agent/（半成品 agent 框架） | 删 |

## 参考

- OpenAlice — file-driven personal trading agent，trading-as-git 审批模型（我们只借鉴
  工作台 UX，不借鉴每单审批）。
- Hermes Agent — rara 自身 North Star 参照的个人 agent 品类。
- 前身 trading-agent Go 项目（同 org），已实现完整自主交易 pipeline + 策略进化闭环。

## 执行阶段的准入条件

研究台完成不等于可以开始碰真钱。以下条件全部满足后，才开始 paper/testnet 执行；live
在 testnet 验证完成后另行批准：

1. **OMS 状态一致性**：持久化订单/仓位 ledger 与订单状态机；每个 `OrderIntent` 使用稳定
   `client-order-id` 作为幂等键。
2. **交易所 reconciliation**：下单/撤单超时、断线和重启后，从交易所拉取订单与成交并收敛
   本地状态；未知提交状态必须 fail closed，禁止盲目重试。
3. **集中风控**：所有下单路径（策略、手动、恢复）经过同一 mandate 与限额检查；账户级并发
   控制、限速、时钟异常处理和本地硬 halt 已测试。
4. **可审计性**：每笔动作可追溯到用户、Guard 决定、`TradingMandate`、策略版本和
   `BacktestArtifact`；密钥永不进入策略 sandbox、聊天上下文或审计载荷。
5. **集成形态**：`rara-trading` 作为 workspace 的编译期 extension，由 app 显式注册 tool、
   配置、生命周期 hook、迁移与只读 Web 路由；它不是动态插件。

## Remaining Open Questions（进入执行阶段前调研）

1. **Rust 执行层选型**：barter-rs 覆盖哪些目标交易所？Bybit/OKX testnet 的下单、撤单、
   websocket 成交回报和 reconciliation 是否足够？不足部分的 adapter 边界如何设计？
2. **资金与账户模型**：首发限定现货、永续或两者？多账户/子账户、保证金模式、币种精度和
   最小下单单位如何映射为强类型约束？
