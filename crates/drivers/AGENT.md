# drivers — Agent Guidelines

## Purpose

Container directory for peripheral driver crates that the kernel depends on but
that are NOT part of core kernel responsibilities (heartbeat, event bus, agent
loop, memory, tools, guard). Driver crates own external integrations that
have heavy dependency closures (chromiumoxide, reqwest+url+tokio_util child
supervisors, etc.) and would otherwise bloat `rara-kernel`'s rebuild surface.

## Architecture

Each subdirectory is its own workspace member crate:

- `browser/` — `rara-browser`: Lightpanda CDP client (`BrowserManager`,
  accessibility-tree snapshot, ref map). Used by `crates/kernel/src/tool/browser/*`
  and wired in `crates/app/src/lib.rs`.
- `stt/` — `rara-stt`: OpenAI-compatible whisper-server HTTP client and an
  optional managed child-process supervisor. Used by `rara-channels`
  (Telegram + web voice messages) and bootstrapped from `rara-app`.

Each crate carries its own `AGENT.md` with subsystem-specific invariants.

## Critical Invariants

- Driver crates MUST NOT depend on `rara-kernel`. The dependency direction is
  `rara-kernel -> rara-{browser,stt}` and `rara-app -> rara-{browser,stt}`.
  A reverse edge would create a cycle and defeat the rebuild-surface goal.
- Driver crates MUST NOT reach into kernel-private types (`crate::llm::Message`,
  `crate::tool::*`, etc.). If a driver needs to exchange a kernel type, lift the
  type into `rara-domain-shared` first.
- Each new driver crate MUST be added to `Cargo.toml` workspace members AND
  registered as a workspace dep alias (`rara-{name} = { path = "..." }`).

## What NOT To Do

- Do NOT put kernel-internal logic (agent loop, tape, guard, security) here —
  drivers are leaf crates that wrap an external service or binary, nothing more.
- Do NOT add a tool implementation here — tool wrappers stay in
  `crates/kernel/src/tool/<name>/*` so `tool_names::*` constants and Guard
  tool-name matches remain in one place.
- Do NOT collapse multiple drivers into one crate — the whole point is finer
  rebuild granularity.

## Dependencies

Upstream: external crates only (chromiumoxide, reqwest, tokio, etc.).
Downstream: `rara-kernel` (for tool wrappers), `rara-app` (for boot wiring),
`rara-channels` (for STT consumers).
