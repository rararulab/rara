# setup — Agent Guidelines

## Purpose
Interactive CLI wizard for configuring rara — database, LLM, Telegram, STT, and users. Also provides standalone subcommands: `rara setup whisper` (whisper.cpp install) and `rara setup boxlite` (sandbox runtime staging).

## Architecture
- `mod.rs` — `SetupCmd` with optional `SetupSub` subcommands (`Whisper`, `Boxlite`). Orchestrates the full wizard flow.
- `whisper_install.rs` — Automated whisper.cpp pipeline: detect existing binary → clone/build from source → download GGML model → start server → verify health + transcription → shutdown. Entry point: `ensure_whisper()`.
- `boxlite.rs` — Stages boxlite microVM runtime files (built by `cargo build -p rara-sandbox`) into the platform user-data dir so `rara-sandbox` can find them at runtime. Entry point: `run_boxlite_setup(check_only)`. See `docs/guides/boxlite-runtime.md` for the user-facing flow.
- `stt.rs` — STT config section for the full wizard (`setup_stt`) and standalone whisper entry point (`run_whisper_setup`).
- `writer.rs` — Config assembly and YAML serialization. `assemble_config()` merges all sections.
- `prompt.rs` — Interactive CLI helpers (ask, confirm, ask_choice, print_step/ok/err).
- `db.rs`, `llm.rs`, `telegram.rs`, `user.rs` — Individual config sections for the full wizard.

## Critical Invariants
- **Setup only writes config files** — it does NOT call settings API. Config syncs to settings automatically at app startup.
- **whisper-server must use `--inference-path /v1/audio/transcriptions`** — rara's `SttService` expects the OpenAI-compatible endpoint, not whisper.cpp's default `/inference`.
- **whisper-server must use `--convert`** — Telegram sends OGG/Opus voice files; without this flag whisper.cpp only accepts 16-bit 16kHz WAV.
- **ChildGuard pattern** — `test_server` wraps the child process in a drop guard to ensure cleanup on any exit path (early error, panic).
- **Model paths use `OsStr`** — never convert paths through `to_string_lossy()` for command arguments; use `.arg(path.as_os_str())`.
- Existing config is always backed up before overwrite (`config.yaml.bak`).
- API keys are masked in preview output.
- FillMissing mode must never overwrite existing values.

## What NOT To Do
- Do NOT add settings API calls to setup — setup only writes `~/.config/rara/config.yaml`.
- Do NOT ask the user for OS/arch — detect automatically from the machine.
- Do NOT use port 8080 as default — use 8178 to avoid collisions with common dev servers.
- Do NOT spawn long-running downloads on the tokio runtime thread — use `spawn_blocking`.
- Do NOT convert `Path` to `String` for command args — use `.as_os_str()` to preserve non-UTF-8 paths.

## Dependencies
- **Upstream**: `rara_paths` (config/data dir paths), `rara_app::AppConfig` (existing config loading)
- **External**: `reqwest` (health checks, test transcription), `serde_yaml` (config read/write), `serde_json` (response parsing)
- **Build-time**: whisper.cpp requires `cmake`, `make`, `git` on the host machine
