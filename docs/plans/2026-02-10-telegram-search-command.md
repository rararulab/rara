# Telegram Bot /search Command Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a `/search <keywords> <location>` command to the Telegram bot that discovers jobs via JobSpyDriver and returns formatted results with a "Load More" inline keyboard button.

**Architecture:** Extend `TelegramBotWorker` with a new `/search` command that calls `JobSourceService::discover()` (via `spawn_blocking` since it's synchronous PyO3). Results are formatted as Telegram messages with inline keyboard. Callback queries handle "Load More" by re-running the search with increased `max_results` and editing the original message.

**Tech Stack:** teloxide 0.14 (macros, inline keyboard, callback queries), job-domain-job-source (JobSourceService, DiscoveryCriteria, NormalizedJob)

---

### Task 1: Add `JobSourceService` to `WorkerState`

**Files:**
- Modify: `crates/workers/src/notification_processor.rs:73-80` (WorkerState struct)
- Modify: `crates/app/src/lib.rs:348-355` (WorkerState construction)

**Step 1: Add the field to WorkerState**

In `crates/workers/src/notification_processor.rs`, add to the `WorkerState` struct:

```rust
pub struct WorkerState {
    pub notification_service: Arc<NotificationService>,
    pub ai_service:           Option<Arc<job_ai::service::AiService>>,
    pub job_repo:             Option<Arc<dyn job_domain_job_source::repository::JobRepository>>,
    pub jd_parse_tx:          mpsc::Sender<JdParseRequest>,
    pub jd_parse_notify:      Arc<tokio::sync::Mutex<Option<NotifyHandle>>>,
    pub telegram:             Arc<TelegramService>,
    pub job_source_service:   Arc<job_domain_job_source::service::JobSourceService>,
}
```

**Step 2: Wire it in the app composition root**

In `crates/app/src/lib.rs`, update the `WorkerState` construction (~line 348) to pass the existing `job_source_service`:

```rust
let worker_state = job_workers::notification_processor::WorkerState {
    notification_service: self.notification_service,
    ai_service:           self.ai_service,
    job_repo:             Some(self.job_repo),
    jd_parse_tx:          jd_tx,
    jd_parse_notify:      jd_parse_notify.clone(),
    telegram:             self.telegram,
    job_source_service:   job_source_service.clone(),
};
```

Note: `job_source_service` is created at line 202 and already `Arc<JobSourceService>`. But it's currently consumed into the routes closure. We need to clone it before the closure captures it. The existing code already has `let job_source_svc = job_source_service.clone();` at line 224 for the routes closure, so `job_source_service` is still available. However we also need `App` to hold onto it. Add `job_source_service` field to `App` struct:

```rust
pub struct App {
    // ... existing fields ...
    job_source_service: Arc<job_domain_job_source::service::JobSourceService>,
}
```

And in `AppConfig::open()` return, add it:
```rust
Ok(App {
    config: self,
    // ... existing fields ...
    job_source_service,
})
```

Then in `App::start()`, use `self.job_source_service` when building WorkerState.

**Step 3: Verify it compiles**

Run: `cargo check -p job-workers -p job-app`
Expected: OK (no functional changes yet)

**Step 4: Commit**

```bash
git add crates/workers/src/notification_processor.rs crates/app/src/lib.rs
git commit -m "feat(workers): add JobSourceService to WorkerState for Telegram search"
```

---

### Task 2: Add `/search` command to TelegramBotWorker

**Files:**
- Modify: `crates/workers/src/telegram_bot.rs` (command enum, handler, dispatcher)

**Step 1: Extend Command enum with Search**

```rust
#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase", description = "Available commands:")]
enum Command {
    #[command(description = "Start the bot")]
    Start,
    #[command(description = "Show help")]
    Help,
    #[command(description = "Search jobs: /search <keywords> [location]")]
    Search(String),
}
```

**Step 2: Add `handle_search` function**

```rust
/// Parse `/search <keywords> [@ location]` and call JobSourceService.
///
/// Format: `/search rust engineer @ beijing`
/// Or simply: `/search rust engineer` (no location)
async fn handle_search(
    bot: Bot,
    msg: Message,
    args: String,
    telegram: Arc<TelegramService>,
    job_source_service: Arc<job_domain_job_source::service::JobSourceService>,
) -> ResponseResult<()> {
    if !telegram.is_primary_chat(msg.chat.id) {
        bot.send_message(msg.chat.id, "Unauthorized chat.").await?;
        return Ok(());
    }

    let args = args.trim();
    if args.is_empty() {
        bot.send_message(
            msg.chat.id,
            "Usage: /search <keywords> [@ location]\nExample: /search rust engineer @ beijing",
        )
        .await?;
        return Ok(());
    }

    // Split on " @ " to separate keywords from location
    let (keywords_str, location) = if let Some(idx) = args.find(" @ ") {
        (&args[..idx], Some(args[idx + 3..].trim().to_owned()))
    } else {
        (args, None)
    };

    let keywords: Vec<String> = keywords_str
        .split_whitespace()
        .map(String::from)
        .collect();

    if keywords.is_empty() {
        bot.send_message(msg.chat.id, "Please provide at least one keyword.")
            .await?;
        return Ok(());
    }

    let location_display = location.as_deref().unwrap_or("any");
    bot.send_message(
        msg.chat.id,
        format!("🔍 Searching: {} @ {} ...", keywords.join(" "), location_display),
    )
    .await?;

    let max_results: u32 = 3;
    let criteria = job_domain_job_source::types::DiscoveryCriteria {
        keywords: keywords.clone(),
        location: location.clone(),
        max_results: Some(max_results),
        ..Default::default()
    };

    let svc = job_source_service.clone();
    let result = tokio::task::spawn_blocking(move || {
        let empty_source = std::collections::HashSet::new();
        let empty_fuzzy = std::collections::HashSet::new();
        svc.discover(&criteria, &empty_source, &empty_fuzzy)
    })
    .await;

    let discovery = match result {
        Ok(d) => d,
        Err(e) => {
            bot.send_message(msg.chat.id, format!("❌ Search failed: {e}"))
                .await?;
            return Ok(());
        }
    };

    if let Some(ref err) = discovery.error {
        bot.send_message(msg.chat.id, format!("❌ Search error: {err}"))
            .await?;
        return Ok(());
    }

    if discovery.jobs.is_empty() {
        bot.send_message(msg.chat.id, "No jobs found matching your criteria.")
            .await?;
        return Ok(());
    }

    let text = format_job_results(&discovery.jobs, &keywords, location.as_deref());

    // Build inline keyboard with "Load More" button.
    // Encode search params in callback data: "search_more:<offset>:<keywords>[@<location>]"
    let callback_data = format!(
        "search_more:{}:{}",
        discovery.jobs.len(),
        encode_search_params(&keywords, location.as_deref()),
    );
    let keyboard = teloxide::types::InlineKeyboardMarkup::new(vec![vec![
        teloxide::types::InlineKeyboardButton::callback("📄 Load More", callback_data),
    ]]);

    bot.send_message(msg.chat.id, text)
        .parse_mode(teloxide::types::ParseMode::Html)
        .reply_markup(keyboard)
        .await?;

    Ok(())
}
```

**Step 3: Add helper functions for formatting**

```rust
/// Format job results into a Telegram message (HTML parse mode).
fn format_job_results(
    jobs: &[job_domain_job_source::types::NormalizedJob],
    keywords: &[String],
    location: Option<&str>,
) -> String {
    let location_display = location.unwrap_or("any");
    let mut text = format!(
        "Found <b>{}</b> jobs for <i>{}</i> @ <i>{}</i>:\n\n",
        jobs.len(),
        keywords.join(" "),
        location_display,
    );

    for (i, job) in jobs.iter().enumerate() {
        text.push_str(&format!("<b>{}.</b> {} — {}\n", i + 1, job.title, job.company));
        if let Some(loc) = &job.location {
            text.push_str(&format!("   📍 {}", loc));
        }
        if let (Some(min), Some(max)) = (job.salary_min, job.salary_max) {
            let currency = job.salary_currency.as_deref().unwrap_or("");
            text.push_str(&format!(" | 💰 {}-{} {}", min, max, currency));
        }
        text.push('\n');
        if let Some(url) = &job.url {
            text.push_str(&format!("   🔗 {}\n", url));
        }
        text.push('\n');
    }

    text
}

/// Encode search params into a compact callback data string.
/// Format: "kw1+kw2[@location]"
fn encode_search_params(keywords: &[String], location: Option<&str>) -> String {
    let kw = keywords.join("+");
    match location {
        Some(loc) => format!("{}@{}", kw, loc),
        None => kw,
    }
}

/// Decode search params from callback data.
/// Returns (keywords, optional location).
fn decode_search_params(encoded: &str) -> (Vec<String>, Option<String>) {
    if let Some(idx) = encoded.find('@') {
        let kw = encoded[..idx]
            .split('+')
            .map(String::from)
            .collect();
        let loc = encoded[idx + 1..].to_owned();
        (kw, Some(loc))
    } else {
        let kw = encoded.split('+').map(String::from).collect();
        (kw, None)
    }
}
```

**Step 4: Update `handle_command` to route Search**

```rust
async fn handle_command(
    bot: Bot,
    msg: Message,
    cmd: Command,
    telegram: Arc<TelegramService>,
    job_source_service: Arc<job_domain_job_source::service::JobSourceService>,
) -> ResponseResult<()> {
    match cmd {
        Command::Start => {
            bot.send_message(
                msg.chat.id,
                "Welcome! I'm the Job Assistant bot.\n\
                 • Send me a JD text and I'll parse it\n\
                 • Use /search <keywords> [@ location] to find jobs\n\
                 • Use /help to see all commands",
            )
            .await?;
        }
        Command::Help => {
            bot.send_message(msg.chat.id, Command::descriptions().to_string())
                .await?;
        }
        Command::Search(args) => {
            handle_search(bot, msg, args, telegram, job_source_service).await?;
        }
    }
    Ok(())
}
```

**Step 5: Add callback query handler for "Load More"**

```rust
/// Handle inline keyboard callback queries (e.g. "Load More" button).
async fn handle_callback_query(
    bot: Bot,
    q: CallbackQuery,
    telegram: Arc<TelegramService>,
    job_source_service: Arc<job_domain_job_source::service::JobSourceService>,
) -> ResponseResult<()> {
    // Acknowledge the callback to remove the "loading" spinner.
    bot.answer_callback_query(&q.id).await?;

    let data = match q.data.as_deref() {
        Some(d) => d,
        None => return Ok(()),
    };

    if !data.starts_with("search_more:") {
        return Ok(());
    }

    // Parse callback data: "search_more:<current_count>:<encoded_params>"
    let parts: Vec<&str> = data.splitn(3, ':').collect();
    if parts.len() != 3 {
        return Ok(());
    }
    let current_count: u32 = match parts[1].parse() {
        Ok(n) => n,
        Err(_) => return Ok(()),
    };
    let (keywords, location) = decode_search_params(parts[2]);

    let new_max = current_count + 3;
    let criteria = job_domain_job_source::types::DiscoveryCriteria {
        keywords: keywords.clone(),
        location: location.clone(),
        max_results: Some(new_max),
        ..Default::default()
    };

    let svc = job_source_service.clone();
    let result = tokio::task::spawn_blocking(move || {
        let empty_source = std::collections::HashSet::new();
        let empty_fuzzy = std::collections::HashSet::new();
        svc.discover(&criteria, &empty_source, &empty_fuzzy)
    })
    .await;

    let discovery = match result {
        Ok(d) => d,
        Err(e) => {
            if let Some(msg) = &q.message {
                let chat_id = msg.chat().id;
                bot.send_message(chat_id, format!("❌ Load more failed: {e}"))
                    .await?;
            }
            return Ok(());
        }
    };

    if let Some(ref err) = discovery.error {
        if let Some(msg) = &q.message {
            let chat_id = msg.chat().id;
            bot.send_message(chat_id, format!("❌ Error: {err}"))
                .await?;
        }
        return Ok(());
    }

    let text = format_job_results(&discovery.jobs, &keywords, location.as_deref());

    // Update the inline keyboard — if we got fewer results than requested,
    // there are no more results, so remove the button.
    if let Some(msg) = &q.message {
        let msg_id = msg.id();
        let chat_id = msg.chat().id;

        if (discovery.jobs.len() as u32) < new_max {
            // No more results — remove the keyboard
            bot.edit_message_text(chat_id, msg_id, text)
                .parse_mode(teloxide::types::ParseMode::Html)
                .await?;
        } else {
            // Still more — update the button with new offset
            let callback_data = format!(
                "search_more:{}:{}",
                discovery.jobs.len(),
                encode_search_params(&keywords, location.as_deref()),
            );
            let keyboard = teloxide::types::InlineKeyboardMarkup::new(vec![vec![
                teloxide::types::InlineKeyboardButton::callback("📄 Load More", callback_data),
            ]]);
            bot.edit_message_text(chat_id, msg_id, text)
                .parse_mode(teloxide::types::ParseMode::Html)
                .reply_markup(keyboard)
                .await?;
        }
    }

    Ok(())
}
```

**Step 6: Update the dispatcher in `TelegramBotWorker::work()`**

The dispatcher needs to handle both messages and callback queries:

```rust
#[async_trait]
impl FallibleWorker<WorkerState> for TelegramBotWorker {
    async fn work(&mut self, ctx: WorkerContext<WorkerState>) -> WorkResult {
        let state = ctx.state();
        let telegram = state.telegram.clone();
        let bot = telegram.bot();
        let jd_tx = state.jd_parse_tx.clone();
        let jd_notify = state.jd_parse_notify.clone();
        let job_source_service = state.job_source_service.clone();

        let handler = dptree::entry()
            .branch(
                Update::filter_message()
                    .branch(
                        dptree::entry()
                            .filter_command::<Command>()
                            .endpoint(handle_command),
                    )
                    .branch(dptree::entry().endpoint(handle_message)),
            )
            .branch(
                Update::filter_callback_query()
                    .endpoint(handle_callback_query),
            );

        let _ = bot.delete_webhook().drop_pending_updates(true).await;

        let mut dispatcher = Dispatcher::builder(bot, handler)
            .dependencies(dptree::deps![jd_tx, jd_notify, telegram, job_source_service])
            .enable_ctrlc_handler()
            .build();

        let shutdown_token = dispatcher.shutdown_token();
        let child_token = ctx.child_token();
        tokio::spawn(async move {
            child_token.cancelled().await;
            if let Ok(f) = shutdown_token.shutdown() {
                f.await;
            }
        });

        dispatcher.dispatch().await;
        Ok(())
    }
}
```

**Step 7: Verify it compiles**

Run: `cargo check -p job-workers`
Expected: OK

**Step 8: Commit**

```bash
git add crates/workers/src/telegram_bot.rs
git commit -m "feat(telegram): add /search command with inline Load More button"
```

---

### Task 3: Verify full build & test

**Step 1: Full workspace build check**

Run: `cargo check --workspace`
Expected: OK

**Step 2: Run existing tests**

Run: `cargo test --workspace -- --skip testcontainers 2>&1 | tail -20`
Expected: All existing tests pass

**Step 3: Commit any fixups if needed**

---

## Notes for Implementer

1. **teloxide CallbackQuery**: `q.message` is `Option<MaybeInaccessibleMessage>`. Use `.chat()` and `.id()` methods to extract chat_id and message_id for editing.

2. **Callback data size limit**: Telegram limits callback_data to 64 bytes. The encoding `search_more:<count>:<kw1+kw2@location>` should stay well within this for typical searches. If keywords are very long, consider truncating.

3. **`discover()` is blocking (PyO3 GIL)**: Always wrap in `tokio::task::spawn_blocking()`.

4. **HTML parse mode**: Use `<b>`, `<i>`, `<a href="...">` for formatting. Escape `<`, `>`, `&` in user-provided text if needed.

5. **`handle_command` signature change**: The function now takes extra deps (`telegram`, `job_source_service`). teloxide's dptree injection handles this — just add them as parameters and ensure they're in `dptree::deps![]`.

6. **No new crate dependencies needed**: `job-workers` already depends on `job-domain-job-source` and `teloxide`.
