# rara-browser — Agent Guidelines

## Purpose

Driver crate providing a Lightpanda (Zig headless browser) CDP client. Exposes `BrowserManager` and snapshot/ref-map types so kernel browser tools can drive a headless browser without pulling chromiumoxide into `rara-kernel`.

## Architecture

```
browser/
  mod.rs         — re-exports
  manager.rs     — BrowserManager: Lightpanda process lifecycle + CDP connection
  snapshot.rs    — Accessibility tree extraction + ref numbering
  ref_map.rs     — ref_id ↔ CDP BackendNodeId mapping
  error.rs       — BrowserError (snafu)

tool/browser/
  mod.rs         — browser_tools() registration function
  navigate.rs    — browser-navigate
  snapshot.rs    — browser-snapshot
  click.rs       — browser-click
  type_text.rs   — browser-type
  evaluate.rs    — browser-evaluate
  ... (17 tools total)
```

**Data flow**: Agent calls tool → tool calls `BrowserManager` method → manager sends CDP command via `chromiumoxide` → Lightpanda processes → result returned as accessibility snapshot text.

**Snapshot format**: Indented accessibility tree with `[ref=N]` markers on interactive elements. ~1-5KB per page. Refs are rebuilt on every snapshot — old refs are invalid after a new snapshot.

## Critical Invariants

- **RefMap is per-tab and rebuilt on every snapshot.** Never cache refs across snapshots. A stale ref will return `BrowserError::RefNotFound`.
- **Single lock for all tab state.** `TabStore` (tabs + active pointer) lives behind one `RwLock` to prevent deadlocks from split locks. Never introduce a second lock for tab-related state.
- **Tabs use `IndexMap` for stable ordering.** Numeric indices are insertion-ordered. Do NOT switch to `HashMap`.
- **`navigate()` reuses the active tab** via `Page::goto()` to preserve browser history. New tabs are only created when no active tab exists.
- **`close_tab()` / `close_all()` call `Page::close()`** to release CDP targets in Lightpanda. Never just remove from the map without closing.
- **Lightpanda process is `kill_on_drop(true)`.** If BrowserManager is dropped, the subprocess is killed.
- **chromiumoxide handler task must stay alive.** The `_handler: JoinHandle` field exists solely to keep the CDP event processing loop running. Never remove it.

## What NOT To Do

- Do NOT hold the `TabStore` `RwLock` guard across `.await` points — causes deadlocks. Clone `Page` handles out of the lock first, then do async I/O.
- Do NOT introduce a second lock for tab-related state — the single-lock design exists to prevent lock-ordering deadlocks.
- Do NOT index into `Quad` directly — use `.inner()` to get the underlying `Vec<f64>`.
- Do NOT use `chromiumoxide_cdp` directly — use `chromiumoxide::cdp` re-exports instead.
- Do NOT assume CDP builder `.build()` always returns `Result` — DOM builders return the struct directly, Input builders return `Result<Params, String>`.
- Do NOT add screenshot/PDF tools — Lightpanda has no rendering engine. Use the existing Playwright-based `screenshot` tool.

## Dependencies

- **Upstream**: `chromiumoxide` (CDP client), `chromiumoxide_cdp` (CDP types, re-exported)
- **External**: `lightpanda` binary must be installed on the host system
- **Downstream**: `rara-app` boot.rs creates BrowserManager and registers tools into ToolRegistry
- **Config**: `BrowserConfig` in `config.yaml` under `browser:` key (optional — if absent, browser tools are not registered)
