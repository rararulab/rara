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

mod app;
mod client;
mod types;
mod ui;

use clap::Args;

use crate::top::{app::App, client::KernelClient};

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
