# rara-dock — Agent Guidelines

## Purpose
Backend foundation for the Dock generative UI — a collaborative canvas workbench where agents mutate structured blocks via tools and humans provide annotations and facts.

## Architecture

```
src/
├── models.rs   — Data types: Block, Fact, Annotation, Mutation, Session, API payloads
├── error.rs    — snafu-based DockError enum
├── store.rs    — DockSessionStore: file-based persistence at ~/.config/rara/dock/
├── state.rs    — Prompt building (<dock_context>, <dock_canvas>), mutation application
├── tools.rs    — 9 AgentTool impls + DockMutationSink (shared mutation channel)
├── routes.rs   — Axum HTTP handlers: SSE streaming turn, session CRUD, history
└── lib.rs      — Public re-exports
```

**Data flow (agent turn):**
1. Frontend POST `/api/dock/turn` with current canvas state (blocks, facts, annotations)
2. `routes.rs` ensures a kernel session exists (deterministic UUID from dock session ID)
3. `state.rs` builds system prompt (dock context + facts) and user prompt (content + canvas + annotations)
4. Kernel runs agent turn via `ingest()`; handler returns SSE stream
5. Agent calls dock tools (`dock.block.add`, `dock.fact.update`, etc.)
6. Tools push full `DockMutation` into shared `DockMutationSink` keyed by `SessionKey`
7. Tools return a compact `{ok, op, id}` confirmation to the LLM (not the full mutation)
8. When stream closes, handler drains sink, applies mutations to store, writes tape anchor
9. Emits `dock_turn_complete` SSE event with authoritative state + `selected_anchor: null`

**Data flow (human edit):**
1. Frontend POST `/api/dock/sessions/{id}/mutate` with mutation batch
2. `store.rs` applies mutations directly to `document.json` — no kernel involvement

**SSE events emitted during a turn:**
- `text_delta` — incremental LLM text
- `tool_call_start` — tool name + id + arguments
- `tool_call_end` — id + result_preview + success/error
- `dock_turn_complete` — authoritative blocks/facts/annotations/history + selected_anchor
- `error` — error message
- `done` — stream end

**Storage layout:**
```
~/.config/rara/dock/
├── workspace.json              — { active_session_id }
└── sessions/{id}/document.json — { session, blocks, annotations, facts }
```

**History & snapshots:**
- Each turn writes a tape anchor (`dock/turn/{epoch_ms}`) with `DockCanvasSnapshot` (blocks + facts)
- `selected_anchor` parameter on GET session restores historical canvas state from the anchor
- Anchors store ISO 8601 timestamps in `extra.timestamp`

## Critical Invariants

- **Session IDs must be filesystem-safe** — `store.rs` validates with `validate_session_id()`. Path traversal (`..`, `/`) is rejected. Violation allows arbitrary file writes.
- **Mutations are the only write path** — all state changes go through `DockMutation` enum. Direct field assignment bypasses merge semantics and can lose data.
- **Update mutations use merge semantics** — `BlockUpdate` and `AnnotationUpdate` preserve existing field values when the mutation's field is empty/default.
- **Tools push to sink, not to store** — dock tools push mutations into `DockMutationSink` during `execute()`. Only the turn handler and mutate route persist to disk via `store.apply_mutations()`.
- **Kernel SessionKey is deterministic** — derived via `Uuid::new_v5(NAMESPACE_OID, dock_session_id.as_bytes())` so the turn handler can predict the key and subscribe to streams.
- **Turn completion resets anchor selection** — `dock_turn_complete` always includes `selected_anchor: null` to exit history-viewing mode.
- **Frontend must sanitize block HTML** — uses DOMPurify with explicit tag/attribute allowlist (not regex).

## What NOT To Do

- Do NOT add kernel or LLM dependencies to this crate beyond the `AgentTool` trait — keep dock logic self-contained.
- Do NOT store blocks/facts/annotations in tape — tape only holds anchors with canvas snapshots for history. Authoritative state lives in `.dock/` files.
- Do NOT apply mutations by replacing objects wholesale — always merge, preserving fields the mutation doesn't explicitly set.
- Do NOT parse dock mutations from `result_preview` — it is truncated to 2048 bytes. Use `DockMutationSink` instead.
- Do NOT render raw block HTML without DOMPurify — agent-generated HTML is untrusted input.

## Dependencies

- **Upstream**: `rara-kernel` (for `AgentTool`, `SessionKey`, `TapeService`, `KernelHandle`), `axum`, `serde`, `snafu`, `chrono`
- **Downstream**: `rara-app` (registers tools + passes `DockMutationSink` via `ToolDeps`, creates `DockRouterState`)
