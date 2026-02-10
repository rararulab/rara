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
use snafu::Whatever;

mod build_info;
use job_app::AppConfig;
use job_telegram_bot::BotConfig;

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

impl ServerArgs {
    async fn run() -> Result<(), Whatever> {
        let _guards = job_common_telemetry::logging::init_tracing_subscriber("job");
        let config = AppConfig::from_env();
        let app = config.open().await?;
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
        let config = BotConfig::from_env();
        let bot = config.open().await?;
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
        let app = AppConfig::from_env().open().await?;
        let bot = BotConfig::from_env().open().await?;

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
