# rara-telegram-bot

Standalone Telegram bot runtime for the Job Automation Platform.

## Overview

This crate provides a separate process that bridges Telegram users with the main
job service. It runs three concurrent loops:

1. **Telegram Polling** — manual `getUpdates` long-polling loop that receives
   user messages and dispatches them to command/message handlers.
2. **Notification Consumer** — dequeues messages from a `pgmq` queue
   (`notification_telegram_dispatch`) and delivers them to Telegram chats.
3. **Settings Sync** — polls the KV store every 10 seconds and hot-updates bot
   credentials (token, chat ID) without restarting.

```
┌──────────────────────────────────────────────────────────────────┐
│  Telegram User                                                   │
└──────────────┬───────────────────────────────────────────────────┘
               │  getUpdates (30s long poll)
               ▼
┌──────────────────────────────────────────────────────────────────┐
│  bot.rs  ─  Manual Polling Loop                                  │
│  • 45s HTTP client timeout (> 30s poll timeout)                  │
│  • Detects TerminatedByOtherGetUpdates conflicts                 │
│  • CancellationToken-based graceful shutdown                     │
└──────────┬──────────────────────────────────────┬────────────────┘
           │ Message                              │ CallbackQuery
           ▼                                      ▼
┌──────────────────────────────────────────────────────────────────┐
│  handlers.rs  ─  Message & Callback Handlers                     │
│  • /start, /help, /search commands                               │
│  • Plain text → JD parse submission                              │
│  • "Load More" pagination via callback queries                   │
│  • Primary-chat authorization gate                               │
└──────────┬──────────────────────────────────────┬────────────────┘
           │ /search, JD parse                    │ Format + send
           ▼                                      ▼
┌─────────────────────────┐    ┌───────────────────────────────────┐
│  http_client.rs         │    │  outbound.rs                      │
│  • POST /jobs/discover  │    │  • Markdown → Telegram HTML       │
│  • POST /bot/jd-parse   │    │  • Auto-chunking (4096 chars)     │
│                         │    │  • Typing indicators              │
└─────────────────────────┘    └───────────────────────────────────┘
                                          │
                                          ▼
                               ┌──────────────────────┐
                               │  markdown.rs         │
                               │  • **bold** → <b>    │
                               │  • *italic* → <i>    │
                               │  • `code` → <code>   │
                               │  • ```pre``` → <pre>  │
                               │  • [t](u) → <a>      │
                               │  • Smart chunking     │
                               └──────────────────────┘

┌──────────────────────────────────────────────────────────────────┐
│  app.rs  ─  Notification Consumer Loop                           │
│  • Dequeues batches of 50 from pgmq                              │
│  • Delivers via outbound.send_markdown()                         │
│  • Acks on success; retries up to max_retries                    │
└──────────────────────────────────────────────────────────────────┘

┌──────────────────────────────────────────────────────────────────┐
│  app.rs  ─  Settings Sync Loop                                   │
│  • Polls KV store every 10s                                      │
│  • Hot-updates bot_token + chat_id via BotState.update_config()  │
└──────────────────────────────────────────────────────────────────┘
```

## Module Structure

| Module          | Purpose                                                  |
|-----------------|----------------------------------------------------------|
| `lib.rs`        | Crate root; module declarations and public re-exports    |
| `config.rs`     | Environment parsing, dependency wiring, `BotConfig::open()` |
| `app.rs`        | Process lifecycle, notification consumer, settings sync  |
| `bot.rs`        | Manual `getUpdates` long-polling loop                    |
| `handlers.rs`   | Message routing, command handlers, callback queries      |
| `state.rs`      | `BotState` — shared runtime state with hot-update support |
| `outbound.rs`   | `TelegramOutbound` — message sending with formatting     |
| `markdown.rs`   | Markdown-to-Telegram-HTML converter and message chunker  |
| `command.rs`    | Telegram command definitions via `teloxide::BotCommands` |
| `http_client.rs`| Typed HTTP client for main service API calls             |

## Bot Commands

| Command                           | Description                              |
|-----------------------------------|------------------------------------------|
| `/start`                          | Welcome message with bot capabilities    |
| `/help`                           | List all available commands              |
| `/search <keywords> [@ location]` | Search jobs with optional location filter |

Sending plain text (non-command) treats the message as a raw Job Description and
submits it to the main service for parsing.

## Environment Variables

| Variable                 | Required | Default                                          | Description                     |
|--------------------------|----------|--------------------------------------------------|---------------------------------|
| `TELEGRAM_BOT_TOKEN`    | Yes*     | —                                                | Bot token from @BotFather       |
| `TELEGRAM_CHAT_ID`      | Yes*     | —                                                | Primary authorized chat ID      |
| `DATABASE_URL`           | No       | `postgres://postgres:postgres@localhost:5432/job` | PostgreSQL connection string    |
| `MIGRATION_DIRECTORY`    | No       | `crates/rara-model/migrations`                   | SQL migration directory         |
| `MAIN_SERVICE_HTTP_BASE` | No       | `http://127.0.0.1:25555`                         | Main service HTTP base URL      |

\* Can also be set via the runtime settings API (`/api/v1/settings`), which
takes precedence over environment variables.

## Authorization

All user-facing operations are gated by **primary chat ID**: only messages from
the configured `TELEGRAM_CHAT_ID` are processed. Messages from other chats
receive an "Unauthorized chat." reply and are dropped.

## Design Decisions

### Manual Polling vs Teloxide Dispatcher

We use a hand-rolled `getUpdates` loop instead of teloxide's built-in
`Dispatcher` for several reasons:

- **Error recovery** — on transient failures we sleep 5s and retry, rather than
  crashing the entire dispatcher.
- **Conflict detection** — if another bot instance is running with the same
  token, the `TerminatedByOtherGetUpdates` error is caught and the loop exits
  gracefully instead of retrying indefinitely.
- **Cancellation** — `tokio::select!` on the `CancellationToken` allows the
  polling loop to exit mid-wait during shutdown, rather than waiting for the
  full 30s poll timeout to expire.
- **Timeout alignment** — the HTTP client timeout (45s) is set higher than the
  long-poll timeout (30s) to prevent the client from aborting before Telegram
  responds.

### Message Formatting

Telegram's Bot API supports a limited HTML subset. The `markdown.rs` module
converts standard Markdown formatting to this subset and automatically splits
messages that exceed the 4096-character limit at newline or space boundaries.

### Hot Configuration

The settings sync loop polls the database KV store and updates bot credentials
in-memory via `Arc<RwLock<TelegramRuntimeConfig>>`. This allows operators to
change the bot token or chat ID through the web UI without restarting the bot
process.
