# rara-stt — Agent Guidelines

## Purpose
HTTP client for OpenAI-compatible Speech-to-Text servers (e.g. whisper.cpp `whisper-server`), with optional managed child-process supervision.

## Architecture
- `config.rs` — `SttConfig` (base_url required, model + language optional, managed-mode fields)
- `error.rs` — `SttError` enum (Http, ServerError, Parse, EmptyResponse) with `is_transient()` helper; per-crate `Result<T>` alias
- `service.rs` — `SttService::transcribe()` sends multipart POST to `/v1/audio/transcriptions`; retries once (2 s delay) on transient failures (timeout, 429, 5xx)
- `process.rs` — `WhisperProcess` child-process supervisor (spawn, health-check, restart, graceful stop)

## Critical Invariants
- When `stt` config section is present, `base_url` MUST be non-empty — startup panics otherwise.
- `SttService::transcribe` returns typed `SttError`; callers must send a user-visible placeholder on failure, never silently skip.
- Audio bytes are not persisted — transcribed text is the only artifact stored in tape.
- `WhisperProcess` uses `kill_on_drop(true)` — if rara panics, the child is still cleaned up.
- Supervisor restarts with a fixed 2s delay; no exponential backoff (whisper-server either works or config is wrong).

## What NOT To Do
- Do NOT add retry logic in channel adapters — retry lives inside `SttService::transcribe`.
- Do NOT use `anyhow` in `service.rs` or `error.rs` — these are domain-facing; `snafu` typed errors only. `process.rs` uses `anyhow` as an application boundary, which is fine.
- Do NOT silently skip voice messages on transcription failure — callers must send a user-visible placeholder.
- Do NOT store audio files on disk — transcribe and discard.
- Do NOT add ContentBlock::Audio to the kernel — voice is transcribed to plain text.
- Do NOT add exponential backoff to supervisor restart — a simple delay is sufficient.

## Dependencies
- Upstream: reqwest (HTTP client), snafu (typed errors), url (URL parsing), tokio (timer for retry + process supervision), tokio-util (CancellationToken)
- Downstream: `rara-channels` (Telegram adapter + web adapter consume `SttService` + `SttError`), `rara-app` (wires WhisperProcess into startup)
