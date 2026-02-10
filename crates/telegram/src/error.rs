use snafu::Snafu;

#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum BotError {
    #[snafu(display("Telegram bot error: {message}"))]
    TelegramError { message: String },
}
