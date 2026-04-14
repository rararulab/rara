# data_feed — Agent Guidelines

## Purpose

External data ingestion subsystem: receives events from webhooks, WebSocket streams, and polling sources, persists them, and dispatches to subscribing agent sessions.

## Architecture

- `event.rs` — `FeedEvent` struct (the atomic event envelope) and `FeedEventId` (strongly-typed UUID).
- `store.rs` — `FeedStore` async trait for persistence and `FeedFilter` for querying.
- `mod.rs` — Re-exports only; no logic.

Data flow: Transport layer (webhook/WS/polling) -> `FeedEvent` -> `FeedStore::append` -> subscription dispatch -> agent session.

## Critical Invariants

- `FeedStore::append` MUST be idempotent on `event.id` — duplicate inserts must not create duplicate rows. Violation causes double-processing by subscribers.
- `FeedEventId` uses `base::define_id!` (UUID v4, NonZeroU128) — never construct from arbitrary u128 without `from_uuid`.
- Read cursors are per-subscriber per-source — do not conflate subscribers.

## What NOT To Do

- Do NOT put transport implementations (webhook HTTP handler, WS client) in this module — they belong in dedicated sub-modules or crates.
- Do NOT add `Default` to `FeedFilter` with hardcoded limit — limit must be caller-specified or config-driven.
- Do NOT store `FeedEvent::payload` as typed structs — it is intentionally `serde_json::Value` to support heterogeneous sources.

## Dependencies

- Upstream: `base` (for `define_id!` macro), `jiff` (timestamps), `crate::session` (for `SessionKey`).
- Downstream: future `DataFeedRegistry`, `query-feed` tool, webhook HTTP handler.
- DB: `feed_events` + `feed_read_cursors` tables (migration in `crates/rara-model/migrations/`).
