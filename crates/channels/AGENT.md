# rara-channels — Agent Guidelines

## Purpose

Concrete channel adapter implementations for the rara platform — bridges the kernel's abstract `ChannelAdapter` trait to real communication platforms (Telegram, Web, CLI terminal).

## Architecture

### Key modules

- `src/telegram/` — `TelegramAdapter` using `teloxide` for Bot API long polling. Sub-modules:
  - `adapter.rs` — Core adapter implementing `ChannelAdapter`. Manages bot lifecycle, dispatches updates to kernel.
  - `commands/` — Slash command handlers (`/session`, `/stop`, `/status`, `/tape`, `/help`, `/mcp`) and inline keyboard callback handlers.
  - `markdown.rs` — Telegram MarkdownV2 escaping utilities.
  - `mod.rs` — `TelegramConfig` (primary chat ID, group policy, allowed group).
- `src/web.rs` — `WebAdapter` for the web chat UI. WebSocket + SSE streaming. Authenticated via owner token; once auth passes, the inbound message's `user_id` is taken from the server-trusted `WebAdapterState::owner_user_id` (validated at boot by `rara_app::validate_owner_auth`) — **never** from the client-supplied query string or request body (see #1763). Each WS/SSE connection holds a **permanent** subscription to `StreamHub::subscribe_session_events` for its session — the subscription outlives individual streams so mid-turn interrupt + re-inject does not drop events (see #1647).
- `src/terminal.rs` — `TerminalAdapter` for interactive CLI chat sessions.
- `tool_display` — re-exported from `rara_kernel::trace::tool_display` (the canonical home, since these helpers render data persisted in `ExecutionTrace`). `rara_channels::tool_display::*` remains the import path for in-tree callers.
- `src/lib.rs` — Crate root, re-exports adapter modules.

### Data flow

1. `rara-app` constructs adapters and registers them with `IOSubsystem` by `ChannelType`.
2. Each adapter implements `ChannelAdapter::start()` to begin receiving inbound messages.
3. Inbound messages are forwarded to `KernelHandle::inbound()`.
4. Outbound messages from the kernel flow back through the adapter's `send()` method.
5. Telegram adapter additionally handles commands (via `CommandHandler` trait) and callback queries (via `CallbackHandler` trait).

### Telegram specifics

- Command and callback handlers are set after adapter construction via `set_command_handlers()` / `set_callback_handlers()`.
- `TelegramConfig` is hot-reloadable via `config_handle()` (returns `Arc<RwLock<TelegramConfig>>`), updated when settings change.
- Proxy support via `HTTPS_PROXY` / `HTTP_PROXY` / `ALL_PROXY` env vars. Uses pinned `reqwest 0.12` for teloxide compatibility.

## Critical Invariants

- `ChannelAdapter::start()` must be called before the adapter can receive/send messages — calling `send()` on an unstarted adapter is a no-op or error.
- Telegram bot token comes from the settings store (not config file) — the adapter is skipped if the token is unset.
- The `reqwest` version for telegram proxy is pinned to 0.12 (`reqwest012`) because teloxide 0.17 requires it — do not upgrade to workspace reqwest 0.13 until teloxide supports it.
- Group message handling is controlled by `GroupPolicy` — respect the policy to avoid responding in unauthorized groups.
- **Rate limiting**: All outbound Telegram API calls (`send_message`, `edit_message_text`, `send_photo`, `send_voice`, `send_chat_action`) MUST pass through `ChatRateLimiter::acquire(chat_id)` first (see `src/telegram/rate_limit.rs`). Telegram's group quota is 20 msg/min per group (editMessage shares sendMessage quota — tdlib/td#3034), and bypassing the limiter will cause 429 FloodWait errors that silently drop plan-summary inline buttons in forum topics.

## What NOT To Do

- Do NOT put business logic in adapters — they are message transport only. Logic belongs in the kernel.
- Do NOT bypass `IOSubsystem` for sending messages — adapters should only be accessed through the kernel's I/O layer.
- Do NOT upgrade the telegram `reqwest` dependency to 0.13 — teloxide 0.17 is pinned to 0.12.
- Do NOT hardcode chat IDs or bot tokens — they come from runtime settings.
- Do NOT use `bot.send_message()` / `bot.edit_message_text()` directly — **why:** bypasses `ChatRateLimiter`; Telegram will 429 and inline buttons will silently vanish in forum topics. Always call `rate_limiter.acquire(chat_id).await` first.
- Do NOT construct `rara_kernel::trace::ExecutionTrace` locally or call `TraceService::save` from an adapter — **why:** trace assembly is kernel-owned (see `rara-kernel` AGENT.md "Execution Trace Ownership"). Adapters listen for `StreamEvent::TraceReady { trace_id }` and fetch the persisted row via `TraceService::get` when rendering compact summaries / cascade buttons.
- Do NOT read the authenticated user id from `SessionQuery`, `SendMessageRequest`, or any other client-controlled payload in the web adapter — **why:** owner-token auth proves "you're the owner", not "you are user X". Mixing a client-controlled identity with an owner-token auth path lets a valid token impersonate any `platform_user_id` and causes `identity resolution failed` when the submitted id doesn't match a configured `users[].platforms` entry (see #1763). Always read from `WebAdapterState::owner_user_id`.
- Do NOT spawn a per-inbound-message `StreamHub::subscribe_session` forwarder in the web adapter — **why:** `subscribe_session` snapshots currently-open streams, so any stream opened later (e.g. after the kernel interrupts turn A and re-injects as turn B) goes unobserved. Long-lived WS/SSE connections subscribe once to the session-level bus via `StreamHub::subscribe_session_events` at connect time (see #1647).

## Dependencies

**Upstream:** `rara-kernel` (for `ChannelAdapter` trait, `ChannelType`, `CommandHandler`, `CallbackHandler`), `rara-dock`, `rara-paths`, `teloxide`, `axum` (WebSocket).

**Downstream:** `rara-app` (constructs and registers adapters).
