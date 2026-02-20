# Tool-Core Primitives Consolidation

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Consolidate all 12 primitive tools into `tool-core` crate, exporting `default_primitives(PrimitiveDeps) -> Vec<AgentToolRef>`.

**Architecture:** Move 8 core primitives from `rara-agents` and 4 domain primitives from `rara-workers` into `tool-core`, organized as `core_primitives/` and `domain_primitives/` sub-modules. `tool-core` becomes the single source of truth for all primitive tool implementations. `ToolRegistry::with_defaults()` in agents is removed; callers use `tool_core::default_primitives()` instead.

**Tech Stack:** Rust, tool-core crate, sqlx (PgPool), opendal (Operator), rara-domain-shared (NotifyClient, SettingsSvc)

---

### Task 1: Add dependencies to tool-core Cargo.toml

**Files:**
- Modify: `crates/core/tool-core/Cargo.toml`

**Step 1: Update Cargo.toml with all required deps**

```toml
[package]
name = "tool-core"
version.workspace = true
edition.workspace = true
license.workspace = true

[lints]
workspace = true

[dependencies]
anyhow.workspace = true
async-trait.workspace = true
chrono.workspace = true
opendal.workspace = true
rara-domain-shared.workspace = true
reqwest.workspace = true
serde_json.workspace = true
sqlx.workspace = true
tokio.workspace = true
uuid.workspace = true
```

**Step 2: Verify it compiles**

Run: `cargo check -p tool-core`
Expected: PASS (no tool code yet, just deps)

**Step 3: Commit**

```bash
git add crates/core/tool-core/Cargo.toml
git commit -m "chore(tool-core): add deps for primitive tool consolidation"
```

---

### Task 2: Move core primitives into tool-core

Move these 8 files from `crates/agents/src/tools/primitives/` to `crates/core/tool-core/src/core_primitives/`:
- `bash.rs`
- `read_file.rs`
- `write_file.rs`
- `edit_file.rs`
- `find_files.rs`
- `grep.rs`
- `list_directory.rs`
- `http_fetch.rs`

**Files:**
- Create: `crates/core/tool-core/src/core_primitives/mod.rs`
- Create: `crates/core/tool-core/src/core_primitives/bash.rs` (move from agents)
- Create: `crates/core/tool-core/src/core_primitives/read_file.rs` (move)
- Create: `crates/core/tool-core/src/core_primitives/write_file.rs` (move)
- Create: `crates/core/tool-core/src/core_primitives/edit_file.rs` (move)
- Create: `crates/core/tool-core/src/core_primitives/find_files.rs` (move)
- Create: `crates/core/tool-core/src/core_primitives/grep.rs` (move)
- Create: `crates/core/tool-core/src/core_primitives/list_directory.rs` (move)
- Create: `crates/core/tool-core/src/core_primitives/http_fetch.rs` (move)
- Modify: `crates/core/tool-core/src/lib.rs`

**Step 1: Create core_primitives/mod.rs**

```rust
//! Core primitives: generic, business-logic-free atomic operations.

mod bash;
mod edit_file;
mod find_files;
mod grep;
mod http_fetch;
mod list_directory;
mod read_file;
mod write_file;

pub use bash::BashTool;
pub use edit_file::EditFileTool;
pub use find_files::FindFilesTool;
pub use grep::GrepTool;
pub use http_fetch::HttpFetchTool;
pub use list_directory::ListDirectoryTool;
pub use read_file::ReadFileTool;
pub use write_file::WriteFileTool;
```

**Step 2: Move each .rs file**

For every file, the only change is replacing the import:

```rust
// OLD (in agents):
use crate::tool_registry::AgentTool;

// NEW (in tool-core):
use crate::AgentTool;
```

No other code changes. The tool implementations are identical.

**Step 3: Register module in tool-core lib.rs**

```rust
pub mod core_primitives;
```

**Step 4: Verify**

Run: `cargo check -p tool-core`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/core/tool-core/src/core_primitives/
git add crates/core/tool-core/src/lib.rs
git commit -m "refactor(tool-core): move 8 core primitives from agents"
```

---

### Task 3: Move domain primitives into tool-core

Move these 4 files from `crates/workers/src/tools/primitives/` to `crates/core/tool-core/src/domain_primitives/`:
- `db_query.rs`
- `db_mutate.rs`
- `notify.rs`
- `storage_read.rs`

**Files:**
- Create: `crates/core/tool-core/src/domain_primitives/mod.rs`
- Create: `crates/core/tool-core/src/domain_primitives/db_query.rs` (move)
- Create: `crates/core/tool-core/src/domain_primitives/db_mutate.rs` (move)
- Create: `crates/core/tool-core/src/domain_primitives/notify.rs` (move)
- Create: `crates/core/tool-core/src/domain_primitives/storage_read.rs` (move)
- Modify: `crates/core/tool-core/src/lib.rs`

**Step 1: Create domain_primitives/mod.rs**

```rust
//! Domain primitives: application-specific atomic operations (db, notify, storage).

mod db_mutate;
mod db_query;
mod notify;
mod storage_read;

pub use db_mutate::DbMutateTool;
pub use db_query::DbQueryTool;
pub use notify::NotifyTool;
pub use storage_read::StorageReadTool;
```

**Step 2: Move each .rs file**

For every file, replace the import:

```rust
// OLD (in workers):
use rara_agents::tool_registry::AgentTool;

// NEW (in tool-core):
use crate::AgentTool;
```

Also replace error types if any reference `rara_agents::err::*`:

```rust
// OLD:
rara_agents::err::Error::Other { message: "...".into() }

// NEW:
anyhow::anyhow!("...")
```

Specifically for `db_query.rs`: the `execute` method uses `rara_agents::err::Result` and `rara_agents::err::Error::Other` — replace with `anyhow::Result` and `anyhow::anyhow!()`. Check lines 139-149.

**Step 3: Register module in tool-core lib.rs**

```rust
pub mod domain_primitives;
```

**Step 4: Verify**

Run: `cargo check -p tool-core`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/core/tool-core/src/domain_primitives/
git add crates/core/tool-core/src/lib.rs
git commit -m "refactor(tool-core): move 4 domain primitives from workers"
```

---

### Task 4: Add PrimitiveDeps and default_primitives()

**Files:**
- Modify: `crates/core/tool-core/src/lib.rs`

**Step 1: Add the registration API**

Append to `lib.rs`:

```rust
use std::sync::Arc;

/// Dependencies required to construct domain-level primitive tools.
pub struct PrimitiveDeps {
    pub pool:          sqlx::PgPool,
    pub notify_client: rara_domain_shared::notify::client::NotifyClient,
    pub settings_svc:  rara_domain_shared::settings::SettingsSvc,
    pub object_store:  opendal::Operator,
}

/// Returns all primitive tools (core + domain), ready for registration.
pub fn default_primitives(deps: PrimitiveDeps) -> Vec<AgentToolRef> {
    let mut tools = core_primitives();
    tools.extend(domain_primitives(deps));
    tools
}

/// Returns only the 8 core primitives (no application deps).
pub fn core_primitives() -> Vec<AgentToolRef> {
    vec![
        Arc::new(core_primitives::BashTool::new()),
        Arc::new(core_primitives::ReadFileTool::new()),
        Arc::new(core_primitives::WriteFileTool::new()),
        Arc::new(core_primitives::EditFileTool::new()),
        Arc::new(core_primitives::FindFilesTool::new()),
        Arc::new(core_primitives::GrepTool::new()),
        Arc::new(core_primitives::ListDirectoryTool::new()),
        Arc::new(core_primitives::HttpFetchTool::new()),
    ]
}

/// Returns only the 4 domain primitives.
pub fn domain_primitives(deps: PrimitiveDeps) -> Vec<AgentToolRef> {
    vec![
        Arc::new(domain_primitives::DbQueryTool::new(deps.pool.clone())),
        Arc::new(domain_primitives::DbMutateTool::new(deps.pool)),
        Arc::new(domain_primitives::NotifyTool::new(deps.notify_client, deps.settings_svc)),
        Arc::new(domain_primitives::StorageReadTool::new(deps.object_store)),
    ]
}
```

**Step 2: Verify**

Run: `cargo check -p tool-core`
Expected: PASS

**Step 3: Commit**

```bash
git add crates/core/tool-core/src/lib.rs
git commit -m "feat(tool-core): add PrimitiveDeps and default_primitives() API"
```

---

### Task 5: Update rara-agents — remove primitives, remove with_defaults()

**Files:**
- Delete: `crates/agents/src/tools/primitives/` (entire directory)
- Modify: `crates/agents/src/tools/mod.rs` — remove `pub mod primitives;`
- Modify: `crates/agents/src/tool_registry.rs` — remove `with_defaults()` impl block
- Modify: `crates/agents/Cargo.toml` — remove `reqwest` dep (only used by HttpFetchTool, no longer needed)

**Step 1: Delete primitives directory**

```bash
rm -rf crates/agents/src/tools/primitives/
```

**Step 2: Update agents/src/tools/mod.rs**

If the file only contained `pub mod primitives;`, it can be removed entirely and `tools` removed from `lib.rs`. If it contains other modules, just remove the `pub mod primitives;` line.

Check current content of `crates/agents/src/tools/mod.rs`. If it is:
```rust
pub mod primitives;
```
Then delete the file and remove `pub mod tools;` from `crates/agents/src/lib.rs`.

**Step 3: Remove `ToolRegistry::with_defaults()` from tool_registry.rs**

Delete the entire impl block (lines 206-221 in current file):

```rust
// DELETE THIS:
impl ToolRegistry {
    /// Create a registry pre-loaded with all built-in generic primitive tools.
    pub fn with_defaults() -> Self {
        use crate::tools::primitives::*;
        let mut registry = Self::new();
        registry.register_primitive(Arc::new(BashTool::new()));
        // ...
        registry
    }
}
```

**Step 4: Clean up Cargo.toml**

Remove deps that were only used by primitive tools. Check each:
- `reqwest` — used by HttpFetchTool only → REMOVE (but check if runner.rs or model.rs needs it first)
- `bytes` — check if used elsewhere → if not, REMOVE
- `tokio` — likely still needed by runner → KEEP
- `anyhow` — still needed for re-export → KEEP

**Step 5: Verify**

Run: `cargo check -p rara-agents`
Expected: PASS

**Step 6: Commit**

```bash
git add -A crates/agents/
git commit -m "refactor(agents): remove primitives, delegate to tool-core"
```

---

### Task 6: Update rara-workers — remove primitives, use default_primitives()

**Files:**
- Delete: `crates/workers/src/tools/primitives/` (entire directory)
- Modify: `crates/workers/src/tools/mod.rs` — remove `pub mod primitives;`
- Modify: `crates/workers/src/worker_state.rs` — replace manual registration with `default_primitives()`

**Step 1: Delete primitives directory**

```bash
rm -rf crates/workers/src/tools/primitives/
```

**Step 2: Update workers/src/tools/mod.rs**

Remove `pub mod primitives;` line. Keep `pub mod services;` and `pub mod mcp_adapter;`.

**Step 3: Update worker_state.rs**

Replace the manual primitive registration (current lines 135-169):

```rust
// OLD:
let mut tool_registry = rara_agents::tool_registry::ToolRegistry::with_defaults();
// ... 4 manual register_primitive calls ...

// NEW:
let mut tool_registry = rara_agents::tool_registry::ToolRegistry::new();
for tool in tool_core::default_primitives(tool_core::PrimitiveDeps {
    pool:          pool.clone(),
    notify_client: notify_client.clone(),
    settings_svc:  settings_svc.clone(),
    object_store:  object_store.clone(),
}) {
    tool_registry.register_primitive(tool);
}
```

**Step 4: Verify**

Run: `cargo check -p rara-workers`
Expected: PASS

**Step 5: Commit**

```bash
git add -A crates/workers/
git commit -m "refactor(workers): remove primitives, use tool_core::default_primitives()"
```

---

### Task 7: Update imports in workers service tools

**Files:**
- Modify: all files in `crates/workers/src/tools/services/` that import `rara_agents::tool_registry::AgentTool`

These files should switch to importing from `tool_core` directly (since workers already depends on tool-core):

```rust
// OLD:
use rara_agents::tool_registry::AgentTool;

// NEW:
use tool_core::AgentTool;
```

Affected files (10):
- `resume_tools.rs`
- `skill_tools.rs`
- `job_pipeline.rs`
- `schedule_tools.rs`
- `screenshot.rs`
- `memory_tools.rs`
- `codex.rs`
- `typst_tools.rs`

Also update `mcp_adapter.rs` if it references the old import path.

**Step 1: Replace imports**

Search-and-replace in `crates/workers/src/tools/`:
```
use rara_agents::tool_registry::AgentTool  →  use tool_core::AgentTool
```

**Step 2: Verify**

Run: `cargo check -p rara-workers`
Expected: PASS

**Step 3: Commit**

```bash
git add crates/workers/src/tools/
git commit -m "refactor(workers): import AgentTool from tool-core directly"
```

---

### Task 8: Full workspace verification

**Step 1: Check entire workspace**

Run: `cargo check --workspace`
Expected: PASS with no errors

**Step 2: Run all tests**

Run: `cargo test --workspace --lib`
Expected: All tests pass (including whitelist tests from db_query/db_mutate that moved to tool-core)

**Step 3: Final commit (if any fixups needed)**

```bash
git add -A
git commit -m "chore: fix any remaining compilation issues from tool consolidation"
```
