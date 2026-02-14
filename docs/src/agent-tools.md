# Agent Tools

This page explains how the agent uses the `workers::memory` crate in the agent loop.

## Memory Integration Flow

At runtime, memory is wired in `AppState::init`:

1. Build `MemoryManager` with the selected storage backend (`postgres` or `sqlite`).
2. Apply runtime memory settings from `SettingsSvc` (`agent.memory`).
3. Run an initial `sync()` to index markdown files.
4. Register `memory_search` and `memory_get` as service tools in the tool registry.
5. Pass the tool registry to `ChatService`, so the model can call memory tools during chat.

Relevant implementation:

- `crates/workers/src/worker_state.rs`
- `crates/workers/src/tools/services/memory_tools.rs`
- `crates/workers/src/memory/manager.rs`

## Data Source

The memory index scans markdown files under the configured memory directory:

- `MEMORY.md`
- `*.md` files in the same memory tree

The manager performs incremental sync based on file metadata and content hash.

## Agent-Facing Tools

### `memory_search`

Purpose:
- Retrieve relevant memory chunks for a user query.

Behavior:
- Refreshes runtime memory settings before each call.
- Runs `sync()` before searching.
- Uses hybrid retrieval when embeddings are enabled.
- Falls back to keyword search when embeddings are disabled or vector path is unavailable.

Input:

```json
{
  "query": "rust engineer tokyo",
  "limit": 8
}
```

Output (shape):

```json
{
  "query": "rust engineer tokyo",
  "storage_backend": "postgres",
  "vector_backend": "local|chroma|none",
  "count": 2,
  "results": [
    {
      "chunk_id": 101,
      "path": "MEMORY.md",
      "chunk_index": 0,
      "score": 0.82,
      "snippet": "..."
    }
  ]
}
```

### `memory_get`

Purpose:
- Fetch full chunk content by `chunk_id` returned from `memory_search`.

Input:

```json
{
  "chunk_id": 101
}
```

Output (shape):

```json
{
  "chunk_id": 101,
  "path": "MEMORY.md",
  "chunk_index": 0,
  "content": "full chunk text"
}
```

## Runtime Configuration (`/api/v1/settings`)

Memory behavior is controlled by `agent.memory` in runtime settings:

```json
{
  "agent": {
    "memory": {
      "storage_backend": "postgres",
      "embeddings_enabled": true,
      "chroma_enabled": true,
      "chroma_url": "http://localhost:8000",
      "chroma_collection": "job-memory",
      "chroma_api_key": ""
    }
  }
}
```

Field notes:

- `storage_backend`: `postgres` (recommended for this project) or `sqlite`.
- `embeddings_enabled`: enables hybrid retrieval path.
- `chroma_enabled`: enables remote vector retrieval via Chroma.
- `chroma_*`: Chroma connection settings.

Settings are hot-applied in tool execution, so updates take effect without restarting the agent process.

## Recommended Agent Loop Pattern

For memory-grounded responses, the model/tool loop should follow:

1. Call `memory_search` with the user request.
2. Select top chunk IDs.
3. Call `memory_get` for the selected chunks.
4. Answer using retrieved content as grounding context.

This pattern avoids losing context between turns and keeps responses tied to indexed memory documents.
