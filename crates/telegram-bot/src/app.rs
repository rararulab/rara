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
//! [`BotApp::run`] is the main entry point. It spawns three concurrent tasks
//! (polling, notification consumer, settings sync) and blocks until Ctrl+C
//! triggers a graceful shutdown.

use std::sync::Arc;

use rara_domain_shared::settings::{model::Settings, service::RUNTIME_SETTINGS_KV_KEY};
use snafu::{ResultExt, Whatever};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::{
    outbound::TelegramOutbound,
    state::BotState,
};

/// Top-level application handle for the bot process.
///
/// Created by [`BotConfig::open`](crate::BotConfig::open). Call [`run`](BotApp::run)
/// to start the polling loop, notification consumer, and settings sync.
pub struct BotApp {
    pub(crate) state:         Arc<BotState>,
    pub(crate) outbound:      Arc<TelegramOutbound>,
    /// Shared notification queue client (`notification_telegram_dispatch`).
    pub(crate) notify_client: Arc<rara_domain_shared::notify::client::NotifyClient>,
    /// KV store used for runtime settings sync.
    pub(crate) kv_store:      yunara_store::KVStore,
}

impl BotApp {
    /// Maximum number of pgmq messages to dequeue per batch.
    const NOTIFY_BATCH_SIZE: i32 = 50;
    /// Sleep duration between poll cycles when the notification queue is empty.
    const NOTIFY_IDLE_SLEEP_SECS: u64 = 5;
    /// pgmq visibility timeout — how long a dequeued message stays invisible
    /// to other consumers before being re-delivered if not acked.
    const NOTIFY_VT_SECONDS: i32 = 60;
    /// How often the settings sync loop polls the KV store for credential
    /// updates.
    const SETTINGS_SYNC_INTERVAL_SECS: u64 = 10;

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

                let delivery = outbound.send_markdown(chat_id, &text).await;

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

    /// Poll the KV store for updated Telegram credentials and apply them
    /// to the running bot without restart.
    ///
    /// This enables operators to change `bot_token` or `chat_id` via the
    /// web settings UI. Changes take effect within
    /// [`SETTINGS_SYNC_INTERVAL_SECS`](Self::SETTINGS_SYNC_INTERVAL_SECS).
    async fn settings_sync_loop(
        kv_store: yunara_store::KVStore,
        state: Arc<BotState>,
        cancellation_token: CancellationToken,
    ) {
        loop {
            tokio::select! {
                () = cancellation_token.cancelled() => break,
                () = tokio::time::sleep(std::time::Duration::from_secs(Self::SETTINGS_SYNC_INTERVAL_SECS)) => {}
            }

            let loaded = kv_store.get::<Settings>(RUNTIME_SETTINGS_KV_KEY).await;
            let mut settings = match loaded {
                Ok(Some(settings)) => settings,
                Ok(None) => continue,
                Err(e) => {
                    warn!(error = %e, "failed to load runtime settings in bot sync loop");
                    continue;
                }
            };
            settings.normalize();

            let (Some(bot_token), Some(chat_id)) = (
                settings.telegram.bot_token.clone(),
                settings.telegram.chat_id,
            ) else {
                continue;
            };

            if state.update_config(bot_token, chat_id) {
                info!("telegram runtime settings updated from DB");
            }
        }
    }

    /// Start all concurrent loops and block until shutdown.
    ///
    /// Spawns three tokio tasks:
    /// 1. `getUpdates` polling loop ([`bot::start_polling`](crate::bot::start_polling))
    /// 2. Notification consumer loop (pgmq -> Telegram delivery)
    /// 3. Settings sync loop (KV store -> hot credential update)
    ///
    /// Installs a Ctrl+C handler that cancels the shared
    /// [`CancellationToken`], then waits for all tasks to join.
    pub async fn run(self) -> Result<(), Whatever> {
        // Start manual getUpdates polling loop.
        let polling_state = self.state.clone();
        let polling_handle = tokio::spawn(async move {
            Box::pin(crate::bot::start_polling(polling_state)).await;
        });

        // Start notify queue consumer (pgmq) for main-service -> bot delivery.
        let notify_consumer_handle = tokio::spawn(Self::notify_consumer_loop(
            self.notify_client.clone(),
            self.outbound.clone(),
            self.state.cancel.clone(),
        ));
        let settings_sync_handle = tokio::spawn(Self::settings_sync_loop(
            self.kv_store.clone(),
            self.state.clone(),
            self.state.cancel.clone(),
        ));

        // Keep process alive until Ctrl+C.
        let cancel = self.state.cancel.clone();
        tokio::spawn(async move {
            tokio::signal::ctrl_c()
                .await
                .expect("failed to install Ctrl+C handler");
            cancel.cancel();
        });

        self.state.cancel.cancelled().await;
        info!("telegram-bot shutdown requested");

        // Graceful teardown: wait for all tasks.
        polling_handle
            .await
            .whatever_context("failed to join polling task")?;
        notify_consumer_handle
            .await
            .whatever_context("failed to join notify consumer task")?;
        settings_sync_handle
            .await
            .whatever_context("failed to join settings sync task")?;

        Ok(())
    }
}
