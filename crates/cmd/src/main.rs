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
use snafu::{ResultExt, Whatever};

mod build_info;

use rara_app::AppConfig;

#[derive(Debug, Parser)]
#[clap(
    name = "job",
    about = "raracli",
    author = build_info::AUTHOR,
    version = build_info::FULL_VERSION
)]
struct Cli {
    #[command(subcommand)]
    commands: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    Server(ServerArgs),
}

#[derive(Debug, Clone, Args)]
#[command(flatten_help = true)]
#[command(about = "Start the job server")]
#[command(long_about = "Start the job server with all services.\n\nExamples:\n  job server")]
struct ServerArgs {}

impl ServerArgs {
    async fn run() -> Result<(), Whatever> {
        // Load config first (Consul KV or env vars) so observability
        // settings are available before initialising the tracing subscriber.
        let config = AppConfig::new()
            .await
            .whatever_context("Failed to load config")?;

        // Priority: Langfuse (OTLP + auth) > general OTLP endpoint > no OTLP.
        let logging_opts =
            if config.langfuse.public_key.is_some() && config.langfuse.secret_key.is_some() {
                common_telemetry::logging::build_langfuse_logging_options(
                    Some(&config.langfuse.host),
                    config.langfuse.public_key.as_deref(),
                    config.langfuse.secret_key.as_deref(),
                )
            } else if let Some(ref endpoint) = config.telemetry.otlp_endpoint {
                use common_telemetry::logging::{LoggingOptions, OtlpExportProtocol};
                let protocol = config.telemetry.otlp_protocol.as_deref().map(|p| match p {
                    "grpc" => OtlpExportProtocol::Grpc,
                    _ => OtlpExportProtocol::Http,
                });
                LoggingOptions {
                    enable_otlp_tracing: true,
                    otlp_endpoint: Some(endpoint.clone()),
                    otlp_export_protocol: protocol,
                    ..Default::default()
                }
            } else {
                common_telemetry::logging::LoggingOptions::default()
            };

        let _guards = common_telemetry::logging::init_global_logging(
            "rara",
            &logging_opts,
            &common_telemetry::logging::TracingOptions::default(),
            None,
        );

        config.run().await
    }
}

#[tokio::main]
async fn main() -> Result<(), Whatever> {
    let cli = Cli::parse();
    match cli.commands {
        Commands::Server(_) => ServerArgs::run().await,
    }
}
