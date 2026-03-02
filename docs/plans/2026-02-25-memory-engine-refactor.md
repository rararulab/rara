# Memory Engine Refactor — Three-Layer Integration

**Date**: 2026-02-25
**Status**: Implemented

## Problem

The current `rara-memory` crate has a homegrown memory engine that:

- Requires Chroma + PG for hybrid search (complex deployment)
- No memory prioritization or expiration (all memories are equal)
- Crude user profile (4 fixed sections in `user_profile.md`)
- Character-level chunking (not structure-aware)
- Simple token-overlap reranking (no neural reranking)
- No daily log or temporal organization
- No compaction-time memory flush

## Solution

Replace the homegrown engine with three specialized open-source services, each representing a different memory philosophy:

| Service | Role | Docker Image |
|---------|------|-------------|
| **mem0** | State layer — structured facts, auto-dedup, conflict resolution | `mem0/mem0-api-server` |
| **Memos** | Storage layer — human-readable Markdown notes, tags, daily logs | `neosmemo/memos:stable` |
| **Hindsight** | Learning layer — 4-network retain/recall/reflect with neural reranking | `ghcr.io/vectorize-io/hindsight:latest` |

## Architecture

### Infrastructure

```
┌──────────────────────────────────────────────┐
│              Docker Services                  │
│                                               │
│  postgres       → rara-app (business data)    │
│  postgres-hs    → Hindsight (pgvector)        │
│  postgres-memos → Memos                       │
│  chroma         → mem0 (vector storage)       │
│                                               │
│  mem0:8000  memos:5230  hindsight:8888        │
└──────────────────────────────────────────────┘
```

Each service gets its own PostgreSQL instance to avoid cross-contamination.

### Facade Design

```
┌──────────────────────────────────────────────┐
│              MemoryManager (facade)           │
│  ┌─────────┐  ┌──────────┐  ┌─────────────┐ │
│  │ Mem0     │  │ Memos    │  │ Hindsight   │ │
│  │ Client   │  │ Client   │  │ Client      │ │
│  │(state)   │  │(storage) │  │(learning)   │ │
│  └────┬─────┘  └────┬─────┘  └──────┬──────┘ │
└───────┼─────────────┼───────────────┼────────┘
        ▼             ▼               ▼
   mem0:8000     memos:5230    hindsight:8888
```

Upper layers only interact with `MemoryManager`. Internal routing dispatches to the appropriate backend based on operation type.

### Data Flow — Conversation Lifecycle

```
User sends message
    │
    ▼
[prepare_session_data]
    ├── Check session inactivity (≥30 min idle?)
    │   └── YES → [spawn_session_consolidation] (background async)
    │              ├── mem0.add(all session exchanges) → batch fact extraction
    │              └── hindsight.retain(full session text) → 4-network storage
    │
    ▼
[build_chat_system_prompt]
    ├── mem0.search(user_text) → inject relevant facts
    ├── hindsight.recall(user_text) → inject deep memory context
    └── memos.list(filter=pinned) → inject pinned notes
    │
    ▼
[AgentRunner generates response]
    │   (no per-turn memory writes)
    ▼
[Response returned to user]
```

> **Note (2026-02-25)**: Per-turn `spawn_memory_reflection` was removed in
> [#318](https://github.com/rararulab/rara/issues/318). Memory consolidation now
> triggers only at session boundaries (inactivity ≥30 min) or via explicit
> agent tools (`memory_add_fact`, `memory_write`).

### Search Routing (`memory_search` tool)

1. Parallel query: `mem0.search` + `hindsight.recall`
2. Reciprocal Rank Fusion to merge results
3. Return to Agent

### Write Routing (`memory_write` tool)

- Markdown content → **Memos** (create note with tags)
- Profile updates → **mem0** (update facts)

## Crate Structure

```
crates/memory/src/
  lib.rs              # pub mod + re-exports
  error.rs            # MemoryError (snafu)
  manager.rs          # MemoryManager facade
  mem0_client.rs      # Mem0Client (reqwest)
  memos_client.rs     # MemosClient (reqwest)
  hindsight_client.rs # HindsightClient (reqwest)
  fusion.rs           # RRF fusion + result normalization
  recall_engine/      # Agent-configurable recall strategy engine (#322)
    mod.rs            # module declarations + re-exports
    types.rs          # RecallRule, Trigger, RecallAction, EventKind, InjectTarget
    engine.rs         # RecallStrategyEngine (evaluate + execute + CRUD)
    interpolation.rs  # query_template variable substitution
    defaults.rs       # default rules (user-profile, post-compaction, etc.)
```

### Files to Remove

- `store.rs` — replaced by external services
- `store_pg.rs` — replaced by external services
- `chroma.rs` — Chroma is now mem0's internal backend
- `reranking.rs` — Hindsight has built-in neural reranking

### MemoryManager API

```rust
pub struct MemoryManager {
    mem0: Mem0Client,
    memos: MemosClient,
    hindsight: HindsightClient,
    user_id: String,
}

impl MemoryManager {
    /// Search: parallel mem0 + hindsight, RRF fusion
    pub async fn search(&self, query: &str, limit: usize)
        -> MemoryResult<Vec<SearchResult>>;

    /// Write a Markdown note to Memos (on-demand only)
    pub async fn write_note(&self, content: &str, tags: &[&str])
        -> MemoryResult<String>;

    /// Session-end consolidation: batch mem0.add + hindsight.retain
    pub async fn consolidate_session(
        &self, exchanges: &[(String, String)]
    ) -> MemoryResult<()>;

    /// Store a single explicit fact in mem0 + Hindsight
    pub async fn add_fact(&self, content: &str) -> MemoryResult<()>;

    /// Read user facts from mem0
    pub async fn get_user_profile(&self) -> MemoryResult<Vec<Mem0Fact>>;

    /// Deep recall via Hindsight reflect
    pub async fn deep_recall(&self, query: &str) -> MemoryResult<String>;
}
```

## Service APIs

### mem0 (http://localhost:8000)

- `POST /v1/memories/` — add memories (messages + user_id, auto-infer facts)
- `POST /v1/memories/search/` — search (query + user_id + top_k)
- `GET /v1/memories/{id}/` — get single memory
- `PUT /v1/memories/{id}/` — update memory
- `DELETE /v1/memories/{id}/` — delete memory

### Memos (http://localhost:5230)

- `POST /api/v1/memos` — create memo (content + visibility + tags via #tag)
- `GET /api/v1/memos` — list (pageSize + filter)
- `GET /api/v1/memos/{id}` — get memo
- `PATCH /api/v1/memos/{id}` — update
- `DELETE /api/v1/memos/{id}` — delete
- Auth: `Authorization: Bearer <token>`

### Hindsight (http://localhost:8888)

- retain(bank_id, content) — store memories into 4 networks
- recall(bank_id, query) — hybrid retrieval (semantic + BM25 + graph + temporal)
- reflect(bank_id, query) — personality-conditioned reasoning

## Configuration

New Consul KV keys:

```
rara/config/memory/mem0_base_url     = http://mem0:8000
rara/config/memory/memos_base_url    = http://memos:5230
rara/config/memory/memos_token       = <access-token>
rara/config/memory/hindsight_base_url = http://hindsight:8888
rara/config/memory/hindsight_bank_id  = rara-default
```

## Implementation Issues

### Issue 1: Infrastructure — Add Memos + Hindsight to Docker Compose & Helm
- Add `postgres-hs`, `postgres-memos`, `memos`, `hindsight` services
- Add Consul KV seed entries
- Add Helm chart values
- Remove Chroma dependency from `app` service (keep Chroma for mem0 only)

### Issue 2: HTTP Clients — mem0, Memos, Hindsight
- Create `mem0_client.rs`, `memos_client.rs`, `hindsight_client.rs`
- reqwest-based, snafu errors
- Unit tests with mock server (wiremock)

### Issue 3: MemoryManager Facade + Fusion
- Rewrite `manager.rs` with new facade
- Create `fusion.rs` for RRF
- Create `error.rs` with new error types
- Remove old files (store.rs, store_pg.rs, chroma.rs, reranking.rs)

### Issue 4: Tool Layer + Orchestrator Integration
- Update `memory_tools.rs` to use new MemoryManager (`add_fact` instead of `reflect_on_exchange`)
- Update `orchestrator/core.rs` with `spawn_session_consolidation` (replaces per-turn `spawn_memory_reflection`)
- Update `chat/service.rs` with session inactivity detection (30 min threshold)
- Remove `MemorySyncWorker`

### Issue 5: Settings + App Composition
- Add memory config to `Settings` model
- Update `rara-app` composition root
- Wire new MemoryManager with config from Consul
