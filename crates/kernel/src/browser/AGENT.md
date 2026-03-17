# browser — Agent Guidelines

## Purpose

Kernel-level browser subsystem powered by Lightpanda (Zig headless browser) via CDP. Provides agent-accessible browser automation tools with accessibility tree snapshots for token-efficient page representation.

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
- **BrowserManager is shared across all agent sessions.** Tabs provide session isolation. The `RwLock<HashMap>` on tabs must not be held across await points.
- **Lightpanda process is `kill_on_drop(true)`.** If BrowserManager is dropped, the subprocess is killed.
- **chromiumoxide handler task must stay alive.** The `_handler: JoinHandle` field exists solely to keep the CDP event processing loop running. Never remove it.

## What NOT To Do

- Do NOT hold tab `RwLock` guards across `.await` — causes deadlocks.
- Do NOT index into `Quad` directly — use `.inner()` to get the underlying `Vec<f64>`.
- Do NOT use `chromiumoxide_cdp` directly — use `chromiumoxide::cdp` re-exports instead.
- Do NOT assume CDP builder `.build()` always returns `Result` — DOM builders return the struct directly, Input builders return `Result<Params, String>`.
- Do NOT add screenshot/PDF tools — Lightpanda has no rendering engine. Use the existing Playwright-based `screenshot` tool.

## Dependencies

- **Upstream**: `chromiumoxide` (CDP client), `chromiumoxide_cdp` (CDP types, re-exported)
- **External**: `lightpanda` binary must be installed on the host system
- **Downstream**: `rara-app` boot.rs creates BrowserManager and registers tools into ToolRegistry
- **Config**: `BrowserConfig` in `config.yaml` under `browser:` key (optional — if absent, browser tools are not registered)
