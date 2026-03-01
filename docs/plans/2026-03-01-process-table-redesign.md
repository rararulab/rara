# ProcessTable Redesign — Tree Index + AgentRegistry + Routing Overhaul

**Date**: 2026-03-01
**Status**: Approved

## Motivation

Current `ProcessTable` uses a flat `DashMap<AgentId, AgentProcess>` with a `name_index: DashMap<String, AgentId>` that only tracks the latest process per name. This has several problems:

1. `children_of()` does O(n) full-table scan — inefficient for cascade operations
2. `name_index` overwrites previous entries — can't track multiple instances of same agent
3. `ManifestLoader` conflates file loading with runtime agent lookup
4. Message routing lacks clear semantics for targeting completed/dead processes

## Design

### 1. ProcessTable New Structure

```rust
pub struct ProcessTable {
    /// Primary storage (unchanged)
    processes: DashMap<AgentId, AgentProcess>,

    /// Channel session → AgentId (unchanged)
    /// Only root processes register here
    session_index: DashMap<SessionId, AgentId>,

    /// NEW: Parent → Children index, O(1) child lookup
    children_index: DashMap<AgentId, Vec<AgentId>>,

    /// RENAMED from name_index: name → Vec<AgentId>
    /// Not used for routing — observability only ("how many scouts running?")
    name_registry: DashMap<String, Vec<AgentId>>,

    /// Counters (unchanged)
    total_spawned: AtomicU64,
    total_completed: AtomicU64,
    total_failed: AtomicU64,
}
```

Key changes:
- **`children_index`**: maintained on insert/remove, makes `children_of()` O(1) instead of O(n)
- **`name_index` → `name_registry`**: 1:1 (overwrite) → 1:N (`Vec<AgentId>`), no longer used for routing
- **`metrics` DashMap removed**: `RuntimeMetrics` moved into `AgentProcess` as `Arc<RuntimeMetrics>` field

### 2. AgentProcess Changes

```rust
pub struct AgentProcess {
    // ... existing fields ...
    pub metrics: Arc<RuntimeMetrics>,  // moved from ProcessTable.metrics
}
```

### 3. ProcessTable API Changes

#### Insert
```rust
pub fn insert(&self, process: AgentProcess) {
    let id = process.agent_id;
    // Session index (unchanged — only root processes)
    if let Some(ref sid) = process.channel_session_id {
        self.session_index.insert(sid.clone(), id);
    }
    // Children index (NEW)
    if let Some(parent_id) = process.parent_id {
        self.children_index.entry(parent_id).or_default().push(id);
    }
    // Initialize empty children list for this process
    self.children_index.entry(id).or_default();
    // Name registry (1:N)
    self.name_registry.entry(process.manifest.name.clone()).or_default().push(id);
    self.total_spawned.fetch_add(1, Ordering::Relaxed);
    self.processes.insert(id, process);
}
```

#### Remove
```rust
pub fn remove(&self, id: AgentId) -> Option<AgentProcess> {
    let removed = self.processes.remove(&id).map(|(_, p)| p);
    if let Some(ref process) = removed {
        // Session index cleanup
        if let Some(ref sid) = process.channel_session_id {
            self.session_index.remove_if(sid, |_, aid| *aid == id);
        }
        // Children index: remove from parent's children list
        if let Some(parent_id) = process.parent_id {
            if let Some(mut children) = self.children_index.get_mut(&parent_id) {
                children.retain(|c| *c != id);
            }
        }
        // Remove own children entry
        self.children_index.remove(&id);
        // Name registry: remove from vec
        if let Some(mut ids) = self.name_registry.get_mut(&process.manifest.name) {
            ids.retain(|aid| *aid != id);
        }
    }
    removed
}
```

#### children_of (O(1) now)
```rust
pub fn children_of(&self, parent_id: AgentId) -> Vec<AgentId> {
    self.children_index
        .get(&parent_id)
        .map(|ids| ids.clone())
        .unwrap_or_default()
}
```

### 4. Message Routing State Machine

Three distinct routing paths based on what addressing info is available:

```
InboundMessage received
  │
  ├─ Has agent_id (direct addressing)?
  │   ├─ Process alive (Running/Waiting/Paused) → deliver (or buffer)
  │   ├─ Process terminal (Completed) → return error + AgentResult
  │   ├─ Process terminal (Failed/Cancelled) → return error
  │   └─ Process not found → return error "process not found"
  │
  ├─ Has session_id matching session_index?
  │   ├─ Process alive → deliver
  │   └─ Process terminal → clear session binding, spawn new process (transparent to user)
  │
  └─ Has agent_name only (no ID, no session match)?
      └─ Lookup AgentRegistry → spawn new process (always)
```

Routing strategies per source (based on industry best practices):
- **ID addressing** (agent-to-agent): Precise delivery, error if dead (A2A Protocol pattern)
- **Session addressing** (external user): Transparent respawn (AutoGen lazy instantiation)
- **Name addressing** (no prior context): Always spawn new (Anthropic pattern)

### 5. AgentRegistry

Replaces `ManifestLoader` as the runtime agent lookup layer.

```rust
pub struct AgentRegistry {
    /// Built-in agents (code-defined, immutable, not persisted)
    builtin: Vec<AgentManifest>,
    /// User/dynamic agents (CRUD, persisted as YAML)
    custom: DashMap<String, AgentManifest>,
    /// Persistence directory for custom agents
    agents_dir: PathBuf,
}

impl AgentRegistry {
    /// Bootstrap: load builtin + ManifestLoader results
    pub fn init(builtin: Vec<AgentManifest>, loader: &ManifestLoader, agents_dir: PathBuf) -> Self;

    /// Lookup: custom first (shadow), then builtin
    pub fn get(&self, name: &str) -> Option<AgentManifest>;

    /// List all available agents (builtin + custom)
    pub fn list(&self) -> Vec<AgentManifest>;

    /// Dynamic register: in-memory + write YAML to agents_dir/{name}.yaml
    pub fn register(&self, manifest: AgentManifest) -> Result<()>;

    /// Unregister: custom only. Builtin returns error.
    pub fn unregister(&self, name: &str) -> Result<()>;
}
```

Semantics:
- `get()` checks `custom` first, then `builtin` — user agents shadow built-in ones
- `register()` writes to memory + persists `{agents_dir}/{name}.yaml`
- `unregister()` removes from memory + deletes YAML file. Rejects builtin agents.
- `ManifestLoader` becomes a pure file loader, only used at startup to feed `AgentRegistry.init()`

### 6. ManifestLoader Changes

`ManifestLoader` stays as-is but is demoted to a startup utility:
- Still loads YAML files from directory → `AgentManifest` objects
- No longer stored in `KernelInner` — only used during `AgentRegistry::init()`
- `AgentRegistry` replaces it for all runtime lookups

### 7. Implementation Steps

1. **Add `children_index` to ProcessTable** — new field, update `insert()`, `remove()`, `children_of()`
2. **Move `metrics` into `AgentProcess`** — remove `metrics` DashMap from ProcessTable
3. **Rename `name_index` to `name_registry`** — change to `DashMap<String, Vec<AgentId>>`, remove from routing
4. **Create `AgentRegistry`** — new module in kernel with builtin/custom split + YAML persistence
5. **Rewire routing in `event_loop.rs`** — implement 3-path routing state machine
6. **Replace `ManifestLoader` references** — swap to `AgentRegistry` in KernelInner, boot, tests
7. **Update tests** — adapt existing tests + add new tests for children_index, registry, routing paths

## References

- [AutoGen Agent Identity and Lifecycle](https://microsoft.github.io/autogen/stable/user-guide/core-user-guide/core-concepts/agent-identity-and-lifecycle.html) — lazy instantiation
- [Anthropic Multi-Agent Research System](https://www.anthropic.com/engineering/multi-agent-research-system) — spawn-new pattern
- [A2A Protocol Specification](https://a2a-protocol.org/latest/specification/) — explicit error on terminal tasks
