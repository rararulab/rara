// Copyright 2025 Crrow
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//      http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Process lifecycle for the Telegram bot.
//!
//! [`BotApp::run`] is the standalone entry point that installs a Ctrl+C
//! handler and blocks until shutdown. [`BotApp::spawn`] is the non-blocking
//! variant used when the bot runs inside the app process — it returns a
//! [`BotHandle`] that the parent can use to await graceful shutdown.

use std::sync::Arc;

use rara_domain_shared::settings::model::Settings;
use snafu::{ResultExt, Whatever};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::{
    config::TelegramConfig, http_client::MainServiceHttpClient, outbound::TelegramOutbound,
    state::BotState,
};

/// Top-level application handle for the bot process.
///
/// Created by [`BotConfig::open`](crate::BotConfig::open) (standalone) or
/// [`BotApp::from_shared`] (embedded). Call [`run`](BotApp::run) to block
/// until shutdown, or [`spawn`](BotApp::spawn) for non-blocking operation.
pub struct BotApp {
    pub(crate) state:         Arc<BotState>,
    pub(crate) outbound:      Arc<TelegramOutbound>,
    /// Shared notification queue client (`notification_telegram_dispatch`).
    pub(crate) notify_client: Arc<rara_domain_shared::notify::client::NotifyClient>,
    /// Watch receiver for instant settings propagation.
    pub(crate) settings_rx:   watch::Receiver<Settings>,
}

/// Handle returned by [`BotApp::spawn`] for non-blocking operation.
///
/// The parent process holds this handle and calls
/// [`shutdown`](BotHandle::shutdown) during its own teardown sequence.
pub struct BotHandle {
    handles: Vec<tokio::task::JoinHandle<()>>,
}

impl BotHandle {
    /// Wait for all bot tasks to finish.
    pub async fn shutdown(self) {
        for h in self.handles {
            let _ = h.await;
        }
    }
}

impl BotApp {
    /// Maximum number of pgmq messages to dequeue per batch.
    const NOTIFY_BATCH_SIZE: i32 = 50;
    /// Sleep duration between poll cycles when the notification queue is empty.
    const NOTIFY_IDLE_SLEEP_SECS: u64 = 5;
    /// pgmq visibility timeout — how long a dequeued message stays invisible
    /// to other consumers before being re-delivered if not acked.
    const NOTIFY_VT_SECONDS: i32 = 60;

    /// Construct from shared infrastructure (used when running inside the app
    /// process).
    ///
    /// The caller supplies a [`CancellationToken`], a
    /// [`watch::Receiver<Settings>`] for instant settings propagation, and a
    /// [`NotifyClient`] that are already initialized by the app crate.
    /// This avoids creating a second DB pool.
    ///
    /// Returns `Ok(Some(bot))` if Telegram credentials are available (from
    /// `telegram_config` or the current settings snapshot). Returns `Ok(None)`
    /// if Telegram is not configured. Returns `Err` on initialization failure.
    pub async fn from_shared(
        cancel: CancellationToken,
        settings_rx: watch::Receiver<Settings>,
        notify_client: Arc<rara_domain_shared::notify::client::NotifyClient>,
        telegram_config: Option<TelegramConfig>,
        main_service_http_base: String,
    ) -> Result<Option<Self>, Whatever> {
        // Read the current settings snapshot from the watch channel.
        let runtime_settings = settings_rx.borrow().clone();

        let bot_token = runtime_settings
            .telegram
            .bot_token
            .or_else(|| telegram_config.as_ref().map(|cfg| cfg.bot_token.clone()));
        let chat_id = runtime_settings.telegram.chat_id;

        let (Some(bot_token), Some(chat_id)) = (bot_token, chat_id) else {
            // Neither env var nor watch channel has Telegram credentials — skip.
            return Ok(None);
        };

        // Build reqwest client with extended timeout for long polling.
        // Use teloxide's re-exported reqwest to match the version teloxide expects.
        let http_client = teloxide::net::default_reqwest_settings()
            .timeout(std::time::Duration::from_secs(45))
            .build()
            .whatever_context("failed to build HTTP client")?;

        let bot = teloxide::Bot::with_client(&bot_token, http_client);

        // Verify token and get bot info.
        let bot_username = crate::bot::initialize(&bot)
            .await
            .whatever_context("failed to initialize telegram bot")?;

        let main_http = Arc::new(MainServiceHttpClient::new(main_service_http_base));

        let state = Arc::new(BotState::new(
            bot.clone(),
            bot_username,
            bot_token,
            chat_id,
            runtime_settings.telegram.allowed_group_chat_id,
            main_http,
            cancel,
        ));

        let outbound = Arc::new(TelegramOutbound::new(bot, state.config.clone()));

        Ok(Some(Self {
            state,
            outbound,
            notify_client,
            settings_rx,
        }))
    }

    /// Spawn all concurrent loops and return a [`BotHandle`] immediately.
    ///
    /// Unlike [`run`](BotApp::run), this method does **not** install a
    /// Ctrl+C handler and does **not** block. The parent process is
    /// responsible for cancelling the token and calling
    /// [`BotHandle::shutdown`].
    #[must_use]
    pub fn spawn(self) -> BotHandle {
        let polling_state = self.state.clone();
        let polling_handle = tokio::spawn(async move {
            Box::pin(crate::bot::start_polling(polling_state)).await;
        });

        let notify_consumer_handle = tokio::spawn(Self::notify_consumer_loop(
            self.notify_client.clone(),
            self.outbound.clone(),
            self.state.cancel.clone(),
        ));

        let settings_sync_handle = tokio::spawn(Self::settings_watch_loop(
            self.settings_rx,
            self.state.clone(),
            self.state.cancel.clone(),
        ));

        BotHandle {
            handles: vec![polling_handle, notify_consumer_handle, settings_sync_handle],
        }
    }

    /// Format a queued notification into a Markdown message for Telegram.
    ///
    /// If the notification has a subject, it is rendered as bold text followed
    /// by the body. Otherwise only the body is sent.
    fn format_notification_message(
        notification: &rara_domain_shared::notify::types::QueuedTelegramNotification,
    ) -> String {
        use std::fmt::Write;
        let mut text = String::new();
        if let Some(subject) = &notification.subject {
            let _ = write!(text, "*{subject}*\n\n");
        }
        text.push_str(&notification.body);
        text
    }

    /// Continuously dequeue notifications from pgmq and deliver them via
    /// Telegram.
    ///
    /// On successful delivery the message is acked. On failure it is left in
    /// the queue for retry until `max_retries` is reached, at which point it
    /// is acked (dropped) to prevent infinite retry loops.
    async fn notify_consumer_loop(
        notify_client: Arc<rara_domain_shared::notify::client::NotifyClient>,
        outbound: Arc<TelegramOutbound>,
        cancellation_token: CancellationToken,
    ) {
        loop {
            tokio::select! {
                () = cancellation_token.cancelled() => break,
                () = tokio::time::sleep(std::time::Duration::from_secs(Self::NOTIFY_IDLE_SLEEP_SECS)) => {}
            }

            let batch = match notify_client
                .dequeue_telegram_batch(Self::NOTIFY_BATCH_SIZE, Self::NOTIFY_VT_SECONDS)
                .await
            {
                Ok(batch) => batch,
                Err(e) => {
                    error!(error = %e, "failed to dequeue notify batch in telegram-bot");
                    continue;
                }
            };

            if batch.is_empty() {
                continue;
            }

            for item in batch {
                let text = Self::format_notification_message(&item.payload);
                let chat_id = item
                    .payload
                    .chat_id
                    .map(teloxide::types::ChatId)
                    .unwrap_or_else(|| {
                        // Use primary chat when no explicit chat_id is set.
                        let config = outbound.primary_config();
                        teloxide::types::ChatId(config.primary_chat_id)
                    });

                // Send photo if present, otherwise send text.
                let delivery = if let Some(ref photo_path) = item.payload.photo_path {
                    let path = std::path::Path::new(photo_path);
                    if path.exists() {
                        let caption = if text.trim().is_empty() {
                            None
                        } else {
                            Some(text.as_str())
                        };
                        outbound.send_photo(chat_id, path, caption).await
                    } else {
                        warn!(photo_path, "photo file not found, falling back to text");
                        outbound.send_markdown(chat_id, &text).await
                    }
                } else {
                    outbound.send_markdown(chat_id, &text).await
                };

                match delivery {
                    Ok(()) => {
                        if let Err(e) = notify_client.ack_telegram(item.msg_id).await {
                            error!(msg_id = item.msg_id, error = %e, "failed to ack delivered telegram notification");
                        }
                    }
                    Err(e) => {
                        warn!(msg_id = item.msg_id, error = %e, read_ct = item.read_ct, "telegram notification delivery failed");
                        if item.read_ct >= item.payload.max_retries
                            && let Err(ack_err) = notify_client.ack_telegram(item.msg_id).await
                        {
                            error!(msg_id = item.msg_id, error = %ack_err, "failed to ack terminal telegram notification");
                        }
                    }
                }
            }
        }
    }

    /// Watch for settings changes via the [`watch::Receiver`] and apply
    /// updated Telegram credentials to the running bot without restart.
    ///
    /// This enables operators to change `bot_token` or `chat_id` via the
    /// web settings UI. Changes propagate instantly through the watch channel
    /// rather than being polled on a timer.
    async fn settings_watch_loop(
        mut settings_rx: watch::Receiver<Settings>,
        state: Arc<BotState>,
        cancellation_token: CancellationToken,
    ) {
        loop {
            tokio::select! {
                () = cancellation_token.cancelled() => break,
                result = settings_rx.changed() => {
                    if result.is_err() {
                        // Sender dropped — service is shutting down.
                        break;
                    }
                    let settings = settings_rx.borrow_and_update().clone();

                    let (Some(bot_token), Some(chat_id)) = (
                        settings.telegram.bot_token.clone(),
                        settings.telegram.chat_id,
                    ) else {
                        continue;
                    };

                    if state.update_config(
                        bot_token,
                        chat_id,
                        settings.telegram.allowed_group_chat_id,
                    ) {
                        info!("telegram runtime settings updated via watch channel");
                    }
                }
            }
        }
    }

    /// Start all concurrent loops and block until shutdown (standalone mode).
    ///
    /// Spawns three tokio tasks:
    /// 1. `getUpdates` polling loop
    ///    ([`bot::start_polling`](crate::bot::start_polling))
    /// 2. Notification consumer loop (pgmq -> Telegram delivery)
    /// 3. Settings watch loop (watch channel -> hot credential update)
    ///
    /// Installs a Ctrl+C handler that cancels the shared
    /// [`CancellationToken`], then waits for all tasks to join.
    pub async fn run(self) -> Result<(), Whatever> {
        let cancel = self.state.cancel.clone();

        // Spawn the three loops via `spawn()`.
        let bot_handle = self.spawn();

        // Install Ctrl+C handler for standalone mode.
        tokio::spawn(async move {
            tokio::signal::ctrl_c()
                .await
                .expect("failed to install Ctrl+C handler");
            cancel.cancel();
        });

        // Wait for cancellation then join all tasks.
        bot_handle.shutdown().await;
        info!("telegram-bot shutdown complete");

        Ok(())
    }
}
