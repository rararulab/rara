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
- `src/web.rs` — `WebAdapter` for the web chat UI. WebSocket + SSE streaming. Authenticated via owner token.
- `src/terminal.rs` — `TerminalAdapter` for interactive CLI chat sessions.
- `src/tool_display.rs` — Human-readable formatting of tool calls/results for channel output.
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

## What NOT To Do

- Do NOT put business logic in adapters — they are message transport only. Logic belongs in the kernel.
- Do NOT bypass `IOSubsystem` for sending messages — adapters should only be accessed through the kernel's I/O layer.
- Do NOT upgrade the telegram `reqwest` dependency to 0.13 — teloxide 0.17 is pinned to 0.12.
- Do NOT hardcode chat IDs or bot tokens — they come from runtime settings.

## Dependencies

**Upstream:** `rara-kernel` (for `ChannelAdapter` trait, `ChannelType`, `CommandHandler`, `CallbackHandler`), `rara-dock`, `rara-paths`, `teloxide`, `axum` (WebSocket).

**Downstream:** `rara-app` (constructs and registers adapters).
