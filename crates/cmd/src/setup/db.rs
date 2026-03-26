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

    let url = prompt::ask("SQLite URL", Some(&default_url));

    match validate_database(&url).await {
        Ok(count) => {
            prompt::print_ok(&format!("connected, {count} migrations applied"));
            Ok(Some(DbResult {
                database_url:    url,
                migration_count: count,
            }))
        }
        Err(e) => {
            prompt::print_err(&format!("database setup failed: {e}"));
            let choice = prompt::ask_choice("What to do?", &["Retry", "Skip", "Exit"]);
            match choice {
                0 => Box::pin(setup_database(mode)).await,
                1 => Ok(None),
                _ => std::process::exit(1),
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
