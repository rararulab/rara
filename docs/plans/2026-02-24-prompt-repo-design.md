# Prompt Repository Redesign

## Problem

Prompt 管理分散在 3 个层次：

1. **源码 `prompts/` 目录** — 12 个 `.md` 文件，通过 13 处 `include_str!()` 编译嵌入到不同 crate
2. **`rara_paths::load_prompt_markdown()`** — 运行时从 `~/.config/job/prompts/` 读文件，fallback 到嵌入默认值
3. **14 个消费者各自加载** — 每个 task agent、orchestrator、settings router 都重复写 `const DEFAULT = include_str!(...)` + 调用 `load_prompt_markdown()`

问题：
- 同一个 prompt 的 `include_str!()` 路径散落在 8+ 个文件中
- Soul prompt 的 resolve 逻辑在 `orchestrator/prompt.rs` 和 `tasks/mod.rs` 各写了一遍
- `compose_system_prompt()` 在 `orchestrator/prompt.rs` 和 `tasks/prompt.rs` 各实现了一遍
- Pipeline agent 完全独立于体系之外（直接 `include_str!("prompt.md")`）
- 没有缓存，每次请求都读文件系统
- settings router 里的 `PROMPT_SPECS` 硬编码了 12 个 prompt 的注册信息

## Design

### 两层架构

```
Layer 0 (Core):      crates/core/rara-prompt/
                     ├── PromptRepo trait        (异步接口)
                     ├── FilePromptRepo          (文件系统实现 + cache + fs notify)
                     ├── PromptEntry             (数据类型)
                     └── compose                 (soul 组合逻辑)

Layer 3 (Extension): crates/extensions/prompt-admin/
                     └── HTTP CRUD routes        (list / get / update / reset)
```

### 1. `rara-prompt` crate (core 层)

#### 1.1 数据类型

```rust
/// 一条 prompt 的元信息 + 内容。
#[derive(Debug, Clone)]
pub struct PromptEntry {
    /// 唯一标识，如 "ai/job_fit.system.md"
    pub name: String,
    /// 人类可读描述
    pub description: String,
    /// 当前生效内容（用户编辑版 或 默认版）
    pub content: String,
}
```

#### 1.2 PromptRepo trait

```rust
#[async_trait::async_trait]
pub trait PromptRepo: Send + Sync + 'static {
    /// 获取单个 prompt，返回 None 表示未注册
    async fn get(&self, name: &str) -> Option<PromptEntry>;

    /// 列出所有已注册 prompt
    async fn list(&self) -> Vec<PromptEntry>;

    /// 更新 prompt 内容（写入文件系统 + 刷新 cache）
    /// 空内容 = 重置为默认
    async fn update(&self, name: &str, content: &str) -> Result<PromptEntry, PromptError>;

    /// 重置为默认内容
    async fn reset(&self, name: &str) -> Result<PromptEntry, PromptError>;
}
```

#### 1.3 FilePromptRepo 实现

```rust
pub struct FilePromptRepo {
    /// prompt_dir: ~/.config/rara/prompts/
    prompt_dir: PathBuf,
    /// 注册表：name -> PromptSpec { description, default_content }
    registry: HashMap<String, PromptSpec>,
    /// 内存缓存：name -> PromptEntry
    cache: Arc<RwLock<HashMap<String, PromptEntry>>>,
    /// fs watcher handle（后台任务）
    _watcher: notify::RecommendedWatcher,
}
```

**初始化流程：**
1. 接收 `Vec<PromptSpec>` 注册列表（name + description + default_content）
2. 对每个注册项，检查文件是否存在于 `prompt_dir/{name}`
3. **不存在** → 创建文件写入 `default_content`
4. **存在** → 读取文件内容
5. 全部加载到 `cache`
6. 启动 `notify::RecommendedWatcher` 监听 `prompt_dir`，文件变更时刷新对应 cache 条目

**get/list：** 直接读 cache（零 IO）

**update：** 写文件 → watcher 触发 → cache 自动刷新（或直接手动刷新 cache 不等 watcher）

#### 1.4 Prompt 注册

所有 prompt 在一个地方注册（`rara-prompt/src/builtin.rs`）：

```rust
use crate::PromptSpec;

pub fn all_builtin_prompts() -> Vec<PromptSpec> {
    vec![
        PromptSpec {
            name:            "agent/soul.md",
            description:     "Global personality / soul prompt",
            default_content: include_str!("../../../prompts/agent/soul.md"),
        },
        PromptSpec {
            name:            "chat/default_system.md",
            description:     "Default chat system prompt",
            default_content: include_str!("../../../prompts/chat/default_system.md"),
        },
        // ... 其余 10 个
        PromptSpec {
            name:            "pipeline/pipeline.md",
            description:     "Job pipeline agent system prompt",
            default_content: include_str!("../../../prompts/pipeline/pipeline.md"),
        },
    ]
}
```

**关键变化：** 所有 `include_str!()` 集中到这一个文件。

#### 1.5 Compose 逻辑统一

```rust
// rara-prompt/src/compose.rs

/// 通用 prompt 组合：soul + base
pub fn compose_with_soul(base: &str, soul: Option<&str>, section_title: &str) -> String {
    if let Some(soul) = soul.filter(|s| !s.trim().is_empty()) {
        if base.contains(soul.trim()) {
            return base.to_owned();
        }
        return format!("{soul}\n\n# {section_title}\n{base}");
    }
    base.to_owned()
}
```

消费者调用：
```rust
// Task agent
let base = prompt_repo.get("ai/job_fit.system.md").await?.content;
let soul = prompt_repo.get("agent/soul.md").await.map(|e| e.content);
let prompt = compose_with_soul(&base, soul.as_deref(), "Task Instructions");

// Chat orchestrator
let base = prompt_repo.get("chat/default_system.md").await?.content;
let soul = prompt_repo.get("agent/soul.md").await.map(|e| e.content);
let prompt = compose_with_soul(&base, soul.as_deref(), "Chat Instructions");
```

#### 1.6 build.rs

在 `rara-cmd`（最终二进制 crate）的 build.rs 中：

```rust
fn main() {
    // 现有 shadow-rs
    shadow_rs::ShadowBuilder::builder().build().unwrap();

    // 持久化内置 prompt 到配置目录
    // 仅在 prompt 文件不存在时写入（不覆盖用户编辑）
    let prompt_dir = dirs::config_dir().unwrap().join("rara/prompts");
    let prompts_src = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../prompts");

    for entry in walkdir::WalkDir::new(&prompts_src)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map_or(false, |ext| ext == "md"))
    {
        let rel = entry.path().strip_prefix(&prompts_src).unwrap();
        let target = prompt_dir.join(rel);
        // 总是覆盖 — 确保磁盘上的文件与源码保持同步
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        std::fs::copy(entry.path(), &target).ok();
    }
}
```

> **注意：** build.rs 在编译时运行，**总是覆盖**磁盘上的文件以确保与源码同步。用户运行时的自定义编辑通过 admin API 写入后会在下次编译时被重置——这是有意为之，源码中的 prompt 是 single source of truth。生产部署时，Docker 入口或 init 逻辑做同样的事。

### 2. `prompt-admin` crate (extension 层)

```rust
// crates/extensions/prompt-admin/src/router.rs

pub fn routes(repo: Arc<dyn PromptRepo>) -> OpenApiRouter {
    OpenApiRouter::new().nest(
        "/api/v1",
        OpenApiRouter::new()
            .route("/prompts", get(list_prompts))
            .route("/prompts/{*name}", get(get_prompt).put(update_prompt).delete(reset_prompt))
    ).with_state(repo)
}
```

路由：
- `GET /api/v1/prompts` — 列出所有 prompt（name + description + content）
- `GET /api/v1/prompts/{name}` — 获取单个
- `PUT /api/v1/prompts/{name}` — 更新内容
- `DELETE /api/v1/prompts/{name}` — 重置为默认

### 3. 迁移策略

#### 阶段 1：创建 rara-prompt + prompt-admin crate
- 实现 `PromptRepo` trait + `FilePromptRepo`
- 注册所有 12+1 个 prompt
- 实现 admin routes

#### 阶段 2：改造消费者
- `TaskAgentService` 接收 `Arc<dyn PromptRepo>`，不再持有 soul_prompt
- 8 个 task agent 的 `analyze/optimize/...` 方法改为从 repo 获取 prompt
- `AgentOrchestrator` 接收 `Arc<dyn PromptRepo>`，替换现有 `load_prompt_markdown` 调用
- `PipelineService` 从 repo 获取 `pipeline/pipeline.md`
- 移动 `pipeline/src/prompt.md` → `prompts/pipeline/pipeline.md`

#### 阶段 3：清理
- 删除 `rara_paths` 中的 `load_prompt_markdown`、`load_agent_soul_prompt` 等函数
- 删除 `settings/router.rs` 中的 `PROMPT_SPECS` 和 prompt 相关路由
- 删除 `orchestrator/prompt.rs` 中的 `resolve_soul_prompt`
- 删除 `builtin/tasks/prompt.rs` 和 `tasks/mod.rs` 中的 `current_soul_prompt`
- 删除各 task agent 中的 `include_str!()` 常量

### 4. 依赖关系

```
rara-prompt (core)
  ├── notify (fs watcher)
  ├── tokio
  ├── async-trait
  ├── snafu
  └── tracing

prompt-admin (extension)
  ├── rara-prompt
  ├── axum
  ├── serde
  └── utoipa-axum

rara-agents
  └── rara-prompt  (不再需要 rara-paths 的 prompt 相关函数)

job-pipeline
  └── rara-prompt  (不再需要 include_str! prompt)

rara-app (composition root)
  ├── rara-prompt      (构造 FilePromptRepo)
  ├── prompt-admin     (挂载 admin routes)
  ├── rara-agents      (注入 PromptRepo)
  └── job-pipeline     (注入 PromptRepo)
```

### 5. 文件系统布局

```
~/.config/rara/prompts/
├── agent/
│   └── soul.md
├── ai/
│   ├── cover_letter.system.md
│   ├── follow_up.system.md
│   ├── interview_prep.system.md
│   ├── jd_analyzer.system.md
│   ├── jd_parser.system.md
│   ├── job_fit.system.md
│   ├── resume_analyzer.system.md
│   └── resume_optimizer.system.md
├── chat/
│   └── default_system.md
├── pipeline/
│   └── pipeline.md
└── workers/
    ├── agent_policy.md
    └── resume_analysis_instructions.md
```

### 6. Settings 中 soul prompt 的处理

当前 `Settings.agent.soul` 字段允许通过 settings JSON 覆盖 soul prompt。改造后：

- **保留** settings 中的 soul 覆盖能力
- `resolve_soul` 逻辑移入 `rara-prompt` 的辅助函数
- 优先级：`Settings.agent.soul` > `PromptRepo.get("agent/soul.md")`

```rust
// rara-prompt/src/compose.rs
pub async fn resolve_soul(
    repo: &dyn PromptRepo,
    settings_soul: Option<&str>,
) -> Option<String> {
    if let Some(soul) = settings_soul.filter(|s| !s.trim().is_empty()) {
        return Some(soul.to_owned());
    }
    repo.get("agent/soul.md").await.map(|e| e.content)
}
```
