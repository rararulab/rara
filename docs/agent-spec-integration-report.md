# agent-spec 集成方案报告

> RAR-6 | Issue #6: 调研 agent-spec 并给出 Rara 集成方案报告

## 1. agent-spec 核心概念与架构

### 1.1 定位

[agent-spec](https://zhanghandong.github.io/agent-spec/) 是一个 **AI-native BDD/spec 验证工具**，核心理念是将代码审查从「人读 diff」转变为「人定义意图、机器验证代码」。

### 1.2 核心概念：Task Contract（任务契约）

一个 Task Contract 由四要素组成：

| 要素 | 英文关键字 | 作用 |
|------|-----------|------|
| **意图** | `## Intent` | 说明做什么、为什么做 |
| **已定决策** | `## Decisions` | 已确定的技术选择，Agent 不得质疑 |
| **边界** | `## Boundaries` | 允许修改的文件 glob + 禁止事项 |
| **完成条件** | `## Completion Criteria` | BDD 场景 + `Test:` 选择器，定义确定性 pass/fail |

DSL 支持英文、中文、日文三语关键字。

### 1.3 七步工作流（三个角色）

```
Human          Machine              Agent
  │               │                   │
  ├─1. Write ─────┤                   │   agent-spec init
  │               ├─2. Lint ──────────┤   agent-spec lint (质量门)
  │               │                   ├─3. Implement   agent-spec contract (读取约束)
  │               ├─4. Lifecycle ─────┤   agent-spec lifecycle (4层验证)
  │               ├─5. Guard ─────────┤   agent-spec guard (pre-commit/CI)
  ├─6. Accept ────┤                   │   agent-spec explain (合约级摘要)
  │               ├─7. Stamp ─────────┤   agent-spec stamp (Git trailers 追溯)
```

### 1.4 Lifecycle 四层验证管线

1. **Lint** — 合约质量检查（模糊动词、未量化约束、讨好偏见检测）
2. **Structural** — 结构性验证
3. **Boundaries** — 路径 glob 机械执行（修改了禁止文件？）
4. **Tests** — 运行 `Test:` 选择器绑定的测试

### 1.5 AI 验证器（实验性）

两种模式共享 `AiRequest` / `AiDecision` 数据结构：

| 模式 | 标志 | 适用场景 |
|------|------|---------|
| **Caller** | `--ai-mode caller` | Agent 自己充当 AI 验证器（两步协议：emit requests → resolve decisions） |
| **Backend** | Rust API: `AiBackend` trait | 编排器注入独立 AI 后端（如 Symphony 使用不同模型验证） |

### 1.6 技术栈

- **语言**: Rust（`cargo install agent-spec`）
- **集成**: 通过 Skills 注入 AI Agent 配置（Claude Code、Codex、Cursor、Aider）
- **安装**: `npx skills add ZhangHanDong/agent-spec`

---

## 2. 与 Rara 当前架构的映射

### 2.1 Rara 架构概览

Rara 是 Rust workspace，采用 kernel-inspired 架构：

| Rara 组件 | Crate | 职责 |
|-----------|-------|------|
| **Kernel** | `rara-kernel` | OS 风格编排器：SessionTable、EventQueue、SyscallDispatcher |
| **Agents** | `rara-agents` | 预定义 Agent manifests（rara/nana/worker/mita） |
| **Skills** | `rara-skills` | SKILL.md 发现、解析、安装、提示注入 |
| **Sessions** | `rara-sessions` | 文件基础会话元数据索引 |
| **Symphony** | `rara-symphony` | 多仓库工作空间编排、issue tracker 集成 |
| **MCP** | `rara-mcp` | MCP 工具桥接到 ToolRegistry |
| **Model** | `rara-model` | SQLx 数据模型 + 迁移 |
| **Tools** | Kernel ToolRegistry | AgentTool trait + DynamicToolProvider |

### 2.2 概念映射表

| agent-spec 概念 | Rara 对应 | 映射关系 |
|-----------------|----------|---------|
| Task Contract (.spec 文件) | Issue → AgentManifest | agent-spec 提供**合约层**，补充 Rara 的 issue-driven 工作流 |
| `agent-spec` CLI | AgentTool (ToolRegistry) | 作为外部工具注册到 Kernel |
| Contract DSL 解析 | rara-skills (parse) | Skills 已有 frontmatter 解析能力，可复用 |
| Lifecycle 验证 | Kernel event loop | Lifecycle 结果可作为 KernelEvent 注入 |
| Boundaries 路径约束 | Guard trait | 可扩展 Guard 子系统实现边界执行 |
| AI Verifier (Backend mode) | rara-kernel LlmDriver | Kernel 已有 LLM 能力，可实现 AiBackend |
| `explain` 输出 | NotificationBus | 验证摘要通过 channels 推送 |
| Skills (agent-spec-tool-first) | rara-skills | 直接安装为 Rara skill |
| Seven-step workflow | Symphony issue lifecycle | Symphony 的 issue 状态推进可嵌入 7 步流程 |

### 2.3 天然契合点

1. **都是 Rust** — agent-spec 是 Rust crate，Rara 是 Rust workspace，可直接作为库依赖
2. **都面向 Agent** — Rara 编排 Agent 执行任务，agent-spec 定义 Agent 的任务约束
3. **Skills 系统兼容** — Rara 已有 SKILL.md 体系，agent-spec 的 Skills 可直接安装
4. **Symphony 编排** — Symphony 管理 per-issue 子进程，agent-spec 的 lifecycle 验证是天然的验证层

---

## 3. 可行的集成方式与实现路径

### 方案 A：CLI 工具集成（推荐 MVP）

**方式**: 将 `agent-spec` CLI 作为外部工具注册到 Kernel ToolRegistry。

```
Symphony 创建 issue
  → ralph run 子进程启动
    → Agent 读取 .spec 合约 (agent-spec contract)
    → Agent 在边界内实现
    → agent-spec lifecycle 验证
    → 验证通过 → 提交
    → agent-spec explain → PR 描述
```

**优势**: 零侵入，利用现有 ToolRegistry 机制
**劣势**: 进程间通信开销，JSON 序列化边界

### 方案 B：Rust 库集成

**方式**: 将 `agent-spec` 作为 Rust crate 依赖，直接调用其核心 API。

```toml
# Cargo.toml
[workspace.dependencies]
agent-spec = "0.x"
```

创建 `crates/integrations/agent-spec/` 桥接 crate：
- 实现 `AiBackend` trait，桥接到 Kernel 的 `LlmDriver`
- 将 Lifecycle 结果转换为 `KernelEvent`
- 将 Boundaries 约束注入 `Guard` 子系统

**优势**: 类型安全，深度集成，零进程开销
**劣势**: 版本耦合，需跟踪 agent-spec API 变更

### 方案 C：混合方案（推荐长期）

**MVP 阶段**: 方案 A（CLI 工具）
**成熟阶段**: 方案 B（库集成）用于核心路径，CLI 保留用于 CI/CD

---

## 4. 需要修改或新增的模块

### 4.1 MVP（方案 A）需要的变更

| 模块 | 变更类型 | 内容 |
|------|---------|------|
| `rara-skills` | **新增 skill** | 安装 `agent-spec-tool-first` skill |
| Kernel ToolRegistry | **注册工具** | 添加 `agent-spec` CLI 系列工具（init/lint/lifecycle/guard/explain） |
| `rara-symphony` | **扩展流程** | issue 创建时自动生成 .spec 合约模板；验证步骤嵌入 issue lifecycle |
| `.claude/skills/` | **新增** | 安装 agent-spec skills 到项目配置 |

### 4.2 长期集成（方案 B）需要的新模块

| 新模块 | 内容 |
|--------|------|
| `crates/integrations/agent-spec/` | 新 crate：AiBackend 实现、Lifecycle 事件桥接、Guard 扩展 |
| `rara-kernel` Guard 扩展 | 新增 `BoundariesGuard`：读取 .spec Boundaries，阻止越界文件修改 |
| `rara-kernel` Event 扩展 | 新增 `EventKind::SpecLifecycle`：验证结果事件 |
| `rara-symphony` Contract 管理 | issue → .spec 自动生成、spec 版本管理 |

---

## 5. 风险与权衡

### 5.1 风险矩阵

| 风险 | 严重性 | 可能性 | 缓解措施 |
|------|--------|--------|---------|
| agent-spec API 不稳定（v0.x） | 中 | 高 | MVP 使用 CLI 集成，隔离 API 变更 |
| 合约编写增加开发摩擦 | 中 | 中 | 仅对 Symphony 管理的 issue 启用；提供模板自动生成 |
| Lifecycle 验证误报 | 低 | 中 | 使用 `--ai-mode off` 仅机械验证；逐步启用 AI 验证 |
| 性能开销（CLI 子进程） | 低 | 低 | Rust 二进制启动快；后续可迁移到库集成 |
| 三语 DSL 增加复杂度 | 低 | 低 | 统一使用中文关键字（与 Rara 用户群一致） |

### 5.2 关键权衡

| 决策点 | 选项 A | 选项 B | 建议 |
|--------|--------|--------|------|
| 集成深度 | CLI 工具（松耦合） | Rust 库（紧耦合） | MVP 选 A，验证价值后迁移到 B |
| 合约范围 | 所有 issue | 仅 Symphony 管理的 issue | 仅 Symphony，降低摩擦 |
| AI 验证模式 | Caller（Agent 自验证） | Backend（Kernel LLM 验证） | Backend 更适合 Rara 架构 |
| 合约存储 | 项目 `specs/` 目录 | 数据库 (rara-model) | `specs/` 目录，与 git 协同 |

---

## 6. 最小可行实现（MVP）步骤

### Phase 1：Skills 安装与工具注册（1-2 天）

```bash
# 1. 安装 agent-spec CLI
cargo install agent-spec

# 2. 安装 agent-spec skills 到 Rara 项目
npx skills add ZhangHanDong/agent-spec
```

- [ ] 在 Rara 项目中安装 `agent-spec-tool-first` skill
- [ ] 验证 Agent（Claude Code/Codex）能识别并使用 agent-spec 工作流
- [ ] 编写一个示例 .spec 合约，手动验证完整 7 步流程

### Phase 2：Symphony 流程集成（2-3 天）

- [ ] 修改 `rara-symphony` 的 issue lifecycle：
  - issue 创建时，自动在 `specs/` 目录生成 `.spec` 合约模板
  - Agent 实现完成后，自动运行 `agent-spec lifecycle` 验证
  - 验证通过后，运行 `agent-spec explain` 生成 PR 描述
- [ ] 将 `agent-spec guard` 添加到 CI pipeline（pre-commit hook 或 GitHub Action）

### Phase 3：工具注册到 Kernel（1-2 天）

- [ ] 在 ToolRegistry 中注册 agent-spec CLI 命令集：
  - `agent_spec_init` — 创建合约
  - `agent_spec_lint` — 质量检查
  - `agent_spec_lifecycle` — 验证管线
  - `agent_spec_explain` — 生成审查摘要
  - `agent_spec_guard` — 仓库级验证
- [ ] Agent 在 kernel event loop 中可直接调用这些工具

### Phase 4：验证与迭代（持续）

- [ ] 在 3-5 个真实 issue 上试用完整流程
- [ ] 收集反馈：合约编写效率、验证准确率、开发者体验
- [ ] 决定是否进入 Phase 5（Rust 库深度集成）

### Phase 5（可选）：深度集成

- [ ] 创建 `crates/integrations/agent-spec/` crate
- [ ] 实现 `AiBackend` trait，桥接 Kernel LlmDriver
- [ ] 扩展 Guard 子系统支持 Boundaries 约束
- [ ] 新增 `EventKind::SpecLifecycle` 事件类型

---

## 7. 结论

agent-spec 与 Rara 在理念和技术栈上高度契合：

1. **互补关系明确** — Rara 负责 Agent 编排与执行，agent-spec 负责任务约束与验证
2. **集成成本低** — 同为 Rust 生态，Skills 体系兼容，CLI 集成零侵入
3. **价值清晰** — 将代码审查从「读 500 行 diff」转变为「审 50 行合约」，特别适合 Symphony 管理的自动化 issue 流程
4. **渐进式采纳** — 从 CLI 工具集成开始，验证价值后再考虑深度库集成

**建议**：立即执行 Phase 1-2（Skills 安装 + Symphony 流程集成），在 3-5 个真实 issue 上验证 ROI，再决定是否推进深度集成。
