# crates/app/src/tools — Agent Guidelines

## Purpose

Concrete `ToolExecute` implementations for every agent-callable tool. This
directory is the boundary between the kernel's tool subsystem and the
underlying primitives (filesystem, sandbox VM, browser, HTTP client, …).

## Architecture

Two execution surfaces, picked per tool by what the tool needs to do:

- **Host-side tools** — read-only inspection and metadata: `file_stats`,
  `discover`, `read_file`, `list_directory`, etc. Run in the rara process
  with no isolation. Workspace boundary is enforced by
  [`path_check::resolve_writable`](./path_check.rs) for any tool that
  takes a path argument and looks up content for the agent.
- **Sandbox-side tools** — write or exec: `bash`, `write_file`,
  `edit_file`, `multi_edit`, `create_directory`, `delete_file`,
  `run_code`. These go through the per-session boxlite microVM
  (`crates/rara-sandbox`). The VM mounts the host workspace at
  `/workspace` (read-write) and runs argv there; nothing outside the
  workspace bind-mount is reachable.

The asymmetry is deliberate: read-side tools dominate every turn and the
sandbox cold-start (~60 ms first call, plus per-exec overhead) would
swamp them, while write/exec tools are rarer and benefit from hardware
isolation.

## Critical Invariants

- **Path inputs to write-class tools MUST go through
  `path_check::resolve_writable`.** It rejects:
  - absolute paths outside `rara_paths::workspace_dir()`;
  - symlinks pointing at out-of-workspace targets (resolved via
    `tokio::fs::canonicalize`);
  - paths whose parent ancestry resolves outside the workspace.

  Tools that bypass this helper reintroduce the lexical-only bug fixed in
  #1936.
- **Path translation for sandbox tools** is one-way: host
  `<workspace>/x` → guest `/workspace/x`. Out-of-workspace paths hit a
  hard error rather than silently being routed through approval; the
  guest has no mount for them. See `bash.rs::translate_cwd`.
- **Network policy is fused, not per-call.** Sandbox-using tools share a
  single per-session VM, which carries a single network policy. The
  effective policy is the union (most-permissive) across all
  sandbox-using tools — see
  `crates/app/src/sandbox.rs::fused_network_policy` and
  `crates/rara-sandbox/AGENT.md` (Network policy fusion). Do not pass a
  per-call `NetworkPolicy` to `sandbox_for_session`.
- **`Sandbox` is single-owner; concurrent tool calls in the same session
  serialise on `Arc<tokio::Mutex<Sandbox>>`.** Tools must hold the lock
  for the whole exec; do not split into "lock — drop — re-lock to read
  output" because the underlying `LiteBox` is not assumed `Sync`.

## What NOT To Do

- Do NOT skip `path_check::resolve_writable` on a write-class tool —
  **why:** lexical-only checks miss symlink escapes (#1936). The check
  is cheap and uniform.
- Do NOT add a per-call `NetworkPolicy` argument to a sandbox tool —
  **why:** the per-session VM is created on the first call; subsequent
  calls reuse it, so a per-call argument is silently dropped on every
  cache hit. This is the leak the fusion rule was added to close (PR
  #1946 review).
- Do NOT write a host-side tool that mutates files outside the
  workspace — **why:** the agent's blast radius is the workspace; any
  exception widens it without a corresponding policy hook.
- Do NOT mix sandbox-side reads with host-side writes within one tool —
  **why:** half-isolated tools are surprising. Pick a side per tool.

## Dependencies

- `rara-kernel` — `ToolExecute`, `ToolContext`, lifecycle hooks.
- `rara-sandbox` — sandbox VM primitives (`Sandbox`, `ExecRequest`,
  `NetworkPolicy`, `VolumeMount`).
- `rara_paths` — workspace dir resolution; the source of truth for
  `path_check`.
- `rara-tool-macro` — `ToolDef` derive that wires JSON schemas into the
  tool registry.
