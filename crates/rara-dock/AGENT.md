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
├── tools.rs    — 9 AgentTool implementations (block/fact/annotation CRUD)
├── routes.rs   — Axum HTTP handlers for dock API (/api/dock/*)
└── lib.rs      — Public re-exports
```

**Data flow (agent turn):**
1. Frontend POST `/api/dock/turn` with current canvas state (blocks, facts, annotations)
2. `state.rs` builds system prompt (dock context + facts) and user prompt (content + canvas + annotations)
3. Kernel runs agent turn; agent calls dock tools (`dock.block.add`, `dock.fact.update`, etc.)
4. Tools return `DockMutation` as JSON via `ToolOutput` — they do NOT directly persist
5. Turn handler collects mutations, applies to `DockSessionStore`, writes tape anchor with snapshot
6. Returns `DockTurnResponse` with mutations + authoritative state

**Data flow (human edit):**
1. Frontend POST `/api/dock/sessions/{id}/mutate` with mutation batch
2. `store.rs` applies mutations directly to `document.json` — no kernel involvement

**Storage layout:**
```
~/.config/rara/dock/
├── workspace.json              — { active_session_id }
└── sessions/{id}/document.json — { session, blocks, annotations, facts }
```

## Critical Invariants

- **Session IDs must be filesystem-safe** — `store.rs` validates with `validate_session_id()`. Path traversal (`..`, `/`) is rejected. Violation allows arbitrary file writes.
- **Mutations are the only write path** — all state changes go through `DockMutation` enum. Direct field assignment bypasses merge semantics and can lose data.
- **Update mutations use merge semantics** — `BlockUpdate` and `AnnotationUpdate` preserve existing field values when the mutation's field is empty/default. Replacing the entire object loses metadata (block_type, diff, selection, anchor_y).
- **Tools do NOT persist** — dock tools return mutations as JSON. Only the turn handler and mutate route persist to disk.

## What NOT To Do

- Do NOT add kernel or LLM dependencies to this crate beyond the `AgentTool` trait — keep dock logic self-contained.
- Do NOT store blocks/facts/annotations in tape — tape only holds anchors with canvas snapshots for history. Authoritative state lives in `.dock/` files.
- Do NOT apply mutations by replacing objects wholesale — always merge, preserving fields the mutation doesn't explicitly set.
- Do NOT render raw block HTML without sanitization — frontend must strip scripts, event handlers, and javascript: URLs.

## Dependencies

- **Upstream**: `rara-kernel` (for `AgentTool` trait only), `axum`, `serde`, `snafu`
- **Downstream**: `rara-app` (registers tools), `rara-server` (mounts routes), `rara-channels` (WebEvent::DockTurnComplete)
