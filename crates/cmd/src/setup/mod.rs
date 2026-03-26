//! Interactive configuration wizard for rara.

mod db;
mod llm;
mod prompt;
mod stt;
mod telegram;
mod user;

use clap::Args;
pub use prompt::SetupMode;
use snafu::Whatever;

/// Interactive setup wizard -- configure database, LLM, Telegram, and more.
#[derive(Debug, Clone, Args)]
#[command(about = "Interactive setup wizard -- configure database, LLM, Telegram, and more")]
pub struct SetupCmd;

impl SetupCmd {
    /// Run the interactive setup wizard.
    pub async fn run(self) -> Result<(), Whatever> {
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

        let _db_result = db::setup_database(mode).await?;

        let _llm_result =
            llm::setup_llm(existing_config.as_ref().and_then(|c| c.llm.as_ref()), mode).await?;

        let _telegram_result = telegram::setup_telegram(
            existing_config.as_ref().and_then(|c| c.telegram.as_ref()),
            mode,
        )
        .await?;

        let _user_result =
            user::setup_users(existing_config.as_ref().map_or(0, |c| c.users.len()), mode).await?;

        let _stt_result = stt::setup_stt(mode).await?;

        // TODO: remaining setup steps
        // writer::write_config(...)

        println!("\n=== Setup complete ===");
        Ok(())
    }
}
