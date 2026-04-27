# rara-app — Agent Guidelines

## Purpose

Application orchestration crate that wires all subsystems together, boots the kernel, starts HTTP/gRPC servers, and manages the full application lifecycle.

## Architecture

### Key modules

- `src/lib.rs` — `AppConfig` (YAML-backed), `run()` / `start()` / `start_with_options()` entry points, `AppHandle` for lifecycle control.
- `src/boot.rs` — `boot()` function that resolves users, builds registries (tool, agent, driver, skill, MCP), session index, tape service, and identity resolver from the database pool + settings.
- `src/tools/` — All application-level tool implementations (bash, file ops, mita proactive tools, email, screenshots, MCP, composio, etc.). Each file is a single tool.
- `src/config_sync.rs` — Bidirectional sync between YAML config file and runtime settings store.
- `src/gateway.rs` — Gateway supervisor for auto-update and health monitoring.
- `src/flatten.rs` — Flattened config structs for seeding settings store from YAML.

### Data flow

1. `AppConfig::new()` loads YAML from `rara_paths::config_file()` + local `./config.yaml`.
2. `init_infra()` opens SQLite via `yunara-store`, runs migrations from `crates/rara-model/migrations/`.
3. `boot()` creates all kernel dependencies (registries, session index, tape service).
4. `Kernel::new()` + `kernel.start()` launches the agent loop.
5. Channel adapters (Telegram, Web, CLI) are registered with `IOSubsystem`.
6. HTTP and gRPC servers start, then `READY` is printed to stdout for the gateway.

### Public API

- `AppConfig` — static config, deserialized from YAML.
- `run(config)` — blocking entry point.
- `start(config)` / `start_with_options(config, options)` — returns `AppHandle`.
- `AppHandle` — shutdown control, access to `KernelHandle` and command handlers.

## Critical Invariants

- Config must come from YAML files, never hardcoded defaults in Rust. Missing required fields cause a startup error.
- `rustls::crypto::ring::default_provider().install_default()` must be called in `main()` before this crate does any TLS work.
- Migrations run from `crates/rara-model/migrations/` via `diesel_migrations::embed_migrations!` — never modify applied migrations.
- `KernelHandle` is injected into `DispatchRaraTool` and `ListSessionsTool` after kernel start via `RwLock` slots — these tools will panic if invoked before wiring completes.
- Mita-exclusive tools: `dispatch-rara`, `list-sessions`, `read-tape`, `write-user-note`, `distill-user-notes`, `update-soul-state`, `evolve-soul`, `update-session-title`, `write-skill-draft`. These are declared in Mita's manifest (`rara-agents`) and must not be added to Rara's tool set.
- `run_code` (sandboxed code execution) is wired to a per-session boxlite microVM. The first call in a session creates the VM, subsequent calls reuse it, and `SandboxCleanupHook` (registered in `start_with_options`) destroys it via `LifecycleHook::on_session_end` when the kernel removes the session. The default rootfs image is required via the YAML `sandbox.default_rootfs_image` key — there is no Rust fallback. Threat model: hardware-isolated execution (Hypervisor.framework on macOS, KVM on Linux). Network egress is currently UNRESTRICTED inside the VM and there are no resource limits beyond boxlite's own defaults — both are documented as out-of-scope for #1700 and #1696.

## Config schema discipline

Adding a top-level `AppConfig` field WITHOUT `#[serde(default)]` is a breaking change for every deployed `config.yaml`. The commit subject MUST use `feat!:` / `fix!:` / `refactor!:` and the PR body MUST document the migration step in operator-readable form. Default behaviour for new fields: provide a `Default` impl or a `#[serde(default = "fn")]` so old configs keep booting. Genuinely required fields (no safe default — auth secrets, identity references) keep no `#[serde(default)]` but MUST carry a `// REQUIRED: <one-line why>` comment so the next agent knows the omission is intentional.

## What NOT To Do

- Do NOT hardcode database URLs or config defaults — use the YAML config file and `rara_paths`.
- Do NOT add tools directly to agent manifests here — tools are registered via `ToolRegistry` in `boot.rs`; agent manifests declare tool names, not implementations.
- Do NOT bypass `IOSubsystem` for sending messages to users — all user-facing output goes through channel adapters.
- Do NOT modify `AppConfig` fields at runtime — use `SettingsSvc` for mutable settings.

## Dependencies

**Upstream (depends on):** `rara-kernel`, `rara-server`, `rara-channels`, `rara-sessions`, `rara-agents`, `rara-soul`, `rara-skills`, `rara-dock`, `rara-paths`, `rara-mcp`, `rara-model` (migrations), `yunara-store`, `rara-backend-admin`.

**Downstream (depended on by):** `crates/cmd` (the binary entry point).

**External services:** SQLite (via diesel-async + bb8), Telegram Bot API, OpenRouter/LLM providers, Composio, MCP servers.
