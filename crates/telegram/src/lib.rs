pub mod error;
pub mod handler;

use teloxide::prelude::*;
use tokio_util::sync::CancellationToken;

use crate::error::BotError;
use crate::handler::{Command, handle_command, handle_message};

pub struct TelegramBot {
    bot: Bot,
}

impl TelegramBot {
    pub fn new(bot_token: &str) -> Self {
        Self {
            bot: Bot::new(bot_token),
        }
    }

    /// Start the bot with long-polling. Runs until cancelled.
    pub async fn run(self, cancel: CancellationToken) -> Result<(), BotError> {
        let handler = Update::filter_message()
            .branch(
                dptree::entry()
                    .filter_command::<Command>()
                    .endpoint(handle_command),
            )
            .branch(dptree::entry().endpoint(handle_message));

        let mut dispatcher = Dispatcher::builder(self.bot, handler)
            .enable_ctrlc_handler()
            .build();

        // Obtain a shutdown token before dispatching so we can stop the
        // dispatcher when the application cancellation token fires.
        let shutdown_token = dispatcher.shutdown_token();

        tokio::spawn(async move {
            cancel.cancelled().await;
            if let Ok(f) = shutdown_token.shutdown() {
                f.await;
            }
        });

        dispatcher.dispatch().await;

        Ok(())
    }
}
