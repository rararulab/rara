// Copyright 2025 Rararulab
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

//! Interactive configuration wizard for rara.

mod db;
mod llm;
mod prompt;
mod stt;
mod telegram;
mod user;
mod writer;

use clap::{Args, Subcommand};
pub use prompt::SetupMode;
use snafu::Whatever;

/// Interactive setup wizard -- configure database, LLM, Telegram, and more.
#[derive(Debug, Clone, Args)]
#[command(about = "Interactive setup wizard -- configure database, LLM, Telegram, and more")]
pub struct SetupCmd {
    #[command(subcommand)]
    sub: Option<SetupSub>,
}

/// Available setup subcommands for configuring individual components.
#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
enum SetupSub {
    /// Configure whisper speech-to-text (STT) settings only.
    Whisper,
}

impl SetupCmd {
    /// Run the full setup wizard, or a specific subcommand.
    pub async fn run(self) -> Result<(), Whatever> {
        if self.sub == Some(SetupSub::Whisper) {
            return stt::run_whisper_setup().await;
        }

        println!("rara setup\n");

        let config_path = rara_paths::config_file();
        let (existing_config, mode) = if config_path.is_file() {
            println!("Detected existing config at {}", config_path.display());
            let mode_idx = prompt::ask_choice(
                "\nSelect mode:",
                &[
                    "Fresh setup (backup and reconfigure)",
                    "Modify existing (review each section, edit as needed)",
                    "Fill missing only (skip already-configured sections)",
                ],
            );
            let mode = match mode_idx {
                0 => SetupMode::Fresh,
                1 => SetupMode::Modify,
                _ => SetupMode::FillMissing,
            };
            let config = rara_app::AppConfig::new().ok();
            (config, mode)
        } else {
            println!("No existing config found. Starting fresh setup.");
            (None, SetupMode::Fresh)
        };

        let db_result = db::setup_database(mode).await?;

        let llm_result =
            llm::setup_llm(existing_config.as_ref().and_then(|c| c.llm.as_ref()), mode).await?;

        let telegram_result = telegram::setup_telegram(
            existing_config.as_ref().and_then(|c| c.telegram.as_ref()),
            mode,
        )
        .await?;

        let user_result =
            user::setup_users(existing_config.as_ref().map_or(0, |c| c.users.len()), mode).await?;

        let stt_result = stt::setup_stt(mode).await?;

        // Load base config for merging (FillMissing/Modify mode)
        let base_yaml = if mode == SetupMode::Fresh {
            None
        } else {
            std::fs::read_to_string(config_path)
                .ok()
                .and_then(|s| serde_yaml::from_str(&s).ok())
        };

        // Assemble final config
        let config_value = writer::assemble_config(
            base_yaml,
            db_result.as_ref().map(|d| d.database_url.as_str()),
            llm_result.as_ref(),
            telegram_result.as_ref(),
            user_result.as_deref(),
            stt_result.as_ref(),
        )?;

        let yaml = writer::to_yaml(&config_value)?;

        // Preview with secrets masked
        println!("\n═══ Config Preview ═══");
        println!("{}", writer::mask_secrets(&yaml));

        // Confirm and write
        if prompt::confirm("Write config?", true) {
            writer::write_config(config_path, &yaml)?;
        } else {
            println!("Aborted. Config not written.");
        }

        println!("\n═══ Setup complete ═══");
        Ok(())
    }
}
