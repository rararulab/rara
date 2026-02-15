# Agent Tools

The agent tool system is organized into two layers registered in the `ToolRegistry`:

- **Layer 1 — Primitives**: low-level database, notification, and storage operations.
- **Layer 2 — Services**: higher-level tools built on top of primitives (memory, resume, job pipeline, typst).

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
| `memory_search` | Hybrid search (PG keyword + Chroma vector) across indexed markdown files |
| `memory_get` | Fetch full chunk content by `chunk_id` |
| `memory_write` | Write markdown to `memory_dir/`, triggers immediate sync |
| `memory_update_profile` | Update a section of the persistent user profile |

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

// Layer 2: Services
tool_registry.register_service(Arc::new(MemorySearchTool::new(memory_manager)));
tool_registry.register_service(Arc::new(MemoryWriteTool::new(memory_manager)));
tool_registry.register_service(Arc::new(MemoryUpdateProfileTool::new(memory_manager)));
// ... and more
```

## Relevant Files

- `crates/workers/src/worker_state.rs` — tool registration and `AppState`
- `crates/workers/src/tools/primitives/` — primitive tool implementations
- `crates/workers/src/tools/services/` — service tool implementations
- `crates/agents/src/tool_registry.rs` — `ToolRegistry` and `AgentTool` trait
