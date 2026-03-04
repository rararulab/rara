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

//! SQLite repository for telegram contacts.

use snafu::{IntoError, ResultExt};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::telegram::contacts::{
    error::{ContactError, DuplicateUsernameSnafu, NotFoundSnafu, RepositorySnafu},
    types::{CreateContactRequest, TelegramContact, UpdateContactRequest},
};

#[derive(Clone)]
pub struct ContactRepository {
    pool: SqlitePool,
}

impl ContactRepository {
    pub fn new(pool: SqlitePool) -> Self { Self { pool } }

    pub async fn list(&self) -> Result<Vec<TelegramContact>, ContactError> {
        sqlx::query_as::<_, TelegramContact>(
            "SELECT id, name, telegram_username, chat_id, notes, enabled, created_at, updated_at \
             FROM telegram_contact ORDER BY name",
        )
        .fetch_all(&self.pool)
        .await
        .context(RepositorySnafu)
    }

    pub async fn get(&self, id: Uuid) -> Result<TelegramContact, ContactError> {
        sqlx::query_as::<_, TelegramContact>(
            "SELECT id, name, telegram_username, chat_id, notes, enabled, created_at, updated_at \
             FROM telegram_contact WHERE id = ?1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .context(RepositorySnafu)?
        .ok_or(ContactError::NotFound { id })
    }

    pub async fn create(&self, req: CreateContactRequest) -> Result<TelegramContact, ContactError> {
        let username = req.telegram_username.trim_start_matches('@').to_lowercase();
        if username.is_empty() {
            return Err(ContactError::Validation {
                message: "telegram_username must not be empty".to_owned(),
            });
        }
        if req.name.trim().is_empty() {
            return Err(ContactError::Validation {
                message: "name must not be empty".to_owned(),
            });
        }

        let enabled = req.enabled.unwrap_or(true);

        sqlx::query_as::<_, TelegramContact>(
            "INSERT INTO telegram_contact (name, telegram_username, chat_id, notes, enabled) \
             VALUES (?1, ?2, ?3, ?4, ?5) RETURNING id, name, telegram_username, chat_id, notes, \
             enabled, created_at, updated_at",
        )
        .bind(req.name.trim())
        .bind(&username)
        .bind(req.chat_id)
        .bind(req.notes.as_deref())
        .bind(enabled)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            if is_unique_violation(&e) {
                DuplicateUsernameSnafu { username }.build()
            } else {
                RepositorySnafu.into_error(e)
            }
        })
    }

    pub async fn update(
        &self,
        id: Uuid,
        req: UpdateContactRequest,
    ) -> Result<TelegramContact, ContactError> {
        let existing = self.get(id).await?;

        let name = req.name.as_deref().unwrap_or(&existing.name);
        let username = req
            .telegram_username
            .as_deref()
            .map(|u| u.trim_start_matches('@').to_lowercase())
            .unwrap_or_else(|| existing.telegram_username.clone());
        let chat_id = req.chat_id.or(existing.chat_id);
        let notes = req.notes.or(existing.notes);
        let enabled = req.enabled.unwrap_or(existing.enabled);

        sqlx::query_as::<_, TelegramContact>(
            "UPDATE telegram_contact SET name = ?2, telegram_username = ?3, chat_id = ?4, notes = \
             ?5, enabled = ?6 WHERE id = ?1 RETURNING id, name, telegram_username, chat_id, \
             notes, enabled, created_at, updated_at",
        )
        .bind(id)
        .bind(name.trim())
        .bind(&username)
        .bind(chat_id)
        .bind(notes.as_deref())
        .bind(enabled)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            if is_unique_violation(&e) {
                DuplicateUsernameSnafu { username }.build()
            } else {
                RepositorySnafu.into_error(e)
            }
        })
    }

    pub async fn delete(&self, id: Uuid) -> Result<(), ContactError> {
        let rows = sqlx::query("DELETE FROM telegram_contact WHERE id = ?1")
            .bind(id)
            .execute(&self.pool)
            .await
            .context(RepositorySnafu)?
            .rows_affected();

        if rows == 0 {
            return Err(NotFoundSnafu { id }.build());
        }
        Ok(())
    }

    /// Resolve a username to a contact row. Used by the allowlist check.
    pub async fn get_by_username(
        &self,
        username: &str,
    ) -> Result<Option<TelegramContact>, ContactError> {
        let normalized = username.trim_start_matches('@').to_lowercase();
        sqlx::query_as::<_, TelegramContact>(
            "SELECT id, name, telegram_username, chat_id, notes, enabled, created_at, updated_at \
             FROM telegram_contact WHERE telegram_username = ?1",
        )
        .bind(&normalized)
        .fetch_optional(&self.pool)
        .await
        .context(RepositorySnafu)
    }

    /// Update chat_id for a contact identified by username. Used by the bot
    /// when it sees a message from a known contact.
    pub async fn set_chat_id(&self, username: &str, chat_id: i64) -> Result<(), ContactError> {
        let normalized = username.trim_start_matches('@').to_lowercase();
        sqlx::query(
            "UPDATE telegram_contact SET chat_id = ?2 WHERE telegram_username = ?1 AND (chat_id \
             IS NULL OR chat_id != ?2)",
        )
        .bind(&normalized)
        .bind(chat_id)
        .execute(&self.pool)
        .await
        .context(RepositorySnafu)?;
        Ok(())
    }
}

fn is_unique_violation(e: &sqlx::Error) -> bool {
    if let sqlx::Error::Database(db_err) = e {
        // SQLite UNIQUE constraint violation code
        return db_err.code().as_deref() == Some("2067");
    }
    false
}
