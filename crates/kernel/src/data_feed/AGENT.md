# data_feed — Agent Guidelines

## Purpose

External data ingestion subsystem: receives events from webhooks, WebSocket streams, and polling sources, persists them, and dispatches to subscribing agent sessions.

## Architecture

- `event.rs` — `FeedEvent` struct (the atomic event envelope) and `FeedEventId` (strongly-typed UUID).
- `store.rs` — `FeedStore` async trait for persistence, `FeedFilter` for querying, `FeedStoreRef` type alias.
- `config.rs` — `DataFeedConfig` (persisted feed registration stored in `data_feeds` table), `FeedType` enum (Webhook/WebSocket/Polling), `FeedStatus` enum (Idle/Running/Error), `AuthConfig` tagged enum (Header/Query/Bearer/Basic/Hmac).
- `feed.rs` — `DataFeed` trait: the abstraction each transport implements (`name`, `tags`, `run`).
- `registry.rs` — `DataFeedRegistry`: in-memory CRUD for feed configs + cancellation token tracking for running tasks. Callers are responsible for syncing mutations to the `data_feeds` table.
- `webhook.rs` — `WebhookState`, `webhook_handler` (axum POST handler), HMAC-SHA256 verification via `AuthConfig::Hmac`, idempotency cache, `webhook_routes` for server registration, `WebhookTransport` for per-feed webhook settings (event filtering, body size limit).
- `polling.rs` — `PollingSource`: config-driven HTTP polling feed. Created from `DataFeedConfig` via `from_config()`. Raw response body is passed through as `FeedEvent.payload` — no response parsing. `PollingTransport` struct for transport-specific config. `apply_request_auth()` handles outbound HTTP auth (header/bearer/basic); query-param auth is applied at URL construction time.
- `mod.rs` — Re-exports only; no logic.

Data flow: Transport layer (webhook/WS/polling) -> `FeedEvent` -> `FeedStore::append` -> subscription dispatch -> agent session.

Webhook flow: External POST -> `/api/v1/webhooks/{feed_name}` -> `webhook_handler` -> registry lookup -> HMAC verify (via `AuthConfig::Hmac`) -> dedup check -> `FeedEvent` -> `event_tx` channel -> kernel.

Polling flow: `PollingSource::run` loop -> `tokio::select!` (cancel vs sleep) -> HTTP GET (with auth from `AuthConfig`) -> raw response body -> `FeedEvent` with `event_type = "poll_response"` -> `event_tx` channel.

Registry flow: Caller loads configs from `data_feeds` table -> `DataFeedRegistry::restore` -> runtime `register`/`remove` -> caller syncs to DB.

Config persistence: Feed configurations are stored in the `data_feeds` table (not settings KV). Transport-specific config is a JSON blob in the `transport` column. Auth config is a JSON blob in the `auth` column. See `crates/rara-model/migrations/` for the schema.

## Agent-Facing API

### query-feed tool (`tool/data_feed.rs`)

Read-only, deferred-tier tool for querying historical feed events.
- Params: `source` (optional), `tags` (optional array), `since` (optional, e.g. "1h", "24h", "7d"), `limit` (optional, default 20, max 100).
- Returns: `{ events: [...], count: N }`.
- Requires `FeedStoreRef` — registered conditionally in `GetToolRegistry` handler when feed store is configured.

### Kernel syscall actions (in `syscall.rs`)

Data feed management via the `kernel` tool:
- `register_data_feed` — register a new feed config. Params: `feed` (DataFeedConfig JSON).
- `subscribe_data_feed` — subscribe current session to a feed's tags. Params: `source_name`.
- `unsubscribe_data_feed` — note unsubscription intent. Params: `source_name`.
- `list_data_feeds` — list all registered feeds with running status, enabled flag, and feed status.
- `remove_data_feed` — remove a feed (cancels running task). Params: `name`.

These operate directly on `DataFeedRegistry` via `KernelHandle` — no event queue round-trip needed since registry methods are synchronous.

## Critical Invariants

- `FeedStore::append` MUST be idempotent on `event.id` — duplicate inserts must not create duplicate rows. Violation causes double-processing by subscribers.
- `FeedEventId` uses `base::define_id!` (UUID v4, NonZeroU128) — never construct from arbitrary u128 without `from_uuid`.
- Read cursors are per-subscriber per-source — do not conflate subscribers.
- `DataFeedRegistry` does NOT own database persistence — callers must sync changes to the `data_feeds` table after mutations. The registry is pure in-memory state.
- Feed names are unique within the registry — `register` rejects duplicates.
- Webhook HMAC verification uses constant-time comparison (`subtle::ConstantTimeEq`) — never use `==` for signature comparison. Violation enables timing attacks.
- HMAC secret and header are configured via `DataFeedConfig.auth` (`AuthConfig::Hmac`), not in the transport config.
- Webhook idempotency cache is in-memory with 1h TTL — process restarts reset it. This is acceptable because `FeedStore::append` is also idempotent on `event.id`.
- `KernelHandle` fields `feed_registry` and `feed_store` are `Option` — both are `None` until the data feed subsystem is fully wired into kernel bootstrap.
- `PollingSource` must gracefully handle transient HTTP errors — log and continue, never crash the poll loop.
- Response parsing is intentionally absent — raw JSON response body is passed through as `FeedEvent.payload`. The subscribing agent interprets the payload.

## What NOT To Do

- Do NOT add `Default` to `FeedFilter` with hardcoded limit — limit must be caller-specified or config-driven.
- Do NOT store `FeedEvent::payload` as typed structs — it is intentionally `serde_json::Value` to support heterogeneous sources.
- Do NOT call database/settings from within `DataFeedRegistry` — persistence is the caller's responsibility to keep the registry sync-only and testable without async.
- Do NOT hold `parking_lot::Mutex` guards across `.cancel()` or `tracing` calls — extract values from the lock first, then act on them.
- Do NOT use `==` for HMAC signature comparison in webhook.rs — use `subtle::ConstantTimeEq` to prevent timing attacks.
- Do NOT add webhook-specific auth fields to `WebhookTransport` — HMAC secret goes in `DataFeedConfig.auth` as `AuthConfig::Hmac`.
- Do NOT add new `Syscall` enum variants for data feed operations — they operate directly on `DataFeedRegistry` (synchronous) via `KernelHandle`, not through the event queue.
- Do NOT add response parsing logic — the design decision is pass-through. Agent intelligence handles interpretation.
- Do NOT add source-specific feed implementations (e.g. yahoo.rs) — feeds are config-driven at runtime, not hardcoded in Rust.

## Dependencies

- Upstream: `base` (for `define_id!` macro), `jiff` (timestamps), `crate::session` (for `SessionKey`), `parking_lot`, `tokio_util` (CancellationToken), `axum` (webhook handler types), `hmac`/`sha2`/`subtle`/`hex` (signature verification), `reqwest` (polling HTTP client).
- Downstream: `query-feed` tool (in `tool/data_feed.rs`), kernel syscall actions (in `syscall.rs`), kernel startup (restore), server route registration (via `webhook_routes`).
- DB: `feed_events` + `feed_read_cursors` + `data_feeds` tables (migrations in `crates/rara-model/migrations/`).
