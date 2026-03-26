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

use snafu::{ResultExt, Whatever};
use yunara_store::config::DatabaseConfig;

use super::prompt::{self, SetupMode};

/// Database setup result.
pub struct DbResult {
    /// The resolved SQLite database URL.
    pub database_url:    String,
    /// Number of applied migrations.
    pub migration_count: usize,
}

/// Guide the user through database setup: create directory, connect, and
/// run pending migrations.
pub async fn setup_database(mode: SetupMode) -> Result<Option<DbResult>, Whatever> {
    prompt::print_step("Database (SQLite)");

    let db_dir = rara_paths::database_dir();
    let default_url = format!("sqlite:{}/rara.db?mode=rwc", db_dir.display());

    if mode == SetupMode::FillMissing && db_dir.join("rara.db").exists() {
        prompt::print_ok("database file already exists, skipping");
        return Ok(None);
    }

    loop {
        let url = prompt::ask("SQLite URL", Some(&default_url));

        match validate_database(&url).await {
            Ok(count) => {
                prompt::print_ok(&format!("connected, {count} migrations applied"));
                return Ok(Some(DbResult {
                    database_url:    url,
                    migration_count: count,
                }));
            }
            Err(e) => {
                prompt::print_err(&format!("database setup failed: {e}"));
                let choice = prompt::ask_choice("What to do?", &["Retry", "Skip", "Exit"]);
                match choice {
                    0 => continue,
                    1 => return Ok(None),
                    _ => std::process::exit(1),
                }
            }
        }
    }
}

/// Open the SQLite database and run pending migrations.
async fn validate_database(url: &str) -> Result<usize, Whatever> {
    // Ensure the database directory exists.
    let db_dir = rara_paths::database_dir();
    std::fs::create_dir_all(db_dir).whatever_context("failed to create database directory")?;

    let config = DatabaseConfig::builder().build();
    let db_store = config
        .open(url)
        .await
        .whatever_context("failed to open SQLite database")?;

    sqlx::migrate!("../rara-model/migrations")
        .run(db_store.pool())
        .await
        .whatever_context("migration failed")?;

    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM _sqlx_migrations")
        .fetch_one(db_store.pool())
        .await
        .whatever_context("failed to count migrations")?;

    Ok(count.0 as usize)
}
