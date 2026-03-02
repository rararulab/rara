# MemOS Memory Scheduling Integration Design

**Date**: 2026-03-02
**Status**: Proposed
**Replaces**: usememos/memos (note service) as knowledge layer

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

Replace usememos/memos (Markdown note service) with **MemOS** (Memory Operating System) as the **knowledge layer + global memory scheduler**. MemOS becomes the single point of contact for the kernel — it orchestrates all three memory layers.

### Memory Scheduling — 5 Dimensions

| Dimension | What It Does | OS Analogy |
|-----------|-------------|------------|
| **Selection** | Choose memory types by task phase, agent role, tool usage | mmap — map on demand |
| **Prioritization** | Score and rank by recency, access frequency, task relevance | Page replacement (LRU/LFU) |
| **Compaction** | Compress, merge, summarize when context budget is tight | Memory compaction / defrag |
| **Lifecycle** | Hot/cold/expired/promoted memory tiers | Hot/cold pages + swap |
| **Prefetch** | Proactively load memories before agent needs them | Page prefetch / readahead |

## Architecture

### Before

```
Kernel event_loop
  → RecallEngine (local rules)
  → MemoryManager
    → mem0 (gRPC)        ← Kernel direct call
    → usememos/memos (REST)  ← Kernel direct call
    → Hindsight (REST)   ← Kernel direct call
  → RRF Fusion (local)
  → inject into prompt
```

### After

```
Kernel event_loop
  → MemosClient::schedule(MemoryRequest { context })
  → MemOS Service (Docker)
    ├─ MemScheduler (5-dimension scheduling)
    ├─ MemCube (knowledge layer, replaces usememos/memos)
    ├─ → mem0 (state layer, orchestrated by MemOS)
    └─ → Hindsight (learning layer, orchestrated by MemOS)
  → MemoryPackage (fused, scheduled, trimmed)
  → inject into prompt
```

### Three-Layer Architecture (preserved)

| Layer | Backend | Role |
|-------|---------|------|
| State | mem0 (gRPC) | Structured fact extraction & semantic search |
| Knowledge | **MemOS MemCube** (replaces usememos/memos) | Documents, knowledge bases, tagged content |
| Learning | Hindsight (REST) | 4-network (world/experience/opinion/observation) |

### Deployment Topology

```
┌─────────────┐
│   Kernel    │──── REST ────┐
└─────────────┘              │
                    ┌────────▼────────┐
                    │     MemOS       │
                    │  (Python/Docker)│
                    │                 │
                    │  MemScheduler   │
                    │  MemCube (知识)  │
                    │  Redis Streams  │
                    └──┬──────────┬───┘
                       │          │
              ┌────────▼──┐  ┌───▼──────────┐
              │   mem0    │  │  Hindsight   │
              │  (gRPC)   │  │   (REST)     │
              └───────────┘  └──────────────┘
```

## API Types

### MemoryRequest — Kernel → MemOS

```rust
struct MemoryRequest {
    // Who is requesting
    agent_id: AgentId,
    agent_role: AgentRole,        // Chat, Scout, Planner, Worker
    session_id: SessionId,
    user_id: Option<UserId>,

    // Current task phase
    task_phase: TaskPhase,        // Planning, Execution, Reflection, Idle
    task_description: Option<String>,

    // Constraints
    token_budget: u32,            // Hard limit for memory package tokens

    // Context signals
    current_query: String,        // Current user message / task input
    recent_tool_calls: Vec<String>, // Recently used tool names
    turn_count: u32,              // Session turn count

    // Memory type hints (MemOS may ignore)
    preferred_types: Vec<MemoryType>,
}

enum TaskPhase {
    Planning,    // Needs goal memories, historical plans
    Execution,   // Needs step memories, tool records
    Reflection,  // Needs failure cases, decision history
    Idle,        // General retrieval
}

enum MemoryType {
    // MemOS native
    Knowledge,    // Documents, resumes, job requirements
    Conversation, // Conversation history
    Tool,         // Tool call traces
    Persona,      // User preferences, personality
    // Custom extensions
    Goal,         // Task goals, plans
    Failure,      // Failure cases, lessons
    Decision,     // Decision records
}
```

### MemoryPackage — MemOS → Kernel

```rust
struct MemoryPackage {
    // Scheduled entries, sorted by priority
    entries: Vec<MemoryEntry>,

    // Metadata
    total_tokens: u32,
    scheduling_metadata: SchedulingMetadata,
}

struct MemoryEntry {
    id: String,
    memory_type: MemoryType,
    content: String,              // Already compressed/summarized
    source_layer: SourceLayer,
    score: f32,                   // MemOS composite score
    inject_target: InjectTarget,
}

enum SourceLayer {
    State,      // From mem0
    Knowledge,  // From MemOS MemCube
    Learning,   // From Hindsight
}

enum InjectTarget {
    SystemPrompt,    // Prepend to system message
    ContextMessage,  // Inject as standalone context message
}

struct SchedulingMetadata {
    phase_used: TaskPhase,
    layers_queried: Vec<SourceLayer>,
    candidates_considered: u32,
    candidates_selected: u32,
    compaction_applied: bool,
    prefetch_hits: u32,
}
```

### WriteRequest — Kernel → MemOS

```rust
struct WriteRequest {
    agent_id: AgentId,
    user_id: Option<UserId>,
    memory_type: MemoryType,
    content: String,
    tags: Vec<String>,
    scope: MemoryScope,
}

enum MemoryScope {
    Agent,          // Private to agent
    Team(String),   // Shared within team
    Global,         // Shared across all
}

struct ConsolidateRequest {
    agent_id: AgentId,
    session_id: SessionId,
    user_id: Option<UserId>,
    exchanges: Vec<Exchange>,
}
```

### MemOS Internal Routing for Writes

| MemoryType | Target Layer | Backend |
|-----------|-------------|---------|
| Persona, Conversation | State | mem0 |
| Knowledge, Goal, Decision | Knowledge | MemOS MemCube |
| Failure | Learning | Hindsight |
| consolidate | All three | MemOS orchestrates parallel write |

## Kernel Integration

### event_loop Change

```rust
// Before: ~30 lines of RecallEngine logic
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
        preferred_types: vec![],
    };

    match self.memos_client.schedule(request).await {
        Ok(pkg) => Some(pkg),
        Err(e) => {
            warn!("memory scheduling failed: {e}");
            None // best-effort: memory failure doesn't block agent
        }
    }
}
```

### Agent Tools Change

```
Before (8 tools):
  memory_search, memory_deep_recall, memory_add_fact, memory_write
  recall_strategy_add, recall_strategy_list, recall_strategy_update, recall_strategy_remove

After (3 tools):
  memory_write    → WriteRequest
  memory_search   → MemoryRequest (manual retrieval, bypasses auto-scheduling)
  memory_forget   → Delete specific memory (new)
```

All 4 recall_strategy tools deleted — agents no longer manage scheduling rules.

## Configuration Change

```
Before (5 keys):
  memory.mem0.base_url
  memory.memos.base_url
  memory.memos.token
  memory.hindsight.base_url
  memory.hindsight.bank_id

After (2 keys):
  memory.memos.base_url     # MemOS service address
  memory.memos.api_key      # MemOS authentication
```

mem0 and Hindsight addresses move to MemOS-side configuration.

## Migration — Delete List

### Rust Files to Delete

```
crates/memory/src/
  ├─ mem0_client.rs          # Kernel no longer connects directly
  ├─ memos_client.rs         # usememos client obsolete
  ├─ hindsight_client.rs     # Kernel no longer connects directly
  ├─ fusion.rs               # RRF now inside MemOS
  ├─ kernel_impl.rs          # 3 trait impls no longer needed
  ├─ lazy_client.rs          # K8s lazy init goes with old clients
  ├─ pod_manager.rs          # Same
  └─ recall_engine/          # Entire directory
      ├─ engine.rs
      ├─ types.rs
      ├─ defaults.rs
      └─ interpolation.rs

crates/core/kernel/src/memory/
  ├─ knowledge.rs            # KnowledgeMemory trait
  ├─ learning.rs             # LearningMemory trait
  └─ state.rs                # StateMemory trait
```

### Rust Files to Add/Rewrite

```
crates/memory/src/
  ├─ client.rs               # New — MemOS REST client (single client)
  ├─ types.rs                # New — MemoryRequest, MemoryPackage, WriteRequest, etc.
  ├─ manager.rs              # Rewrite — thin wrapper over MemOS client
  └─ lib.rs                  # Rewrite — export client + types only

crates/core/boot/src/
  ├─ memory.rs               # Rewrite — init needs only memos_base_url + api_key
  └─ tools/services/
      └─ memory_tools.rs     # Rewrite — 8 tools → 3 tools
```

### Docker Changes

```
docker-compose.yml:
  - Remove: memos service (neosmemo/memos:stable)
  - Add: memos service (memtensor/memos + Redis + PG)
  - Keep: mem0, hindsight (but remove from Kernel config, add to MemOS config)

helm/:
  - Corresponding K8s deployment adjustments
```

## What Stays Unchanged

- **SlidingWindowCompaction** in kernel — this is conversation history management, not memory scheduling
- **ProcessRuntime** conversation storage — local turn-by-turn state
- **Session consolidation timing** — kernel decides when to trigger, MemOS executes
- **Best-effort error handling** — memory failure never blocks agent execution
