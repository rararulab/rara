mod app;
mod client;
mod types;
mod ui;

use clap::Args;

use crate::top::app::App;
use crate::top::client::KernelClient;

#[derive(Debug, Clone, Args)]
#[command(about = "Real-time TUI dashboard for kernel observability")]
pub struct TopCmd {
    /// Kernel HTTP API base URL.
    #[arg(long, default_value = "http://localhost:25555")]
    url: String,
}

impl TopCmd {
    pub async fn run(self) -> Result<(), snafu::Whatever> {
        use snafu::ResultExt;

        let client = KernelClient::new(self.url);
        let mut app = App::new();

        // Initialize the terminal.
        let mut terminal = ratatui::init();

        // Run the event loop; restore terminal on any exit path.
        let result = app.run(&mut terminal, &client).await;

        // Restore the terminal no matter what.
        ratatui::restore();

        result.whatever_context("TUI error")
    }
}
