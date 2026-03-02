# MemOS Memory Scheduling Integration Design

**Date**: 2026-03-02
**Status**: Proposed (v2 — revised after API research)
**Replaces**: mem0 (state) + usememos/memos (knowledge)

## Problem

The current memory system has **no scheduling** — only retrieval:

- `RecallEngine` fires static rules → top-k retrieval → stuff into prompt → let LLM decide
- No task-phase awareness (planning vs execution vs reflection need different memories)
- No memory prioritization beyond RRF fusion scores
- No lifecycle management (no hot/cold, no expiration, no promotion)
- No prefetch (synchronous retrieval blocks every agent turn)
- Sliding window compaction cuts by position, not importance

This leads to: prompt explosion, important information drowned, irrelevant content repeated, cost out of control.

## Solution

Replace **mem0 + usememos/memos** with **MemOS** (Memory Operating System). Keep **Hindsight** for deep reasoning (`reflect()`). Build **MemoryScheduler in Kernel Rust** to orchestrate both backends with 5-dimension scheduling.

### Two-Backend Architecture

| Layer | Backend | Role |
|-------|---------|------|
| State + Knowledge | **MemOS** (Docker) | Text Memory (facts), Preference Memory (user prefs), Tool Memory (tool traces), Skill Memory (methodologies), Knowledge Base (documents) |
| Learning | **Hindsight** (REST) | 4-network (world/experience/opinion/observation), `reflect()` deep reasoning |

### Memory Scheduling — 5 Dimensions

| Dimension | Implementation | Details |
|-----------|---------------|---------|
| **Selection** | **Kernel Rust** | MemOS API has no task_phase/agent_role concept. Kernel decides which memory types to query based on phase/role, then maps to MemOS parameters (`include_preference`, `search_tool_memory`, `include_skill_memory`, `top_k`) |
| **Prioritization** | **Kernel Rust** | MemOS returns relevance-sorted results. Kernel re-ranks with recency, access frequency, task-relevance weights |
| **Compaction** | **Kernel Rust** | MemOS has no token_budget parameter. Kernel enforces budget by trimming/summarizing after retrieval |
| **Lifecycle** | **MemOS** | MemScheduler handles async organize/archive/feedback via Redis Streams task queue |
| **Prefetch** | **Kernel Rust** | Predict next memory needs from recent_tool_calls, async pre-load into local cache |

## Architecture

### Before

```
Kernel event_loop
  → RecallEngine (local static rules)
  → MemoryManager
    → mem0 (gRPC)             ← Kernel direct call
    → usememos/memos (REST)   ← Kernel direct call
    → Hindsight (REST)        ← Kernel direct call
  → RRF Fusion (local)
  → inject into prompt
```

### After

```
Kernel event_loop
  → MemoryScheduler (Rust, in-kernel)
    ├─ Selection: pick memory types by task_phase + agent_role
    ├─ Retrieval: parallel fetch
    │   ├─ MemOS REST (state + knowledge)
    │   └─ Hindsight REST (learning, only for reflect-worthy queries)
    ├─ Prioritization: re-rank with recency/frequency/relevance
    ├─ Compaction: trim to token_budget
    └─ Prefetch cache: hit or miss
  → MemoryPackage (scheduled, ranked, trimmed)
  → inject into prompt
```

### Deployment Topology

```
┌──────────────────────────────────────┐
│              Kernel                  │
│  ┌────────────────────────────────┐  │
│  │      MemoryScheduler          │  │
│  │  Selection │ Priority │ Cache  │  │
│  └──────┬─────────────────┬──────┘  │
│         │                 │          │
└─────────┼─────────────────┼──────────┘
          │ REST            │ REST
 ┌────────▼────────┐  ┌────▼──────────┐
 │     MemOS       │  │  Hindsight    │
 │  (Python/Docker) │  │   (REST)     │
 │                  │  └──────────────┘
 │  Neo4j + Qdrant  │
 │  Redis (Scheduler)│
 └──────────────────┘
```

## MemOS API Surface (Actual)

Based on research of MemOS v2.0.7 source code.

### Search — `POST /product/search`

```python
# Request
{
    "query": "string",                    # Required
    "user_id": "string",                  # Required
    "readable_cube_ids": ["cube1"],       # MemCube isolation
    "mode": "fast" | "fine" | "mixture",  # Search depth
    "top_k": 10,                          # Result count
    "relativity": 0.45,                   # Relevance threshold
    "dedup": "no" | "sim" | "mmr",        # Dedup strategy
    "include_preference": true,           # Include user prefs
    "pref_top_k": 6,                      # Preference result count
    "search_tool_memory": true,           # Include tool traces
    "tool_mem_top_k": 6,                  # Tool memory count
    "include_skill_memory": true,         # Include skills
    "skill_mem_top_k": 3,                 # Skill count
    "filter": {},                         # Metadata filter
    "neighbor_discovery": false           # Graph traversal
}

# Response
{
    "memory_detail_list": [...],          # Main text memories
    "preference_detail_list": [...],      # User preferences
    "tool_memory_detail_list": [...],     # Tool traces
    "preference_note": "summary..."       # Preference summary
}
```

### Add Memory — `POST /product/add`

```python
{
    "messages": [{"role": "user", "content": "..."}],
    "user_id": "string",
    "readable_cube_ids": ["cube1"],
    "writable_cube_ids": ["cube1"],
    "async_mode": true,                   # Use MemScheduler
    "tags": ["goal", "failure"]           # Custom tags
}
```

### Delete — `POST /product/delete_memory`

```python
{
    "user_ids": ["user1"],
    "memory_ids": ["mem1", "mem2"]
}
```

### Feedback — `POST /product/feedback`

```python
{
    "user_id": "string",
    "conversation_id": "string",
    "feedback_content": "Actually, I prefer Python over Rust"  # Natural language correction
}
```

### Scheduler Status — `GET /product/scheduler/status`

Returns task state: `running` / `completed` / `failed` / `waiting` / `cancelled`

## Kernel-Side Types

### MemoryScheduler — Core Orchestrator

```rust
/// In-kernel memory scheduler. Owns all 5 scheduling dimensions.
pub struct MemoryScheduler {
    memos_client: MemosClient,           // MemOS REST client
    hindsight_client: HindsightClient,   // Hindsight REST client (kept)
    prefetch_cache: PrefetchCache,       // L1 memory cache
    access_tracker: AccessTracker,       // Recency + frequency tracking
    config: SchedulerConfig,
}

pub struct SchedulerConfig {
    pub memos_base_url: String,
    pub memos_api_key: String,
    pub hindsight_base_url: String,
    pub hindsight_bank_id: String,
    pub default_token_budget: u32,       // Default: 4096
    pub prefetch_cache_size: usize,      // Default: 256 entries
    pub prefetch_ttl_secs: u64,          // Default: 300
}
```

### MemoryRequest — Internal Scheduling Input

```rust
/// Built by event_loop from ProcessRuntime state.
/// NOT sent to MemOS directly — MemoryScheduler translates to MemOS API params.
pub struct MemoryRequest {
    // Identity
    pub agent_id: AgentId,
    pub agent_role: AgentRole,        // Chat, Scout, Planner, Worker
    pub session_id: SessionId,
    pub user_id: Option<UserId>,

    // Task context
    pub task_phase: TaskPhase,
    pub task_description: Option<String>,

    // Constraints
    pub token_budget: u32,

    // Signals
    pub current_query: String,
    pub recent_tool_calls: Vec<String>,
    pub turn_count: u32,
}

pub enum TaskPhase {
    Planning,     // → prefer Knowledge + Skill + Goal memories
    Execution,    // → prefer Tool + Knowledge memories
    Reflection,   // → prefer Hindsight reflect() + Failure memories
    Idle,         // → general retrieval
}
```

### Selection Logic — Phase-to-API Mapping

```rust
impl MemoryScheduler {
    /// Translate TaskPhase + AgentRole into MemOS search parameters.
    fn build_search_params(&self, req: &MemoryRequest) -> MemosSearchParams {
        let mut params = MemosSearchParams::default();

        match req.task_phase {
            TaskPhase::Planning => {
                params.include_skill_memory = true;
                params.skill_mem_top_k = 5;
                params.include_preference = true;
                params.search_tool_memory = false;  // not needed for planning
                params.top_k = 8;
                params.mode = SearchMode::Fine;      // deeper search for planning
            }
            TaskPhase::Execution => {
                params.search_tool_memory = true;
                params.tool_mem_top_k = 8;
                params.include_skill_memory = true;
                params.include_preference = false;   // not needed during execution
                params.top_k = 6;
                params.mode = SearchMode::Fast;
            }
            TaskPhase::Reflection => {
                // Reflection primarily uses Hindsight reflect()
                // MemOS search is supplementary
                params.include_preference = true;
                params.search_tool_memory = true;
                params.top_k = 4;
                params.mode = SearchMode::Fast;
            }
            TaskPhase::Idle => {
                params.include_preference = true;
                params.search_tool_memory = true;
                params.include_skill_memory = true;
                params.top_k = 6;
                params.mode = SearchMode::Fast;
            }
        }

        // Agent role adjustments
        match req.agent_role {
            AgentRole::Chat => params.include_preference = true,
            AgentRole::Worker => params.search_tool_memory = true,
            _ => {}
        }

        params
    }

    /// Decide whether to call Hindsight reflect().
    fn needs_reflect(&self, req: &MemoryRequest) -> bool {
        matches!(req.task_phase, TaskPhase::Reflection)
            || req.recent_tool_calls.iter().any(|t| t.contains("analyze"))
    }
}
```

### MemoryPackage — Scheduler Output

```rust
pub struct MemoryPackage {
    pub entries: Vec<MemoryEntry>,        // Sorted by final score
    pub total_tokens: u32,                // Actual tokens used (≤ budget)
    pub metadata: SchedulingMetadata,
}

pub struct MemoryEntry {
    pub id: String,
    pub memory_type: MemoryType,
    pub content: String,
    pub source: MemorySource,
    pub score: f32,                       // Final score after re-ranking
    pub inject_target: InjectTarget,
}

pub enum MemorySource {
    MemOS,        // From MemOS (text/preference/tool/skill/knowledge)
    Hindsight,    // From Hindsight reflect()
    PrefetchCache,// From local prefetch cache
}

pub enum MemoryType {
    // MemOS native
    Text,          // General facts
    Preference,    // User preferences
    Tool,          // Tool usage traces ("when...then..." rules)
    Skill,         // Reusable methodologies
    KnowledgeBase, // Imported documents/URLs
    // Custom (via MemOS tags)
    Goal,          // Task goals/plans
    Failure,       // Failure cases/lessons
    Decision,      // Decision records
}

pub enum InjectTarget {
    SystemPrompt,
    ContextMessage,
}

pub struct SchedulingMetadata {
    pub phase: TaskPhase,
    pub sources_queried: Vec<MemorySource>,
    pub candidates_total: u32,
    pub candidates_selected: u32,
    pub token_budget: u32,
    pub tokens_used: u32,
    pub prefetch_hits: u32,
    pub reflect_called: bool,
}
```

### PrefetchCache — L1 Memory Cache

```rust
/// LRU cache with TTL. Keyed by (user_id, query_hash).
/// Populated by prefetch predictions, hit on next schedule() call.
pub struct PrefetchCache {
    cache: LruCache<CacheKey, Vec<MemoryEntry>>,
    ttl: Duration,
}

pub struct AccessTracker {
    /// Track per-memory access count and last-access time.
    /// Used for Prioritization re-ranking.
    entries: DashMap<String, AccessRecord>,
}

pub struct AccessRecord {
    pub access_count: u32,
    pub last_accessed: Instant,
    pub first_accessed: Instant,
}
```

## Scheduling Pipeline — Full Flow

```
schedule(MemoryRequest) {
    // 1. PREFETCH CHECK
    if let Some(cached) = prefetch_cache.get(req.query_hash) {
        return trim_to_budget(cached, req.token_budget)
    }

    // 2. SELECTION — pick what to query
    let memos_params = build_search_params(&req)
    let need_reflect = needs_reflect(&req)

    // 3. PARALLEL RETRIEVAL
    let (memos_results, hindsight_results) = tokio::join!(
        memos_client.search(req.user_id, req.current_query, memos_params),
        if need_reflect {
            hindsight_client.reflect(req.current_query)
        } else {
            future::ready(vec![])
        }
    )

    // 4. MERGE + PRIORITIZE
    let mut entries = merge(memos_results, hindsight_results)
    for entry in &mut entries {
        let access = access_tracker.get_or_default(entry.id)
        entry.score = reprioritize(entry.score, access, req.task_phase)
    }
    entries.sort_by(|a, b| b.score.partial_cmp(&a.score))

    // 5. COMPACT TO BUDGET
    let package = trim_to_budget(entries, req.token_budget)

    // 6. UPDATE TRACKING
    for entry in &package.entries {
        access_tracker.record_access(entry.id)
    }

    // 7. PREFETCH — predict next needs
    if let Some(predicted_query) = predict_next_query(&req) {
        tokio::spawn(prefetch(predicted_query))
    }

    return package
}
```

## Write Flow

### Agent Tools → MemoryScheduler → MemOS/Hindsight

| Tool | MemoryScheduler Method | Backend |
|------|----------------------|---------|
| `memory_write` | `write(WriteRequest)` | MemOS `POST /product/add` |
| `memory_search` | `schedule(MemoryRequest)` | MemOS + Hindsight |
| `memory_forget` | `forget(memory_id)` | MemOS `POST /product/delete_memory` |
| `memory_feedback` | `feedback(content)` | MemOS `POST /product/feedback` (new) |

### WriteRequest

```rust
pub struct WriteRequest {
    pub agent_id: AgentId,
    pub user_id: Option<UserId>,
    pub memory_type: MemoryType,
    pub content: String,
    pub tags: Vec<String>,           // MemOS custom tags
    pub scope: MemoryScope,
    pub async_mode: bool,            // Use MemScheduler async processing
}

pub enum MemoryScope {
    Agent,          // Private — MemOS cube per agent
    Team(String),   // Shared — MemOS shared cube
    Global,         // Global — MemOS global cube
}
```

### Session Consolidation

```rust
pub struct ConsolidateRequest {
    pub agent_id: AgentId,
    pub session_id: SessionId,
    pub user_id: Option<UserId>,
    pub exchanges: Vec<Exchange>,
}

// MemoryScheduler.consolidate() does:
// 1. MemOS POST /product/add (async_mode=true) — extract facts, prefs, tool traces
// 2. Hindsight retain() — store in learning networks
// Both in parallel, best-effort
```

## Kernel Integration

### event_loop Change

```rust
// Before: ~30 lines (RecallEngine evaluate → execute → RRF → inject)
// After: ~5 lines

async fn prepare_memory(
    &self,
    process: &AgentProcess,
    runtime: &ProcessRuntime,
    user_text: &str,
) -> Option<MemoryPackage> {
    let request = MemoryRequest {
        agent_id: process.agent_id(),
        agent_role: process.manifest().role,
        session_id: runtime.session_id(),
        user_id: process.principal().user_id(),
        task_phase: runtime.current_phase(),
        task_description: runtime.task_description(),
        token_budget: self.config.memory_token_budget,
        current_query: user_text.to_string(),
        recent_tool_calls: runtime.recent_tool_names(),
        turn_count: runtime.turn_count(),
    };

    match self.memory_scheduler.schedule(request).await {
        Ok(pkg) => Some(pkg),
        Err(e) => {
            warn!("memory scheduling failed: {e}");
            None // best-effort
        }
    }
}
```

### Agent Tools Change

```
Before (8 tools):
  memory_search, memory_deep_recall, memory_add_fact, memory_write
  recall_strategy_add, recall_strategy_list, recall_strategy_update, recall_strategy_remove

After (4 tools):
  memory_write     → WriteRequest → MemOS add
  memory_search    → MemoryRequest → full scheduling pipeline
  memory_forget    → MemOS delete
  memory_feedback  → MemOS feedback (natural language memory correction)
```

## Configuration

```
Before (5 keys):
  memory.mem0.base_url
  memory.memos.base_url
  memory.memos.token
  memory.hindsight.base_url
  memory.hindsight.bank_id

After (4 keys):
  memory.memos.base_url        # MemOS service address
  memory.memos.api_key         # MemOS authentication
  memory.hindsight.base_url    # Hindsight (kept)
  memory.hindsight.bank_id     # Hindsight bank (kept)
```

## Docker Deployment

```yaml
# MemOS stack (replaces mem0 + usememos/memos)
memos:
  image: memtensor/memos:latest
  ports: ["8000:8000"]
  environment:
    MOS_ENABLE_SCHEDULER: "true"
    MOS_GRAPH_STORE_TYPE: "neo4j"
    MOS_VECTOR_STORE_TYPE: "qdrant"
  depends_on: [neo4j, qdrant, redis]

neo4j:
  image: neo4j:5-community
  ports: ["7474:7474", "7687:7687"]

qdrant:
  image: qdrant/qdrant:latest
  ports: ["6333:6333"]

redis:
  image: redis:7-alpine
  ports: ["6379:6379"]

# Kept as-is
hindsight:
  image: hindsight:latest
  # ...
```

## Migration — Delete List

### Rust Files to Delete

```
crates/memory/src/
  ├─ mem0_client.rs          # Replaced by MemOS
  ├─ memos_client.rs         # usememos client obsolete
  ├─ fusion.rs               # RRF replaced by Kernel re-ranking
  ├─ kernel_impl.rs          # StateMemory/KnowledgeMemory/LearningMemory trait impls
  ├─ lazy_client.rs          # K8s lazy init for old clients
  ├─ pod_manager.rs          # K8s pod lifecycle for old clients
  └─ recall_engine/          # Entire directory — scheduling moves to MemoryScheduler
      ├─ engine.rs
      ├─ types.rs
      ├─ defaults.rs
      └─ interpolation.rs

crates/core/kernel/src/memory/
  ├─ knowledge.rs            # KnowledgeMemory trait — MemOS covers this
  └─ state.rs                # StateMemory trait — MemOS covers this
```

### Rust Files to Add/Rewrite

```
crates/memory/src/
  ├─ memos_client.rs         # Rewrite — MemOS REST client (was usememos, now MemTensor)
  ├─ hindsight_client.rs     # Keep — Hindsight REST client (unchanged)
  ├─ scheduler.rs            # New — MemoryScheduler (5-dimension orchestrator)
  ├─ prefetch.rs             # New — PrefetchCache (LRU + TTL)
  ├─ tracker.rs              # New — AccessTracker (recency + frequency)
  ├─ types.rs                # New — MemoryRequest, MemoryPackage, WriteRequest, etc.
  ├─ manager.rs              # Rewrite — thin facade over MemoryScheduler
  └─ lib.rs                  # Rewrite — export scheduler + types

crates/core/kernel/src/memory/
  └─ learning.rs             # Keep — LearningMemory trait (Hindsight still used)

crates/core/boot/src/
  ├─ memory.rs               # Rewrite — init MemoryScheduler
  └─ tools/services/
      └─ memory_tools.rs     # Rewrite — 8 tools → 4 tools
```

## What Stays Unchanged

- **SlidingWindowCompaction** — conversation history trimming (not memory scheduling)
- **Hindsight client** — kept for `reflect()` deep reasoning
- **LearningMemory trait** — kept for Hindsight integration
- **ProcessRuntime** — local conversation state
- **Best-effort error handling** — memory failure never blocks agent
