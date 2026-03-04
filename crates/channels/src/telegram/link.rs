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

//! Telegram account linking service.
//!
//! Handles the `/link` command in the Telegram adapter:
//! - `/link <code>` — web→TG direction: verify code and create platform
//!   identity
//! - `/link` (no code) — TG→web direction: generate a code for the user to
//!   verify on web

use rand::{Rng, distr::Alphanumeric};
use sqlx::SqlitePool;
use tracing::info;

/// Service for handling Telegram account linking operations.
///
/// Uses raw SqlitePool to avoid circular dependencies with the user domain crate.
#[derive(Clone)]
pub struct TelegramLinkService {
    pool:     SqlitePool,
    base_url: String,
}

impl TelegramLinkService {
    pub fn new(pool: SqlitePool, base_url: String) -> Self { Self { pool, base_url } }

    /// Handle `/link <code>` — web_to_tg direction.
    ///
    /// Verifies the link code, creates a platform identity binding for the
    /// Telegram user, and deletes the used code. Returns a user-facing message.
    pub async fn handle_link_code(
        &self,
        code: &str,
        chat_id: i64,
        display_name: Option<&str>,
    ) -> Result<String, String> {
        // Verify link code exists and is not expired.
        let row = sqlx::query_as::<_, LinkCodeRow>("SELECT * FROM link_codes WHERE code = ?1")
            .bind(code)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| format!("database error: {e}"))?
            .ok_or_else(|| "Invalid or expired link code.".to_string())?;

        if row.expires_at < chrono::Utc::now() {
            return Err("Link code has expired.".to_string());
        }

        if row.direction != "web_to_tg" {
            return Err("Invalid link code direction.".to_string());
        }

        // Create platform identity binding.
        let identity_id = uuid::Uuid::new_v4();
        sqlx::query(
            "INSERT INTO user_platform_identities (id, user_id, platform, platform_user_id, \
             display_name) VALUES (?1, ?2, 'telegram', ?3, ?4) ON CONFLICT (platform, \
             platform_user_id) DO UPDATE SET user_id = ?2, display_name = ?4",
        )
        .bind(identity_id)
        .bind(row.user_id)
        .bind(chat_id.to_string())
        .bind(display_name)
        .execute(&self.pool)
        .await
        .map_err(|e| format!("failed to create platform identity: {e}"))?;

        // Delete the used link code.
        sqlx::query("DELETE FROM link_codes WHERE code = ?1")
            .bind(code)
            .execute(&self.pool)
            .await
            .map_err(|e| format!("failed to delete link code: {e}"))?;

        info!(chat_id = %chat_id, "telegram account linked via web_to_tg");
        Ok("Account linked successfully!".to_string())
    }

    /// Handle `/link` (no code) — TG→web direction.
    ///
    /// Generates a 6-character link code with the chat_id stored in
    /// `platform_data`, then returns a URL message for the user to open
    /// in their browser.
    pub async fn handle_link_request(&self, chat_id: i64) -> Result<String, String> {
        let code = generate_random_code(6);
        let expires_at = chrono::Utc::now() + chrono::Duration::minutes(5);
        let platform_data = serde_json::json!({ "chat_id": chat_id });

        sqlx::query(
            "INSERT INTO link_codes (code, user_id, direction, platform_data, expires_at) VALUES \
             (?1, (SELECT id FROM kernel_users WHERE name = 'system'), ?2, ?3, ?4)",
        )
        .bind(&code)
        .bind("tg_to_web")
        .bind(&platform_data)
        .bind(expires_at)
        .execute(&self.pool)
        .await
        .map_err(|e| format!("failed to generate link code: {e}"))?;

        let url = format!("{}/link?tg_code={}", self.base_url, code);
        info!(chat_id = %chat_id, code = %code, "tg_to_web link code generated");

        Ok(format!(
            "To link your Telegram account, open this URL in your browser (expires in 5 \
             minutes):\n\n{}",
            url
        ))
    }
}

/// Generate a random alphanumeric code of the given length.
fn generate_random_code(len: usize) -> String {
    rand::rng()
        .sample_iter(&Alphanumeric)
        .take(len)
        .map(char::from)
        .collect()
}

/// DB row type for link_codes table.
#[derive(sqlx::FromRow)]
struct LinkCodeRow {
    #[allow(dead_code)]
    id:            uuid::Uuid,
    #[allow(dead_code)]
    code:          String,
    user_id:       uuid::Uuid,
    direction:     String,
    #[allow(dead_code)]
    platform_data: Option<serde_json::Value>,
    expires_at:    chrono::DateTime<chrono::Utc>,
    #[allow(dead_code)]
    created_at:    chrono::DateTime<chrono::Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_code_correct_length() {
        let code = generate_random_code(6);
        assert_eq!(code.len(), 6);
        assert!(code.chars().all(|c| c.is_ascii_alphanumeric()));
    }

    #[test]
    fn link_service_new() {
        // Verify construction doesn't panic (no real pool needed for unit test).
        let pool = SqlitePool::connect_lazy("sqlite::memory:").unwrap();
        let svc = TelegramLinkService::new(pool, "https://example.com".to_string());
        assert_eq!(svc.base_url, "https://example.com");
    }
}
