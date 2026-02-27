# memory-core Trait Abstraction Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Create `crates/core/memory-core` crate defining the 3-layer memory trait abstraction (State / Knowledge / Learning) with 3-scope support (Global / Team / Agent).

**Architecture:** Pure trait + types crate at Layer 0. Zero business logic, zero external service dependencies. Defines `StateMemory`, `KnowledgeMemory`, `LearningMemory` traits with `MemoryContext` (identity) + `Scope` (visibility partition). Future `rara-memory` will implement these traits against mem0/Memos/Hindsight backends.

**Tech Stack:** uuid, jiff, serde, serde_json, async-trait, snafu

---

### Task 1: Create crate skeleton and register in workspace

**Files:**
- Create: `crates/core/memory-core/Cargo.toml`
- Create: `crates/core/memory-core/src/lib.rs` (empty placeholder)
- Modify: `Cargo.toml` (workspace root)

**Step 1: Create Cargo.toml**

```toml
[package]
name = "memory-core"
version = "0.0.1"
edition.workspace = true
license.workspace = true
authors.workspace = true
repository.workspace = true
homepage.workspace = true
readme.workspace = true
keywords.workspace = true
categories.workspace = true
description = "Memory trait abstraction: State / Knowledge / Learning × Global / Team / Agent"

[dependencies]
async-trait.workspace = true
jiff.workspace = true
serde.workspace = true
serde_json.workspace = true
snafu.workspace = true
uuid.workspace = true

[lints]
workspace = true
```

**Step 2: Create empty lib.rs**

```rust
// Copyright 2025 Crrow
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//      http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
```

**Step 3: Register in workspace**

Add to `Cargo.toml` workspace `members` array:
```
"crates/core/memory-core",
```

Add to `[workspace.dependencies]`:
```
memory-core = { path = "crates/core/memory-core" }
```

**Step 4: Verify**

Run: `cargo check -p memory-core`
Expected: PASS (empty lib)

**Step 5: Commit**

```bash
git add crates/core/memory-core/ Cargo.toml
git commit -m "chore(memory-core): scaffold empty crate"
```

---

### Task 2: Define error types

**Files:**
- Create: `crates/core/memory-core/src/error.rs`
- Modify: `crates/core/memory-core/src/lib.rs`

**Step 1: Write error.rs**

```rust
// Copyright 2025 Crrow
// ... (license header)

use snafu::Snafu;

/// Errors from the memory abstraction layer.
#[derive(Debug, Snafu)]
pub enum MemoryError {
    /// State layer backend error.
    #[snafu(display("state memory error: {message}"))]
    State { message: String },

    /// Knowledge layer backend error.
    #[snafu(display("knowledge memory error: {message}"))]
    Knowledge { message: String },

    /// Learning layer backend error.
    #[snafu(display("learning memory error: {message}"))]
    Learning { message: String },

    /// Target record does not exist.
    #[snafu(display("not found: {id}"))]
    NotFound { id: uuid::Uuid },

    /// Insufficient scope permissions.
    #[snafu(display("scope denied: {message}"))]
    ScopeDenied { message: String },
}

/// Convenience alias used by all memory trait methods.
pub type Result<T> = std::result::Result<T, MemoryError>;
```

**Step 2: Update lib.rs**

```rust
// ... (license header)

pub mod error;

pub use error::{MemoryError, Result};
```

**Step 3: Verify**

Run: `cargo check -p memory-core`
Expected: PASS

**Step 4: Commit**

```bash
git add crates/core/memory-core/src/error.rs crates/core/memory-core/src/lib.rs
git commit -m "feat(memory-core): add MemoryError with snafu"
```

---

### Task 3: Define shared types

**Files:**
- Create: `crates/core/memory-core/src/types.rs`
- Modify: `crates/core/memory-core/src/lib.rs`

**Step 1: Write types.rs**

```rust
// Copyright 2025 Crrow
// ... (license header)

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ─── Identity ────────────────────────────────────────────────────────

/// Identity context for the entity performing memory operations.
#[derive(Debug, Clone)]
pub struct MemoryContext {
    /// Who is the end-user.
    pub user_id: Uuid,
    /// Which agent is operating.
    pub agent_id: Uuid,
    /// Current session / run (if any).
    pub session_id: Option<Uuid>,
}

// ─── Scope ───────────────────────────────────────────────────────────

/// Visibility partition for memory records.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Scope {
    /// Shared across all agents.
    Global,
    /// Shared within a team / project.
    Team(Uuid),
    /// Private to the agent identified by `MemoryContext::agent_id`.
    Agent,
}

// ─── State Layer types ───────────────────────────────────────────────

/// A conversation message fed into [`StateMemory::add`](crate::StateMemory::add).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

/// A single structured fact extracted and maintained by the state layer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateFact {
    pub id: Uuid,
    pub content: String,
    #[serde(default)]
    pub score: Option<f64>,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
    #[serde(default)]
    pub created_at: Option<Timestamp>,
    #[serde(default)]
    pub updated_at: Option<Timestamp>,
}

/// Result of a single [`StateMemory::add`](crate::StateMemory::add) event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateEvent {
    pub id: Uuid,
    /// `ADD`, `UPDATE`, `DELETE`, or `NOOP`.
    pub event: String,
    pub content: String,
    #[serde(default)]
    pub previous_content: Option<String>,
}

/// One entry in the change history of a [`StateFact`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateHistory {
    pub id: Uuid,
    pub memory_id: Uuid,
    #[serde(default)]
    pub old_content: Option<String>,
    #[serde(default)]
    pub new_content: Option<String>,
    pub event: String,
    #[serde(default)]
    pub created_at: Option<Timestamp>,
    pub is_deleted: bool,
}

// ─── Knowledge Layer types ───────────────────────────────────────────

/// A persistent knowledge note managed by the knowledge layer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeNote {
    pub id: Uuid,
    pub content: String,
    pub tags: Vec<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

// ─── Learning Layer types ────────────────────────────────────────────

/// A single entry recalled from the learning layer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecallEntry {
    pub id: Uuid,
    pub content: String,
    pub score: f64,
}
```

**Step 2: Update lib.rs — add `pub mod types` and re-exports**

```rust
// ... (license header)

pub mod error;
pub mod types;

pub use error::{MemoryError, Result};
pub use types::*;
```

**Step 3: Verify**

Run: `cargo check -p memory-core`
Expected: PASS

**Step 4: Commit**

```bash
git add crates/core/memory-core/src/types.rs crates/core/memory-core/src/lib.rs
git commit -m "feat(memory-core): add MemoryContext, Scope, and data types"
```

---

### Task 4: Define StateMemory trait

**Files:**
- Create: `crates/core/memory-core/src/state.rs`
- Modify: `crates/core/memory-core/src/lib.rs`

**Step 1: Write state.rs**

```rust
// Copyright 2025 Crrow
// ... (license header)

//! State memory layer — structured fact extraction and retrieval.
//!
//! Semantically equivalent to mem0: automatic inference, deduplication,
//! and semantic search over structured facts.

use uuid::Uuid;

use crate::{
    error::Result,
    types::{MemoryContext, Message, Scope, StateEvent, StateFact, StateHistory},
};

/// Structured fact memory — infer, store, search, and manage facts.
#[async_trait::async_trait]
pub trait StateMemory: Send + Sync {
    /// Extract facts from conversation messages and store them.
    ///
    /// The implementation may perform automatic inference and deduplication.
    async fn add(
        &self,
        ctx: &MemoryContext,
        scope: Scope,
        messages: Vec<Message>,
    ) -> Result<Vec<StateEvent>>;

    /// Semantic search over stored facts.
    async fn search(
        &self,
        ctx: &MemoryContext,
        scope: Scope,
        query: &str,
        limit: usize,
    ) -> Result<Vec<StateFact>>;

    /// Retrieve a single fact by ID.
    async fn get(&self, ctx: &MemoryContext, scope: Scope, id: Uuid) -> Result<Option<StateFact>>;

    /// List all facts within the given scope.
    async fn get_all(
        &self,
        ctx: &MemoryContext,
        scope: Scope,
        limit: usize,
    ) -> Result<Vec<StateFact>>;

    /// Update the content of a single fact.
    async fn update(
        &self,
        ctx: &MemoryContext,
        scope: Scope,
        id: Uuid,
        data: &str,
    ) -> Result<()>;

    /// Delete a single fact.
    async fn delete(&self, ctx: &MemoryContext, scope: Scope, id: Uuid) -> Result<()>;

    /// Delete all facts within the given scope.
    async fn delete_all(&self, ctx: &MemoryContext, scope: Scope) -> Result<()>;

    /// Retrieve the change history for a single fact.
    async fn history(
        &self,
        ctx: &MemoryContext,
        scope: Scope,
        id: Uuid,
    ) -> Result<Vec<StateHistory>>;
}
```

**Step 2: Update lib.rs — add `pub mod state` and re-export**

Add:
```rust
pub mod state;
pub use state::StateMemory;
```

**Step 3: Verify**

Run: `cargo check -p memory-core`
Expected: PASS

**Step 4: Commit**

```bash
git add crates/core/memory-core/src/state.rs crates/core/memory-core/src/lib.rs
git commit -m "feat(memory-core): add StateMemory trait"
```

---

### Task 5: Define KnowledgeMemory trait

**Files:**
- Create: `crates/core/memory-core/src/knowledge.rs`
- Modify: `crates/core/memory-core/src/lib.rs`

**Step 1: Write knowledge.rs**

```rust
// Copyright 2025 Crrow
// ... (license header)

//! Knowledge memory layer — persistent document and note storage.
//!
//! Semantically equivalent to Memos: human-readable Markdown notes
//! with tag-based organisation.

use uuid::Uuid;

use crate::{
    error::Result,
    types::{KnowledgeNote, MemoryContext, Scope},
};

/// Persistent knowledge note storage.
#[async_trait::async_trait]
pub trait KnowledgeMemory: Send + Sync {
    /// Write a knowledge note with optional tags.
    async fn write(
        &self,
        ctx: &MemoryContext,
        scope: Scope,
        content: &str,
        tags: &[&str],
    ) -> Result<KnowledgeNote>;

    /// Read a single note by ID.
    async fn read(
        &self,
        ctx: &MemoryContext,
        scope: Scope,
        id: Uuid,
    ) -> Result<Option<KnowledgeNote>>;

    /// List notes, optionally filtered by tags.
    async fn list(
        &self,
        ctx: &MemoryContext,
        scope: Scope,
        tags: &[&str],
        limit: usize,
    ) -> Result<Vec<KnowledgeNote>>;

    /// Delete a single note.
    async fn delete(&self, ctx: &MemoryContext, scope: Scope, id: Uuid) -> Result<()>;
}
```

**Step 2: Update lib.rs — add `pub mod knowledge` and re-export**

Add:
```rust
pub mod knowledge;
pub use knowledge::KnowledgeMemory;
```

**Step 3: Verify**

Run: `cargo check -p memory-core`
Expected: PASS

**Step 4: Commit**

```bash
git add crates/core/memory-core/src/knowledge.rs crates/core/memory-core/src/lib.rs
git commit -m "feat(memory-core): add KnowledgeMemory trait"
```

---

### Task 6: Define LearningMemory trait

**Files:**
- Create: `crates/core/memory-core/src/learning.rs`
- Modify: `crates/core/memory-core/src/lib.rs`

**Step 1: Write learning.rs**

```rust
// Copyright 2025 Crrow
// ... (license header)

//! Learning memory layer — experience retention, recall, and reflection.
//!
//! Semantically equivalent to Hindsight's 4-network model:
//! retain experiences, recall relevant ones, and reflect for synthesis.

use uuid::Uuid;

use crate::{
    error::Result,
    types::{MemoryContext, RecallEntry, Scope},
};

/// Experience-based learning memory.
#[async_trait::async_trait]
pub trait LearningMemory: Send + Sync {
    /// Store content into long-term experience memory.
    async fn retain(&self, ctx: &MemoryContext, scope: Scope, content: &str) -> Result<()>;

    /// Semantically recall relevant experiences.
    async fn recall(
        &self,
        ctx: &MemoryContext,
        scope: Scope,
        query: &str,
        limit: usize,
    ) -> Result<Vec<RecallEntry>>;

    /// Deep reflection — synthesise an answer by reasoning across all
    /// stored experiences.
    async fn reflect(&self, ctx: &MemoryContext, scope: Scope, query: &str) -> Result<String>;
}
```

**Step 2: Update lib.rs — add `pub mod learning` and re-export**

Add:
```rust
pub mod learning;
pub use learning::LearningMemory;
```

**Step 3: Verify**

Run: `cargo check -p memory-core`
Expected: PASS

**Step 4: Commit**

```bash
git add crates/core/memory-core/src/learning.rs crates/core/memory-core/src/lib.rs
git commit -m "feat(memory-core): add LearningMemory trait"
```

---

### Task 7: Final verification

**Step 1: Full workspace check**

Run: `cargo check`
Expected: PASS — memory-core compiles, no breakage in other crates

**Step 2: Clippy**

Run: `cargo clippy -p memory-core`
Expected: No warnings

**Step 3: Squash commit (optional)**

If all tasks were committed individually, optionally squash into one:
```bash
git log --oneline -6
# If desired:
# git rebase -i HEAD~6 → squash into "feat(memory-core): 3-layer memory trait abstraction"
```
