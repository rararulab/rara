# stt — Agent Guidelines

## Purpose
HTTP client for OpenAI-compatible Speech-to-Text servers (e.g. whisper.cpp `whisper-server`).

## Architecture
- `config.rs` — `SttConfig` (base_url required, model + language optional)
- `service.rs` — `SttService` sends multipart POST to `/v1/audio/transcriptions`

## Critical Invariants
- When `stt` config section is present, `base_url` MUST be non-empty — startup panics otherwise.
- Runtime STT failures (server down, empty response) degrade silently — voice messages are skipped, not queued.
- Audio bytes are not persisted — transcribed text is the only artifact stored in tape.

## What NOT To Do
- Do NOT add retry logic in SttService — transient failures skip the message, user can resend.
- Do NOT store audio files on disk — transcribe and discard.
- Do NOT add ContentBlock::Audio to the kernel — voice is transcribed to plain text.

## Dependencies
- Upstream: reqwest (HTTP client)
- Downstream: `rara-channels` (Telegram adapter consumes SttService)
