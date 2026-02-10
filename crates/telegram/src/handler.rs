use teloxide::{prelude::*, utils::command::BotCommands};

#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase", description = "Available commands:")]
pub enum Command {
    #[command(description = "Start the bot")]
    Start,
    #[command(description = "Show help")]
    Help,
}

pub async fn handle_command(bot: Bot, msg: Message, cmd: Command) -> ResponseResult<()> {
    match cmd {
        Command::Start => {
            bot.send_message(
                msg.chat.id,
                "Welcome! I'm the Job Assistant bot. Use /help to see available commands.",
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

pub async fn handle_message(bot: Bot, msg: Message) -> ResponseResult<()> {
    if let Some(text) = msg.text() {
        bot.send_message(msg.chat.id, format!("Received: {text}"))
            .await?;
    }
    Ok(())
}
