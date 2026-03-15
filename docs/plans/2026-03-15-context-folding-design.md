# Context Folding Design

> Issue: #341
> 依赖: #339 (background agent), #321 (上下文无限增长)

## Summary

让 agent 主动管理自己的上下文窗口，而不是被动依赖硬编码截断规则。借鉴 [RLM](https://www.primeintellect.ai/blog/rlm) 的 context folding 理念，在 Rara 现有的 tape + anchor 架构上实现三层折叠机制。

核心原则：**agent 自己决定何时折叠、折叠什么**。

## Design Decisions

| 决策 | 选项 | 理由 |
|------|------|------|
| 折叠触发 | 上下文压力阈值 + agent 主动 | 两者互补：阈值兜底，agent 可提前 fold |
| 摘要生成 | 独立 LLM 调用（不走主 loop） | 避免递归，摘要调用用短上下文 |
| 分支隔离 | fork_tape + spawn_child | 复用已有基础设施 |
| 摘要存储 | `HandoffState` via `TapeService::handoff()` | 直接复用已有类型和方法，`anchor_context()` 零改动兼容 |
| ContextFolder 位置 | kernel 层（`agent/fold.rs`） | fold 涉及 LLM 调用是 orchestration 逻辑，memory 模块只负责 tape 存取 |
| 与 RLM 差异 | 不引入 Python REPL | Rara 的 tool 系统已等价于 REPL |

## 现状分析

Rara 已有的上下文管理基础设施：

```
已有                                    缺少
──────────────────────────────────────────────────────────
✅ Tape 每次迭代重建上下文               ❌ 自动 anchor 创建
✅ Anchor + HandoffState 做上下文检查点  ❌ Anchor 摘要由 LLM 自动生成
✅ fork_tape / discard_tape             ❌ fork 完成后自动压缩结果回父
✅ Child agent 独立 tape                ❌ Child 结果自动压缩回父上下文
✅ 两层工具输出截断                      ❌ 对话历史本身的折叠压缩
✅ classify_context_pressure (0.70/0.85) ❌ 压力驱动的自动折叠（在 warn 之前）
✅ anchors(tape_name, limit) 查询历史    ❌ 历史回溯时利用 anchor chain
```

## Architecture

### 与现有 context pressure 的关系

现有 `agent.rs` 中已有两级压力机制：

```
0.0 ──── 0.60 ────── 0.70 ──────── 0.85 ──── 1.0
          │           │              │
     FOLD_THRESHOLD   │         CRITICAL
      (auto-fold)   WARNING     (must handoff)
                   (should handoff)
```

Auto-fold 在 0.60 触发，正常情况下 fold 成功后压力回落，0.70/0.85 的提示永远不会触发。如果 fold 失败或摘要质量差，现有 0.70/0.85 机制作为 fallback 继续生效。

### 防震荡：Cooldown 机制

Fold 后上下文缩短 → 几轮后涨回 0.6 → 再次 fold → 循环。需要 cooldown：

```
触发 fold 的条件（必须全部满足）：
  1. pressure > FOLD_THRESHOLD (0.60)
  2. 距上次 auto-fold anchor 之后，新增 entry 数 >= min_entries_between_folds (15)
  3. context_folding.enabled == true
```

用 entry 数而非时间/轮数作为 cooldown 指标——entry 数直接反映上下文增长量，比 turn 数更精确（一个 turn 可能有多次 tool call = 多条 entry）。

**注意**：cooldown 只看 `phase == "auto-fold"` 的 anchor，不看用户手动 handoff 或 session start anchor。否则用户手动 handoff 后会意外重置 cooldown 计数。实现方式：agent loop 中维护 `last_fold_entry_id: Option<u64>`，每次 fold 后记录新 anchor 的 entry ID，检查 cooldown 时用 `tape.entries_after(last_fold_entry_id).count()` 而非通用的 entries_since_last_anchor。

### 整体流程

```
Agent Turn Iteration
  │
  ├─ 1. 检测上下文压力（复用现有 classify_context_pressure）
  │     tape_info = tape.info(tape_name)
  │     pressure = tape_info.estimated_context_tokens / context_window_tokens
  │
  ├─ 2. 判断是否触发 fold
  │     pressure > FOLD_THRESHOLD
  │       AND entries_since_last_fold >= min_entries_between_folds
  │     ├─ YES → 触发 Auto-Anchor fold
  │     │    ├─ 独立 LLM 调用：总结当前上下文
  │     │    ├─ TapeService::handoff() 创建 anchor
  │     │    └─ 下次 rebuild 自动只读 anchor 后的 entries
  │     └─ NO → 继续（现有 0.70/0.85 机制照常工作）
  │
  ├─ 3. rebuild_messages_for_llm()
  │     ├─ 读取 anchor 后的 entries
  │     ├─ 注入 anchor 摘要作为 system message（anchor_context 已有逻辑）
  │     └─ 应用两层截断（已有逻辑不变）
  │
  ├─ 4. LLM 调用
  │     ├─ agent 可能调用 fold_branch tool（P1）
  │     │    ├─ fork tape + spawn child
  │     │    ├─ child 在独立上下文中执行子任务
  │     │    ├─ 等待完成，压缩结果
  │     │    └─ 返回压缩结果作为 ToolResult
  │     └─ 正常 tool 调用
  │
  └─ 5. 追加结果到 tape，继续迭代
```

## Components

### P0: Auto-Anchor（对话级折叠）

最小改动，最大收益。解决长对话 context rot 问题。

#### 1. ContextFolder

Orchestration 模块，放在 kernel 层（与 agent loop 同级），不放 memory 模块。memory 只负责 tape 存取。

```rust
// crates/kernel/src/agent/fold.rs

use crate::llm::{LlmDriver, Message};
use crate::memory::HandoffState;

pub struct FoldSummary {
    /// 当前上下文的关键信息摘要
    pub summary: String,
    /// 下一步需要做什么
    pub next_steps: String,
}

pub struct ContextFolder {
    /// 用于摘要的 LLM driver
    driver: Arc<dyn LlmDriver>,
    /// 摘要用的模型标识符（provider-agnostic）
    model: String,
}

impl ContextFolder {
    /// 将一组 messages 折叠为摘要。
    ///
    /// 使用独立的短上下文 LLM 调用，不走主 agent loop。
    /// max_summary_tokens 根据被压缩内容的量动态计算。
    pub async fn fold_with_prior(
        &self,
        prior_summary: Option<&str>,
        messages: &[Message],
        source_token_estimate: usize,
    ) -> Result<FoldSummary> {
        // 动态摘要长度：原内容的 ~10%，下限 256，上限 2048
        let max_tokens = (source_token_estimate / 10).clamp(256, 2048);

        let fold_prompt = Message::system(FOLD_SYSTEM_PROMPT.to_string());

        let mut content = String::new();
        if let Some(prior) = prior_summary {
            content.push_str(&format!("## 之前的对话历史\n{}\n\n", prior));
        }
        content.push_str("## 需要总结的新对话\n");
        content.push_str(&self.format_messages_for_fold(messages));

        let user_msg = Message::user(content);

        let response = self.driver.chat(
            &self.model,
            &[fold_prompt, user_msg],
            &ChatOptions {
                max_tokens: Some(max_tokens as u32),
                temperature: Some(0.0),
                ..Default::default()
            },
        ).await?;

        self.parse_fold_response(&response)
    }

    /// 将纯文本压缩到目标字符数。
    ///
    /// 用于 P1 fold_branch：子 agent 返回的结果文本可能很长，
    /// 需要压缩后再作为 ToolResult 写回父上下文。
    pub async fn fold_text(&self, text: &str, target_chars: usize) -> Result<String> {
        let prompt = Message::system(
            "Compress the following text to be concise while preserving all key facts, \
             decisions, and actionable information. Use the same language as the input. \
             Output ONLY the compressed text, no wrapper."
                .to_string(),
        );
        let user_msg = Message::user(format!(
            "Compress to ~{target_chars} characters:\n\n{text}"
        ));

        let max_tokens = (target_chars / 3).clamp(128, 2048) as u32; // rough char→token
        let response = self.driver.chat(
            &self.model,
            &[prompt, user_msg],
            &ChatOptions {
                max_tokens: Some(max_tokens),
                temperature: Some(0.0),
                ..Default::default()
            },
        ).await?;

        Ok(response.text)
    }

    /// 将 FoldSummary 转为 HandoffState，直接复用现有 anchor 体系。
    pub fn to_handoff_state(summary: &FoldSummary, pressure: f64) -> HandoffState {
        HandoffState {
            phase: Some("auto-fold".into()),
            summary: Some(summary.summary.clone()),
            next_steps: Some(summary.next_steps.clone()),
            source_ids: vec![],
            owner: Some("system".into()),
            extra: Some(serde_json::json!({
                "trigger": "context_pressure",
                "pressure_at_fold": pressure,
            })),
        }
    }
}

const FOLD_SYSTEM_PROMPT: &str = r#"You are a context compression specialist.
Given a conversation history, produce two parts:

1. **summary**: Key information summary. MUST preserve:
   - User identity and preferences
   - All factual information (file paths, code state, config values)
   - Decisions made and their reasoning
   - Errors encountered and solutions attempted
   DELETE: greetings, redundant tool outputs, intermediate reasoning steps

2. **next_steps**: Work currently in progress or about to begin.

Output JSON: {"summary": "...", "next_steps": "..."}
IMPORTANT: Generate the summary in the SAME LANGUAGE as the conversation being summarized."#;
```

#### 2. Agent Loop 集成

```rust
// crates/kernel/src/agent.rs — run_agent_loop 内

const FOLD_THRESHOLD: f64 = 0.60;

// agent loop 开始前初始化（与 consecutive_silent_iters 等同级）
let mut last_fold_entry_id: Option<u64> = None;

// 在每次迭代的 rebuild_messages_for_llm 之前，
// 复用现有 tape.info() + classify_context_pressure 检测压力
if let Ok(tape_info) = tape.info(tape_name).await {
    let pressure = tape_info.estimated_context_tokens as f64
        / capabilities.context_window_tokens as f64;

    if pressure > FOLD_THRESHOLD {
        // Cooldown 检查：距上次 fold anchor 后是否有足够的新 entry
        let entries_since_last_fold = tape_info.entries_since_last_anchor;
        let min_entries = config.context_folding.min_entries_between_folds; // 默认 15

        // Cooldown: 只看 auto-fold anchor，不看用户手动 handoff
        let entries_since_last_fold = match last_fold_entry_id {
            Some(id) => tape.entries_after(tape_name, id).await?.len(),
            None => tape_info.total_entries, // 从未 fold 过，全部 entry 都算
        };

        if entries_since_last_fold >= min_entries {
            tracing::info!(
                pressure = %pressure,
                entries_since_fold = entries_since_last_fold,
                "auto-fold: context pressure {:.0}% exceeded threshold, creating anchor",
                pressure * 100.0,
            );

            let messages = tape.build_llm_context(tape_name).await?;

            // 层级折叠：获取前一个 anchor 的摘要
            let prior_summary = anchor_summary_text(
                &tape.from_last_anchor(tape_name).await?,
            );

            // fold 失败不中断 agent loop，fallback 到现有 0.70/0.85 机制
            match context_folder.fold_with_prior(
                prior_summary.as_deref(),
                &messages,
                tape_info.estimated_context_tokens as usize,
            ).await {
                Ok(fold) => {
                    let handoff_state = ContextFolder::to_handoff_state(&fold, pressure);
                    tape.handoff(tape_name, "auto-fold", handoff_state).await?;
                    // 记录 fold anchor 的 entry ID 用于 cooldown
                    last_fold_entry_id = tape.last_entry_id(tape_name).await.ok();
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "auto-fold: LLM summarization failed, skipping fold; \
                         0.70/0.85 pressure warnings remain as fallback"
                    );
                }
            }
        }
    }
}
```

#### 3. Fold 模型配置

```yaml
# config.yaml
kernel:
  context_folding:
    enabled: true
    fold_threshold: 0.60             # 上下文压力阈值（低于现有 0.70 warn）
    min_entries_between_folds: 15     # cooldown：至少 15 条新 entry 才允许再次 fold
    # 摘要模型标识符（provider-agnostic，留空则 fallback 到当前 session 模型）
    fold_model: null
```

- `fold_model: null` 时 fallback 到当前 session 使用的模型（避免硬编码 provider-specific 名称）
- 摘要 `max_tokens` 动态计算：`(source_tokens / 10).clamp(256, 2048)`，不再硬编码 800 字

### P1: Branch-Return（子任务级折叠）

依赖 #339 的 spawn_child 基础。

#### FoldBranchTool

```rust
// crates/kernel/src/tool/builtin/fold_branch.rs

/// 将子任务分支到独立上下文中执行，结果压缩后返回。
///
/// 用途：当子任务会产生大量中间上下文（如分析多个文件、
/// 搜索+汇总等），用 fold_branch 避免膨胀父上下文。
pub struct FoldBranchTool;

#[derive(Deserialize)]
pub struct FoldBranchArgs {
    /// 子任务描述（作为子 agent 的 system prompt 补充）
    pub task: String,
    /// 具体指令（作为子 agent 的 user message）
    pub instruction: String,
    /// 子 agent 可用的 tool 列表（可选，默认继承父）
    pub tools: Option<Vec<String>>,
    /// 最大迭代数（可选，默认 10）
    pub max_iterations: Option<u32>,
    /// 超时秒数（可选，默认从配置读取）
    pub timeout_secs: Option<u64>,
}

impl BuiltinTool for FoldBranchTool {
    fn name(&self) -> &str { "fold_branch" }

    fn description(&self) -> &str {
        "将子任务分支到独立上下文中执行。子 agent 有干净的上下文窗口，\
         不受父对话历史影响。执行完成后，结果自动压缩返回。\
         适用于：分析大量文件、搜索汇总、复杂推理等会产生大量中间上下文的任务。"
    }

    async fn execute(&self, ctx: &ToolContext, args: Value) -> Result<ToolResult> {
        let args: FoldBranchArgs = serde_json::from_value(args)?;
        let kernel = ctx.kernel_handle()?;

        let manifest = AgentManifest {
            name: format!("fold-branch-{}", Uuid::new_v4().as_simple()),
            system_prompt: Some(format!(
                "你是一个专注的子任务执行者。\n\n## 任务\n{}\n\n\
                 完成后用简洁的结构化格式输出结果，不需要过程叙述。",
                args.task
            )),
            tools: args.tools.unwrap_or_else(|| ctx.available_tools()),
            max_iterations: args.max_iterations.unwrap_or(10),
            ..AgentManifest::ephemeral()
        };

        // 同步等待，区别于 spawn_background
        let handle = kernel.spawn_child(
            &ctx.session_key,
            &ctx.principal,
            manifest,
            args.instruction,
        ).await?;

        let timeout_secs = args.timeout_secs
            .unwrap_or(ctx.config.context_folding.branch_timeout_secs); // 默认 120
        let timeout = Duration::from_secs(timeout_secs);

        let result = tokio::time::timeout(timeout, async {
            let mut final_text = String::new();
            while let Some(event) = handle.result_rx.recv().await {
                if let AgentEvent::FinalText(text) = event {
                    final_text = text;
                }
            }
            final_text
        }).await
        .map_err(|_| Error::BranchTimeout { timeout_secs })?;

        // 结果过长时压缩
        let compressed = if result.len() > COMPACT_TARGET_CHARS {
            ctx.context_folder.fold_text(&result, COMPACT_TARGET_CHARS).await?
        } else {
            result
        };

        Ok(ToolResult::text(compressed))
    }
}
```

#### 资源隔离：与 Background Agent 的 Semaphore 竞争

fold_branch 和 spawn_background 共享 `child_semaphore`。如果多个 fold_branch 同时触发，可能占满 semaphore 导致 background agent 无法 spawn。

方案：在 `child_semaphore` 中为 background agent 保留 slot。

```rust
// Session 上新增配置
pub struct ChildSlotConfig {
    /// 总 child 并发数（默认 8）
    pub total: usize,
    /// 为 background agent 保留的 slot 数（默认 2）
    pub reserved_background: usize,
}

// fold_branch spawn 前检查
let available = session.child_semaphore.available_permits();
let reserved = session.child_slot_config.reserved_background;
if available <= reserved {
    return Err(Error::NoChildSlotAvailable {
        reason: "remaining slots reserved for background agents",
    });
}
```

#### 与 Background Agent 的关系

```
                    ┌─────────────────────────────────────┐
                    │          Child Agent Spawning        │
                    │          (KernelHandle::spawn_child) │
                    └──────────┬──────────┬───────────────┘
                               │          │
                    ┌──────────▼──┐  ┌────▼──────────────┐
                    │ fold_branch │  │ spawn_background   │
                    │ (P1, #341)  │  │ (#339)             │
                    ├─────────────┤  ├────────────────────┤
                    │ 同步等待     │  │ 异步 fire-and-forget│
                    │ 结果内联返回  │  │ 结果触发 proactive  │
                    │ 压缩为       │  │ turn 推送给用户     │
                    │ ToolResult   │  │                    │
                    │ 目的：上下文  │  │ 目的：长任务        │
                    │ 管理         │  │ 不阻塞用户          │
                    │ 用 general   │  │ 用 reserved         │
                    │ slots        │  │ slots               │
                    └─────────────┘  └────────────────────┘
```

### P2: Hierarchical Summarization（层级摘要）

P0 的 `fold_with_prior` 已经内置了层级摘要能力。当创建新 anchor 时，前一个 anchor 的摘要会作为上下文传入，LLM 自然会生成累积摘要。

```
Anchor-0 (session 开始)
  → 15 轮对话
Anchor-1 { summary: "用户要做 X，已完成 A 和 B" }
  → 20 轮对话
Anchor-2 { summary: "项目背景：做 X（A、B 已完成）。本阶段完成了 C，遇到问题 D" }
  → 30 轮对话
Anchor-3 { summary: "做 X：A→B→C 完成，D 问题已解决。当前在做 E" }
```

随着 anchor 链增长，摘要自然越来越精炼——早期细节被压缩，关键决策保留。

#### 信息衰减与历史回溯

多次递归压缩会导致早期重要信息丢失。缓解措施：

- **Anchor chain 已可查询**：`TapeService::anchors(tape_name, limit)` 已支持获取所有历史 anchor。当用户明确需要历史回溯时，可拉取多个 anchor 的摘要而非只依赖最新累积版本。
- **Tape search 不受 fold 影响**：tape 本身永不截断，全文搜索仍可找到任何历史内容。
- 未来考虑：如果衰减严重，可在 fold prompt 中要求保留 "关键实体列表"（用户名、文件路径、决策 ID）作为 anchor extra 字段，独立于摘要文本。

## 不变量

1. **Tape 永不截断** — fold 只影响 LLM 看到的 messages，tape 完整保留
2. **Fold 用独立 LLM 调用** — 不走主 agent loop，不递归
3. **Anchor 是唯一的折叠载体** — 复用 `HandoffState` + `TapeService::handoff()`，不引入新的持久化结构
4. **fold_branch 是同步的** — 与 spawn_background 互补，不是替代
5. **可关闭** — `context_folding.enabled: false` 退回原有行为
6. **User tape 不受 fold 影响** — fold 只作用于 session tape，user tape notes 的注入逻辑不变
7. **TapEvent 向后兼容** — `ContextFolded` 事件存储在 `TapEntryKind::Event` 的 payload JSON 中，不新增 TapEntryKind variant，现有反序列化不受影响

## Metrics

需要追踪的指标（通过 `TapEntryKind::Event` payload 中的 `context_folded` 事件）：

| 指标 | 用途 |
|------|------|
| fold_count_per_session | 每个 session 触发了多少次 fold |
| fold_pressure_at_trigger | 触发 fold 时的上下文压力值 |
| fold_source_tokens | 被压缩的原内容 token 数 |
| fold_summary_tokens | 每次摘要消耗的 token |
| fold_model_latency_ms | 摘要 LLM 调用延迟 |
| branch_count_per_turn | 每个 turn 使用了多少次 fold_branch |
| branch_child_iterations | 子 agent 执行了多少次迭代 |

## 风险

| 风险 | 缓解 |
|------|------|
| 摘要丢失关键信息 | prompt 要求保留事实/决策/代码状态；动态 max_tokens（原内容 10%，上限 2048）；anchor chain 可回溯历史 |
| fold 调用增加延迟 | 用小模型；只在压力超阈值 + cooldown 满足时触发 |
| fold 震荡 | min_entries_between_folds cooldown（默认 15 条 entry） |
| fold LLM 调用失败 | 跳过 fold，现有 0.70/0.85 机制作为 fallback 继续生效 |
| fold_branch 超时 | 可配置超时（默认 120s）+ max_iterations 限制 |
| fold_branch 占满 child semaphore | 为 background agent 保留 slot |
| 无限递归 fold | fold 用独立 LLM 调用，不走 agent loop |
| 摘要质量不稳定 | temperature=0 + 结构化输出（JSON） |
| TapEvent 反序列化不兼容 | 用 Event payload JSON 字段，不新增 TapEntryKind variant |

## 实现顺序

```
P0 Auto-Anchor:
  1. agent/fold.rs — ContextFolder struct（kernel 层，非 memory 模块）
  2. agent.rs — 压力检测 + cooldown + 自动 fold 逻辑
     - 复用 classify_context_pressure + tape.info()
     - 复用 TapeService::handoff() + HandoffState
  3. config — context_folding 配置项（enabled, fold_threshold, min_entries_between_folds, fold_model）
  4. 测试：构造长对话 tape，验证 fold 触发 + anchor 创建 + cooldown 防震荡

P1 Branch-Return (依赖 #339 完成后):
  1. tool/builtin/fold_branch.rs — FoldBranchTool
  2. ToolContext 加 kernel_handle（#339 已计划）
  3. child semaphore slot 保留机制
  4. 测试：fold_branch 子任务执行 + 结果压缩 + semaphore 竞争

P2 Hierarchical Summarization:
  1. fold_with_prior 已在 P0 实现
  2. 验证：100+ 轮对话中摘要链的连贯性
  3. 可选：anchor extra 中保留关键实体列表防衰减
```
