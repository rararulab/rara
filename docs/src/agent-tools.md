# Agent Tools

The agent tool system is organized into two layers registered in the `ToolRegistry`:

- **Layer 1 — Primitives**: low-level database, notification, and storage operations.
- **Layer 2 — Services**: higher-level tools built on top of primitives (memory, recall strategy, resume, job pipeline, typst).

Tools are wired in `AppState::init` (`crates/workers/src/worker_state.rs`) and made available to the `AgentRunner` during chat.

## Primitive Tools

| Tool | Description |
|------|-------------|
| `db_query` | Read-only SQL query against whitelisted tables |
| `db_mutate` | INSERT/UPDATE/DELETE against whitelisted tables |
| `notify` | Send Telegram notifications |
| `storage_read` | Read files from object storage (S3/local) |

## Service Tools

### Memory Tools

| Tool | Description |
|------|-------------|
| `memory_search` | Hybrid search across mem0 + Hindsight (RRF fusion) |
| `memory_deep_recall` | Deep reasoning via Hindsight's 4-network reflect |
| `memory_write` | Write a Markdown note with tags to Memos |
| `memory_add_fact` | Store an explicit fact in mem0 + Hindsight |

### Recall Strategy Tools

Agents can configure their own memory recall rules at runtime. See [Memory System](./memory-system.md) for the full architecture.

| Tool | Description |
|------|-------------|
| `recall_strategy_add` | Register a new recall rule (trigger + action + inject target) |
| `recall_strategy_list` | List all rules with enabled/disabled status |
| `recall_strategy_update` | Update a rule (enable/disable, modify trigger/action/priority) |
| `recall_strategy_remove` | Delete a rule by ID |

### Resume & Job Tools

| Tool | Description |
|------|-------------|
| `list_resumes` | List all resumes |
| `get_resume_content` | Get resume text content |
| `analyze_resume` | AI-powered resume analysis against a job description |
| `job_pipeline` | Create and manage job applications |

### Typst Tools

| Tool | Description |
|------|-------------|
| `list_typst_projects` | List Typst document projects |
| `list_typst_files` | List files in a Typst project |
| `read_typst_file` | Read a Typst source file |
| `update_typst_file` | Update a Typst source file |
| `compile_typst_project` | Compile a Typst project to PDF |

## Tool Registration

Tools are registered in `AppState::init`:

```rust
// Layer 1: Primitives
tool_registry.register_primitive(Arc::new(DbQueryTool::new(pool.clone())));
tool_registry.register_primitive(Arc::new(DbMutateTool::new(pool.clone())));
tool_registry.register_primitive(Arc::new(NotifyTool::new(notify_client, settings_svc)));
tool_registry.register_primitive(Arc::new(StorageReadTool::new(object_store)));

// Layer 2: Memory + Recall Strategy
tool_registry.register_service(Arc::new(MemorySearchTool::new(mm.clone())));
tool_registry.register_service(Arc::new(MemoryDeepRecallTool::new(mm.clone())));
tool_registry.register_service(Arc::new(MemoryWriteTool::new(mm.clone())));
tool_registry.register_service(Arc::new(MemoryAddFactTool::new(mm.clone())));
tool_registry.register_service(Arc::new(RecallStrategyAddTool::new(engine.clone())));
tool_registry.register_service(Arc::new(RecallStrategyListTool::new(engine.clone())));
tool_registry.register_service(Arc::new(RecallStrategyUpdateTool::new(engine.clone())));
tool_registry.register_service(Arc::new(RecallStrategyRemoveTool::new(engine.clone())));
```

## Relevant Files

- `crates/workers/src/worker_state.rs` — tool registration and `AppState`
- `crates/workers/src/tools/primitives/` — primitive tool implementations
- `crates/workers/src/tools/services/` — service tool implementations
- `crates/workers/src/tools/services/memory_tools.rs` — memory + recall strategy tools
- `crates/agents/src/tool_registry.rs` — `ToolRegistry` and `AgentTool` trait
