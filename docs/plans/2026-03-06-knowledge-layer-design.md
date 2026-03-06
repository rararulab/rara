# Knowledge Layer Design

> 借鉴 MemU 的三层记忆架构，在 Rara 内部实现结构化长期记忆。

## 动机

Rara 当前的 tape 系统是全量对话历史（append-only JSONL），检索靠 substring + fuzzy 匹配。
随着对话积累，tape 越来越长，缺乏语义级别的检索和知识沉淀能力。

借鉴 MemU 的设计理念，在 tape 之上增加 Knowledge Layer——自动从对话中提取结构化记忆，
支持 embedding 语义检索，并按主题聚合为人类可读的 category 文件。

## 三层架构

```
Layer 3: Memory Categories    ← 磁盘 markdown 文件 (profile.md, events.md...)
    ↕ item IDs 双向引用
Layer 2: Memory Items          ← SQLite 表 + usearch 向量索引
    ↕ source_tape + source_entry_id 回溯
Layer 1: Resources (Tape)      ← 现有 JSONL tape 文件（不改动）
```

- **Layer 1 (Tape)** — 全量历史，ground truth，只增不删，现有实现不改动
- **Layer 2 (Memory Items)** — tape 的语义提炼，加速检索的索引层
- **Layer 3 (Categories)** — items 的主题聚合，给 LLM 和人类快速理解用

三层保持双向可追溯：Category → Item IDs → Tape entry。

## 文件布局

```
~/.local/share/rara/
  memory/
    memory.usearch            # usearch 向量索引
    categories/
      ryan/
        profile.md
        preferences.md
        events.md
        ...                   # LLM 可动态创建新 category
  tapes/
    <session>.jsonl           # 现有 tape（Layer 1，不改动）
```

- SQLite `memory_items` 表存在现有数据库中（通过 sqlx migration 添加）
- usearch 索引文件独立存储
- category 文件按 username 目录隔离
- 路径在 knowledge 模块内部拼接，不暴露到 `rara_paths`

## 数据模型

### SQLite `memory_items` 表

```sql
CREATE TABLE memory_items (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    username        TEXT NOT NULL,
    content         TEXT NOT NULL,           -- 自然语言句子，如 "Ryan 偏好简洁回复"
    memory_type     TEXT NOT NULL,           -- preference / fact / event / habit / skill
    category        TEXT NOT NULL,           -- 对应的 category 文件名，如 "profile"
    source_tape     TEXT,                    -- 来源 tape 名，回溯到 Layer 1
    source_entry_id INTEGER,                 -- 来源 tape entry id
    embedding       BLOB,                   -- f32 向量，1536 维 × 4 bytes = 6KB
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);

CREATE INDEX idx_items_username ON memory_items(username);
CREATE INDEX idx_items_category ON memory_items(username, category);
```

### Category markdown 文件格式

```markdown
# Profile

## 基本信息
- Ryan，开发者，主力语言 Rust
- 使用 macOS，编辑器偏好 Neovim

## 工作
- 正在开发 Rara，一个 proactive AI agent
- 偏好 kernel-inspired 架构设计

## 来源
- [item:3] [item:7] [item:12]
```

底部 `来源` 维护 item ID 引用，实现 Layer 3 → Layer 2 回溯。

## 写入流程（Memorize）

Session 结束时 emit `memory:extract` event，kernel event loop 异步处理：

```
Session 结束
  ↓
emit Event { name: "memory:extract", data: { tape_name, username } }
  ↓
Kernel EventBus 分发给 MemoryExtractor handler
  ↓
Step 1: 加载 tape entries（从上次提取的 anchor 之后）
  ↓
Step 2: LLM 提取 memory items
         Prompt: "从以下对话中提取关键事实、偏好、事件，每条用一句自然语言表达"
         输入: tape entries
         输出: Vec<{content, memory_type, category}>
  ↓
Step 3: 去重
         对每个新 item，embedding 搜索已有 items
         相似度 > 阈值 → 跳过或合并（更新 updated_at）
  ↓
Step 4: 持久化
         新 items → INSERT memory_items 表
         新 embeddings → OpenAI API 批量获取 → 存 SQLite blob + usearch 索引追加
  ↓
Step 5: LLM 更新 category 文件
         Prompt: "以下是新提取的 memory items，请更新对应的 category markdown 文件"
         输入: 新 items + 现有 category 文件内容
         输出: 更新后的 markdown（LLM 决定放哪个 category，也可创建新的）
  ↓
Step 6: 写入 category markdown 文件到磁盘
```

Step 2 和 Step 5 各需要一次 LLM 调用，使用便宜的模型（如 Haiku）。

## 读取流程（Retrieve）

### 场景 1：Session 开始 — Context 注入

```
新 session 开始，已知 username
  ↓
加载该用户所有 category 文件的摘要（文件名 + 前几行）
  ↓
作为 system message 注入（扩展现有 user_tape_context()）
```

### 场景 2：对话中 — 按需深度检索

暴露给 LLM 的 tool 接口：

- `memory_search(query)` — 语义搜索 memory items（query → embedding → usearch top-k → SQLite 过滤）
- `memory_categories()` — 列出当前用户所有 category 文件名和摘要
- `memory_read_category(name)` — 读取某个 category 文件全文

Agent 可组合使用：先 `memory_search` 快速定位，再 `memory_read_category` 深度理解。

## 模块结构

```
crates/kernel/src/memory/
  mod.rs              # 现有，加 re-export 新模块
  store.rs            # 现有 FileTapeStore（不改）
  service.rs          # 现有 TapeService（不改）
  context.rs          # 现有，扩展：注入 category 摘要
  anchors.rs          # 现有（不改）
  knowledge/          # 新模块
    mod.rs            # KnowledgeStore 组装
    items.rs          # SQLite memory_items CRUD
    categories.rs     # Category markdown 文件读写
    embedding.rs      # OpenAI embedding API + usearch 索引
    extractor.rs      # LLM 驱动的 memorize 流程
```

## 依赖

```toml
# crates/kernel/Cargo.toml 新增
usearch = "2"
```

- SQLite: 复用现有 sqlx 连接
- HTTP: 复用现有 reqwest（调 OpenAI embedding API）
- LLM: 复用 kernel 现有 LLM 基础设施

## 配置

```yaml
# config.yaml
memory:
  knowledge:
    enabled: true
    embedding_model: "text-embedding-3-small"
    embedding_dimensions: 1536
    search_top_k: 20
    similarity_threshold: 0.85
    extractor_model: "haiku"
```

所有配置项必须显式声明，不在代码中提供 fallback 默认值。

## 作用域

- **per-username** — 每个用户一套 memory items + categories
- 所有 agent 人格（Rara/Mita/Scout）共享同一份用户记忆
- 与现有 user tape 约定兼容

## 不改动的部分

- Tape 系统（FileTapeStore, TapeService, anchors）完全不改
- TapeTool 现有接口不改
- Session 管理不改
