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

use clap::{Args, Parser, Subcommand};
use job_common_telemetry;
use snafu::{ResultExt, Whatever};

mod build_info;

use job_app::settings::Settings;
use job_telegram_bot::{BotConfig, TelegramConfig};

#[derive(Debug, Parser)]
#[clap(
name = "job",
about= "job-cli",
author = build_info::AUTHOR,
version = build_info::FULL_VERSION)]
struct Cli {
    #[command(subcommand)]
    commands: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    Hello(HelloArgs),
    Server(ServerArgs),
    Bot(BotArgs),
    Combined(CombinedArgs),
}

#[derive(Debug, Clone, Args)]
#[command(flatten_help = true)]
#[command(about = "Print hello")]
#[command(long_about = "Print hello.\n\nExamples:\n  job hello")]
struct HelloArgs {}

impl HelloArgs {
    fn run() {
        println!("Hello, world!");
    }
}

#[derive(Debug, Clone, Args)]
#[command(flatten_help = true)]
#[command(about = "Start the job server")]
#[command(long_about = "Start the job server.\n\nExamples:\n  job server")]
struct ServerArgs {}

fn load_settings() -> Result<Settings, Whatever> {
    Settings::new().whatever_context("Failed to load settings")
}

/// Convert [`Settings`] into [`BotConfig`] for telegram-bot.
fn settings_to_bot_config(settings: &Settings) -> BotConfig {
    let db_config = settings.database.clone().into_database_config();
    let telegram = settings.telegram.as_ref().map(|t| TelegramConfig {
        bot_token: t.bot_token.clone(),
        chat_id:   t.chat_id,
    });
    BotConfig {
        db_config,
        telegram,
        main_service_http_base: settings.main_service_http_base.clone(),
    }
}

impl ServerArgs {
    async fn run() -> Result<(), Whatever> {
        let _guards = job_common_telemetry::logging::init_tracing_subscriber("job");
        let settings = load_settings()?;
        let app = settings.into_app_config().open().await?;
        app.run().await
    }
}

#[derive(Debug, Clone, Args)]
#[command(flatten_help = true)]
#[command(about = "Start standalone telegram-bot service")]
#[command(long_about = "Start standalone telegram-bot service.\n\nExamples:\n  job bot")]
struct BotArgs {}

impl BotArgs {
    async fn run() -> Result<(), Whatever> {
        let _guards = job_common_telemetry::logging::init_tracing_subscriber("job-bot");
        let settings = load_settings()?;
        let bot_config = settings_to_bot_config(&settings);
        let bot = bot_config.open().await?;
        bot.run().await
    }
}

#[derive(Debug, Clone, Args)]
#[command(flatten_help = true)]
#[command(about = "Start main service and telegram-bot in one process")]
#[command(
    long_about = "Start main service and telegram-bot in one process.\n\nExamples:\n  job combined"
)]
struct CombinedArgs {}

impl CombinedArgs {
    async fn run() -> Result<(), Whatever> {
        let _guards = job_common_telemetry::logging::init_tracing_subscriber("job-combined");
        let settings = load_settings()?;
        let bot_config = settings_to_bot_config(&settings);
        let app = settings.into_app_config().open().await?;
        let bot = bot_config.open().await?;

        let (app_res, bot_res) = tokio::join!(app.run(), bot.run());
        app_res?;
        bot_res?;
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<(), Whatever> {
    let cli = Cli::parse();
    match cli.commands {
        Commands::Hello(_) => {
            HelloArgs::run();
            Ok(())
        }
        Commands::Server(_) => ServerArgs::run().await,
        Commands::Bot(_) => BotArgs::run().await,
        Commands::Combined(_) => CombinedArgs::run().await,
    }
}
