# stt — Agent Guidelines

## Purpose
HTTP client for OpenAI-compatible Speech-to-Text servers (e.g. whisper.cpp `whisper-server`), with optional managed child-process supervision.

## Architecture
- `config.rs` — `SttConfig` (base_url required, model + language optional, managed-mode fields)
- `service.rs` — `SttService` sends multipart POST to `/v1/audio/transcriptions`
- `process.rs` — `WhisperProcess` child-process supervisor (spawn, health-check, restart, graceful stop)

## Critical Invariants
- When `stt` config section is present, `base_url` MUST be non-empty — startup panics otherwise.
- Runtime STT failures (server down, empty response) degrade silently — voice messages are skipped, not queued.
- Audio bytes are not persisted — transcribed text is the only artifact stored in tape.
- `WhisperProcess` uses `kill_on_drop(true)` — if rara panics, the child is still cleaned up.
- Supervisor restarts with a fixed 2s delay; no exponential backoff (whisper-server either works or config is wrong).

## What NOT To Do
- Do NOT add retry logic in SttService — transient failures skip the message, user can resend.
- Do NOT store audio files on disk — transcribe and discard.
- Do NOT add ContentBlock::Audio to the kernel — voice is transcribed to plain text.
- Do NOT add exponential backoff to supervisor restart — a simple delay is sufficient; persistent failures are logged and voice messages degrade silently.

## Dependencies
- Upstream: reqwest (HTTP client), url (URL parsing), tokio-util (CancellationToken)
- Downstream: `rara-channels` (Telegram adapter consumes SttService), `rara-app` (wires WhisperProcess into startup)
