# rara-tts — Agent Guidelines

## Purpose
OpenAI-compatible Text-to-Speech HTTP client that sends JSON to `/v1/audio/speech` and returns raw audio bytes.

## Architecture
- `config.rs` — `TtsConfig` (base_url + model + voice + format required; api_key, speed, timeout, max_text_length optional)
- `error.rs` — `TtsError` via `snafu` with `TextTooLong`, `Http`, `ServerError` variants
- `service.rs` — `TtsService::from_config()` builds a reqwest client; `synthesize()` / `synthesize_with_voice()` POST JSON and return `AudioOutput { data, mime_type }`

## Critical Invariants
- No hardcoded defaults in Rust — all config values come from YAML.
- `TtsService` is a concrete struct, not a trait impl. No dynamic dispatch.
- `api_key` is sent as `Authorization: Bearer` only when `Some`.

## What NOT To Do
- Do NOT add a `Tts` trait or provider abstraction — YAGNI until a second backend exists.
- Do NOT add ElevenLabs, fallback chains, or retry logic.
- Do NOT persist audio output to disk — callers decide storage policy.
- Do NOT wire into app startup here — that belongs in `rara-app` (#1163).

## Dependencies
- Upstream: reqwest (HTTP), serde/serde_json (serialization), snafu (errors), bon (builder)
- Downstream: `rara-channels` and `rara-app` will consume this crate (future PRs)
