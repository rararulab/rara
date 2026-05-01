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

//! `rara session-index <subcommand>` — the rescue toolbox for the
//! SQLite-backed session index introduced in issue #2025.
//!
//! The only operator-facing command today is `rebuild`, which scans the
//! on-disk tape for one (or all) sessions and overwrites the
//! derived-state row(s) in `sessions`. Use it after a crash, or after
//! manually editing a tape file out-of-band, to bring the index back in
//! sync with reality.

use std::sync::Arc;

use clap::{Args, Subcommand};
use rara_app::TapeReconciler;
use rara_kernel::{memory::TapeService, session::SessionKey};
use rara_sessions::sqlite_index::{ReconcileTape, SqliteSessionIndex};
use snafu::{ResultExt, Whatever, whatever};

#[derive(Debug, Clone, Args)]
#[command(about = "Maintain the SQLite-backed session index")]
pub struct SessionIndexCmd {
    #[command(subcommand)]
    pub action: SessionIndexAction,
}

#[derive(Debug, Clone, Subcommand)]
pub enum SessionIndexAction {
    /// Rebuild derived-state rows from on-disk tapes.
    Rebuild(RebuildArgs),
}

#[derive(Debug, Clone, Args)]
pub struct RebuildArgs {
    /// Rebuild only this session. Without `--key`, every session in the
    /// index is rebuilt.
    #[arg(long)]
    pub key: Option<String>,
}

impl SessionIndexCmd {
    pub async fn run(self) -> Result<(), Whatever> {
        match self.action {
            SessionIndexAction::Rebuild(args) => rebuild(args).await,
        }
    }
}

async fn rebuild(args: RebuildArgs) -> Result<(), Whatever> {
    let config = rara_app::AppConfig::new().whatever_context("Failed to load config")?;
    let pools = rara_app::open_pools_for_cli(&config)
        .await
        .whatever_context("Failed to open SQLite pools")?;

    let index = Arc::new(SqliteSessionIndex::new(pools.clone()));

    let workspace_path = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let store = rara_kernel::memory::FileTapeStore::new(rara_paths::memory_dir(), &workspace_path)
        .await
        .whatever_context("Failed to initialize FileTapeStore")?;
    let tape = TapeService::new(store);
    let reconciler = TapeReconciler { tape };

    match args.key {
        Some(raw) => {
            let key = SessionKey::try_from_raw(&raw).map_err(|e| {
                snafu::FromString::without_source(format!("invalid session key '{raw}': {e}"))
            })?;
            let Some(report) = reconciler.read_tape(&key).await else {
                whatever!("no on-disk tape found for session {raw}");
            };
            index
                .rebuild_session_with_report(&key, &report)
                .await
                .whatever_context("rebuild failed")?;
            println!("rebuilt session {raw}");
        }
        None => {
            let repaired = index
                .reconcile_all(reconciler)
                .await
                .whatever_context("reconcile_all failed")?;
            println!("rebuild complete: {repaired} session(s) repaired");
        }
    }
    Ok(())
}
