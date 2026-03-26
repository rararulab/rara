# setup — Agent Guidelines

## Purpose
Interactive CLI wizard that guides users through configuring rara's core dependencies.

## Architecture
- `mod.rs` — entry point, mode selection, orchestrates all steps
- `prompt.rs` — generic Q&A utilities (ask, ask_choice, confirm, etc.)
- `db.rs` — database URL + connection test + migration
- `llm.rs` — provider selection + API key + model + verification
- `telegram.rs` — bot token + chat_id + test message
- `user.rs` — user identity (name, role, platform mappings)
- `stt.rs` — optional whisper-server config + connectivity check
- `writer.rs` — YAML assembly, secret masking, backup + write

## Critical Invariants
- Each step validates immediately after input — never write unvalidated config.
- Existing config is always backed up before overwrite (`config.yaml.bak`).
- API keys are masked in preview output.
- FillMissing mode must never overwrite existing values.
- Setup only writes config.yaml — no runtime behavior changes.

## What NOT To Do
- Do NOT auto-discover services — only configure what the user explicitly provides.
- Do NOT install dependencies — only configure connection info.
- Do NOT modify runtime behavior — setup only writes config.yaml.
- Do NOT add new setup steps without the validate-then-retry pattern.
