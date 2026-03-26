use snafu::Whatever;

use super::prompt::{self, SetupMode};

/// User configuration result.
pub struct UserResult {
    /// Username.
    pub name:             String,
    /// User role (e.g. "root", "admin", "user").
    pub role:             String,
    /// Optional Telegram user ID for platform identity mapping.
    pub telegram_user_id: Option<String>,
}

/// Guide the user through identity configuration.
///
/// When `existing_users_count` is non-zero and mode is `FillMissing`, the step
/// is skipped entirely. Otherwise the wizard collects one or more user entries
/// with name, role, and optional Telegram binding.
pub async fn setup_users(
    existing_users_count: usize,
    mode: SetupMode,
) -> Result<Option<Vec<UserResult>>, Whatever> {
    prompt::print_step("User Identity");

    if mode == SetupMode::FillMissing && existing_users_count > 0 {
        prompt::print_ok("already configured, skipping");
        return Ok(None);
    }

    let mut users = Vec::new();
    loop {
        let name = prompt::ask("Username", None);

        if name.trim().is_empty() {
            prompt::print_err("username cannot be empty");
            continue;
        }

        let role = prompt::ask("Role", Some("root"));

        let tg_id = prompt::ask("Telegram User ID (optional, press Enter to skip)", None);
        let telegram_user_id = if tg_id.is_empty() { None } else { Some(tg_id) };

        prompt::print_ok(&format!("user {name} configured"));
        users.push(UserResult {
            name,
            role,
            telegram_user_id,
        });

        if !prompt::confirm("Add another user?", false) {
            break;
        }
    }

    Ok(Some(users))
}
