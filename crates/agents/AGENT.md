# rara-agents — Agent Guidelines

## Purpose

Predefined agent manifest registry — declares built-in agent personalities (rara, nana, worker, mita, scheduled_job) as static `AgentManifest` values ready for kernel registration.

## Architecture

### Key modules

- `src/lib.rs` — The entire crate. Contains `LazyLock<AgentManifest>` statics for each agent and their system prompts as `const &str`.

### Agents defined

| Function | Agent | Role | Description |
|---|---|---|---|
| `rara()` | rara | Chat | Main user-facing assistant with full tool access |
| `nana()` | nana | Chat | Friendly companion, chat-only (rara's sister) |
| `worker()` | worker | Worker | Lightweight sub-agent for task execution |
| `mita()` | mita | Worker | Background proactive agent with heartbeat observation |
| `scheduled_job(...)` | scheduled_job | Worker | Dynamically constructed per scheduled task |

### Key design decisions

- **Soul prompts are `None`** — the kernel loads soul files at runtime via `rara-soul`. Manifests only carry operational system prompts.
- **Tool lists are empty** (except mita) — `rara-app` boot injects tools into manifests via `ToolRegistry`. Mita declares its tools explicitly because it has a fixed, curated set.
- `scheduled_job()` is the only non-static manifest — it takes runtime parameters (job ID, schedule, message) to bake into the system prompt.

## Critical Invariants

- System prompts define agent behavior boundaries — changes must be tested with simple inputs ("hello", greeting) to verify no redundant/repeated output.
- Do NOT add "plan before acting" instructions to any system prompt — causes verbose narration on simple interactions (see issue #201).
- Do NOT add broad memory search triggers — must have explicit conditions to avoid unnecessary searches on every interaction.

## What NOT To Do

- Do NOT register tools directly in manifest `tools` vec (except mita) — tools are injected by `rara-app` boot.
- Do NOT add `soul_prompt` content here — soul prompts live in `rara-soul` and are loaded at runtime.
- Do NOT modify system prompts without testing with basic conversational inputs.

## Dependencies

**Upstream:** `rara-kernel` (for `AgentManifest`, `AgentRole`, `Priority` types).

**Downstream:** `rara-app` (loads manifests into the kernel's agent registry at boot).
