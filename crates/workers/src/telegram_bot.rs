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

//! Telegram bot worker — receives messages and delegates JD parsing.

use async_trait::async_trait;
use job_common_worker::{FallibleWorker, WorkResult, WorkerContext};
use teloxide::prelude::*;
use teloxide::utils::command::BotCommands;
use tokio::sync::mpsc;

use crate::notification_processor::WorkerState;
use crate::types::JdParseRequest;

/// Long-running worker that starts the Telegram bot dispatcher.
///
/// Spawned with a `Once` trigger — it runs the teloxide long-polling
/// loop until cancelled.
pub struct TelegramBotWorker;

#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase", description = "Available commands:")]
enum Command {
    #[command(description = "Start the bot")]
    Start,
    #[command(description = "Show help")]
    Help,
}

#[async_trait]
impl FallibleWorker<WorkerState> for TelegramBotWorker {
    async fn work(&mut self, ctx: WorkerContext<WorkerState>) -> WorkResult {
        let state = ctx.state();
        let bot = match &state.bot {
            Some(b) => b.clone(),
            None => return Ok(()),
        };
        let jd_tx = state.jd_parse_tx.clone();

        let handler = Update::filter_message()
            .branch(
                dptree::entry()
                    .filter_command::<Command>()
                    .endpoint(handle_command),
            )
            .branch(dptree::entry().endpoint(handle_message));

        // Drop any pending updates accumulated while the bot was offline,
        // so we don't replay stale messages on every restart.
        let _ = bot.delete_webhook().drop_pending_updates(true).await;

        let mut dispatcher = Dispatcher::builder(bot, handler)
            .dependencies(dptree::deps![jd_tx])
            .enable_ctrlc_handler()
            .build();

        // Graceful shutdown: when the worker context is cancelled,
        // shut down the teloxide dispatcher.
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

async fn handle_command(bot: Bot, msg: Message, cmd: Command) -> ResponseResult<()> {
    match cmd {
        Command::Start => {
            bot.send_message(
                msg.chat.id,
                "Welcome! I'm the Job Assistant bot. Send me a job description \
                 and I'll parse it for you. Use /help to see commands.",
            )
            .await?;
        }
        Command::Help => {
            bot.send_message(msg.chat.id, Command::descriptions().to_string())
                .await?;
        }
    }
    Ok(())
}

async fn handle_message(
    bot: Bot,
    msg: Message,
    jd_tx: mpsc::Sender<JdParseRequest>,
) -> ResponseResult<()> {
    if let Some(text) = msg.text() {
        bot.send_message(msg.chat.id, "Received your JD, processing...")
            .await?;

        let _ = jd_tx
            .send(JdParseRequest {
                chat_id: msg.chat.id.0,
                text:    text.to_string(),
            })
            .await;
    }
    Ok(())
}
