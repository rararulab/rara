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

//! Manual `getUpdates` long-polling loop.
//!
//! We use a hand-rolled polling loop instead of teloxide's built-in
//! `Dispatcher` for three reasons:
//!
//! 1. **Error recovery** — on transient failures we sleep 5 seconds and retry,
//!    rather than crashing the entire dispatcher.
//! 2. **Conflict detection** — if another bot instance is running with the same
//!    token, the `TerminatedByOtherGetUpdates` API error is caught and the loop
//!    exits gracefully.
//! 3. **Cancellation** — `tokio::select!` on the [`CancellationToken`] lets the
//!    loop exit mid-wait during shutdown, instead of blocking for the full
//!    30-second poll timeout.
//!
//! The HTTP client timeout (45s) is intentionally higher than the Telegram
//! long-poll timeout (30s) to prevent the client from aborting the request
//! before Telegram responds.

use std::sync::Arc;

use snafu::{ResultExt, Snafu};
use teloxide::{
    payloads::GetUpdatesSetters,
    requests::{Request, Requester},
    types::AllowedUpdate,
};
use tracing::{error, info, warn};

use crate::{handlers, state::BotState};

/// Long-polling timeout in seconds (Telegram server-side wait).
const POLL_TIMEOUT_SECS: u32 = 30;

/// Error retry delay.
const ERROR_RETRY_DELAY: std::time::Duration = std::time::Duration::from_secs(5);

/// Errors from the polling loop.
#[derive(Debug, Snafu)]
pub(crate) enum PollingError {
    #[snafu(display("failed to delete webhook: {source}"))]
    DeleteWebhook { source: teloxide::RequestError },

    #[snafu(display("failed to verify bot token via getMe: {source}"))]
    GetMe { source: teloxide::RequestError },
}

/// Initialize the bot: delete webhook and verify token via `getMe`.
///
/// Returns the bot username on success.
pub(crate) async fn initialize(bot: &teloxide::Bot) -> Result<Option<String>, PollingError> {
    // Clear any existing webhook to ensure getUpdates works.
    bot.delete_webhook().await.context(DeleteWebhookSnafu)?;
    info!("webhook cleared");

    // Verify token and get bot info.
    let me = bot.get_me().await.context(GetMeSnafu)?;
    let username = me.username.clone();
    info!(
        bot_id = me.id.0,
        bot_username = ?username,
        "bot identity verified"
    );

    Ok(username)
}

/// Run the manual `getUpdates` long-polling loop.
///
/// This function blocks until the cancellation token is triggered or an
/// unrecoverable error (like `TerminatedByOtherGetUpdates`) is detected.
pub(crate) async fn start_polling(state: Arc<BotState>) {
    let mut offset: Option<i32> = None;

    info!("starting manual getUpdates polling loop");

    loop {
        // Check for cancellation before each poll.
        if state.cancel.is_cancelled() {
            info!("polling loop cancelled");
            break;
        }

        let mut request = state
            .bot
            .get_updates()
            .timeout(POLL_TIMEOUT_SECS)
            .allowed_updates(vec![AllowedUpdate::Message, AllowedUpdate::CallbackQuery]);

        if let Some(off) = offset {
            request = request.offset(off);
        }

        // Use select to allow cancellation during the long poll.
        let result = tokio::select! {
            () = state.cancel.cancelled() => {
                info!("polling cancelled during getUpdates wait");
                break;
            }
            result = request.send() => result,
        };

        match result {
            Ok(updates) => {
                for update in updates {
                    // Advance offset past this update.
                    #[allow(clippy::cast_possible_wrap)]
                    let next_offset = update.id.0 as i32 + 1;
                    offset = Some(next_offset);
                    // Spawn handler as a separate task so the polling loop
                    // is never blocked by slow operations (e.g. LLM calls).
                    let st = Arc::clone(&state);
                    tokio::spawn(async move {
                        handlers::handle_update(update, &st).await;
                    });
                }
            }
            Err(teloxide::RequestError::Api(ref api_err)) => {
                // Check for conflict with another getUpdates instance.
                let err_str = format!("{api_err}");
                if err_str.contains("terminated by other getUpdates request") {
                    warn!("another bot instance is running — this instance will exit");
                    break;
                }
                error!(error = %api_err, "telegram API error in getUpdates");
                tokio::time::sleep(ERROR_RETRY_DELAY).await;
            }
            Err(e) => {
                error!(error = %e, "getUpdates request failed");
                tokio::time::sleep(ERROR_RETRY_DELAY).await;
            }
        }
    }

    info!("polling loop stopped");
}
