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
use rsketch_common_runtime::{GlobalRuntimeOptions, block_on_background, init_global_runtimes};
use snafu::Whatever;

mod build_info;
use rsketch_app::AppConfig;

#[derive(Debug, Parser)]
#[clap(
name = "rsketch",
about= "rsketch-cmd",
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
}

#[derive(Debug, Clone, Args)]
#[command(flatten_help = true)]
#[command(about = "Print hello")]
#[command(long_about = "Print hello.\n\nExamples:\n  rsketch hello")]
struct HelloArgs {}

impl HelloArgs {
    fn run() {
        println!("Hello, world!");
    }
}

#[derive(Debug, Clone, Args)]
#[command(flatten_help = true)]
#[command(about = "Start the rsketch server")]
#[command(long_about = "Start the rsketch server.\n\nExamples:\n  rsketch server")]
struct ServerArgs {}

impl ServerArgs {
    fn run() -> Result<(), Whatever> {
        let app = AppConfig::default().open();
        init_global_runtimes(&GlobalRuntimeOptions::default());
        block_on_background(app.run())
    }
}

fn main() -> Result<(), Whatever> {
    let cli = Cli::parse();
    match cli.commands {
        Commands::Hello(_) => {
            HelloArgs::run();
            Ok(())
        }
        Commands::Server(_) => ServerArgs::run(),
    }
}
