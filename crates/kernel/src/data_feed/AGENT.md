# data_feed — Agent Guidelines

## Purpose

External data ingestion subsystem: receives events from webhooks, WebSocket streams, and polling sources, persists them, and dispatches to subscribing agent sessions.

## Architecture

- `event.rs` — `FeedEvent` struct (the atomic event envelope) and `FeedEventId` (strongly-typed UUID).
- `store.rs` — `FeedStore` async trait for persistence and `FeedFilter` for querying.
- `config.rs` — `DataFeedConfig` (persisted feed registration) and `FeedType` enum (Webhook/WebSocket/Polling).
- `feed.rs` — `DataFeed` trait: the abstraction each transport implements (`name`, `tags`, `run`).
- `registry.rs` — `DataFeedRegistry`: in-memory CRUD for feed configs + cancellation token tracking for running tasks.
- `webhook.rs` — `WebhookState`, `webhook_handler` (axum POST handler), HMAC-SHA256 verification, idempotency cache, `webhook_routes` for server registration, `WebhookConfig` for per-feed webhook settings.
- `mod.rs` — Re-exports only; no logic.

Data flow: Transport layer (webhook/WS/polling) -> `FeedEvent` -> `FeedStore::append` -> subscription dispatch -> agent session.

Webhook flow: External POST -> `/api/v1/webhooks/{feed_name}` -> `webhook_handler` -> registry lookup -> HMAC verify -> dedup check -> `FeedEvent` -> `event_tx` channel -> kernel.

Registry flow: Caller loads configs from settings -> `DataFeedRegistry::restore` -> runtime `register`/`remove` -> caller persists via `configs()`.

## Critical Invariants

- `FeedStore::append` MUST be idempotent on `event.id` — duplicate inserts must not create duplicate rows. Violation causes double-processing by subscribers.
- `FeedEventId` uses `base::define_id!` (UUID v4, NonZeroU128) — never construct from arbitrary u128 without `from_uuid`.
- Read cursors are per-subscriber per-source — do not conflate subscribers.
- `DataFeedRegistry` does NOT own settings persistence — callers must read `configs()` and write to `SettingsProvider` after mutations. The registry is pure in-memory state.
- Feed names are unique within the registry — `register` rejects duplicates.
- Webhook HMAC verification uses constant-time comparison (`subtle::ConstantTimeEq`) — never use `==` for signature comparison. Violation enables timing attacks.
- Webhook idempotency cache is in-memory with 1h TTL — process restarts reset it. This is acceptable because `FeedStore::append` is also idempotent on `event.id`.

## What NOT To Do

- Do NOT add `Default` to `FeedFilter` with hardcoded limit — limit must be caller-specified or config-driven.
- Do NOT store `FeedEvent::payload` as typed structs — it is intentionally `serde_json::Value` to support heterogeneous sources.
- Do NOT call `SettingsProvider` from within `DataFeedRegistry` — persistence is the caller's responsibility to keep the registry sync-only and testable without async.
- Do NOT hold `parking_lot::Mutex` guards across `.cancel()` or `tracing` calls — extract values from the lock first, then act on them.
- Do NOT use `==` for HMAC signature comparison in webhook.rs — use `subtle::ConstantTimeEq` to prevent timing attacks.
- Do NOT add webhook-specific config fields to `DataFeedConfig` — webhook settings go in `WebhookConfig` and are stored inside `DataFeedConfig::config` as JSON.

## Dependencies

- Upstream: `base` (for `define_id!` macro), `jiff` (timestamps), `crate::session` (for `SessionKey`), `parking_lot`, `tokio_util` (CancellationToken), `axum` (webhook handler types), `hmac`/`sha2`/`subtle`/`hex` (signature verification).
- Downstream: `query-feed` tool, kernel startup (restore), server route registration (via `webhook_routes`).
- DB: `feed_events` + `feed_read_cursors` tables (migration in `crates/rara-model/migrations/`).
- Settings: feed configs persisted via `SettingsProvider` KV store (key: `data_feeds.configs`).
