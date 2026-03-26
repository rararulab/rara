//! Interactive configuration wizard for rara.

mod prompt;

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

        // Suppress unused warnings until subsequent tasks wire these in.
        drop(existing_config);
        let _ = mode;

        // TODO: setup steps will be added in subsequent tasks
        // db::setup_database(...)
        // llm::setup_llm(...)
        // telegram::setup_telegram(...)
        // user::setup_users(...)
        // stt::setup_stt(...)
        // writer::write_config(...)

        println!("\n=== Setup complete ===");
        Ok(())
    }
}
