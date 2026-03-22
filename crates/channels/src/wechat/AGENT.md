# wechat — Agent Guidelines

## Purpose
WeChat iLink Bot channel adapter — bridges rara's `ChannelAdapter` trait to WeChat's personal bot API via long-polling.

## Architecture
- `adapter.rs` — `WechatAdapter` struct implementing `ChannelAdapter`. Uses `wechat-agent-rs` for protocol.
- Inbound: long-polling via `WeixinApiClient::get_updates()` → parse JSON → build `RawPlatformMessage` → `KernelHandle::ingest()`
- Outbound: `PlatformOutbound::Reply` → `markdown_to_plain_text()` → `WeixinApiClient::send_text_message()`
- `context_tokens` DashMap caches the latest `context_token` per user_id (required by iLink protocol for reply routing).

## Critical Invariants
- Token comes from `wechat-agent-rs` storage (filesystem at `~/.openclaw/`), not from rara config YAML. Only `account_id` is configured in rara.
- Long-polling loop must respect the `shutdown_rx` watch channel — check it between polls.
- `context_token` from each incoming message must be cached and passed back when sending replies. Without it, the iLink API rejects outbound messages.
- `StreamChunk` and `Progress` outbound types are silently ignored — WeChat has no streaming edit API.

## What NOT To Do
- Do NOT put message processing logic here — adapter is transport only; logic belongs in kernel.
- Do NOT call `wechat_agent_rs::bot::start()` — the adapter manages its own polling loop to integrate with `KernelHandle`.
- Do NOT store WeChat tokens in rara's settings DB — they live in `wechat-agent-rs` filesystem storage.
- Do NOT send markdown to WeChat — always convert via `markdown_to_plain_text()` first.

## Dependencies
- Upstream: `rara-kernel` (ChannelAdapter trait, types, KernelHandle)
- External: `wechat-agent-rs` (WeChat iLink Bot protocol, storage, runtime helpers)
- Downstream: `rara-app` (adapter registration via `try_build_wechat()`)
