// Copyright 2026 Crrow
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

//! Telegram outbox repository.
//!
//! This table is bot-owned and stores bot delivery attempts to avoid
//! transport-level data mixing with domain notification records.

use sqlx::PgPool;
use uuid::Uuid;

#[derive(Clone)]
pub(crate) struct TelegramOutboxRepository {
    pool: PgPool,
}

impl TelegramOutboxRepository {
    /// Build repository from a shared postgres pool.
    pub(crate) fn new(pool: PgPool) -> Self { Self { pool } }

    /// Insert a pending outbox item and return its generated id.
    pub(crate) async fn enqueue(
        &self,
        chat_id: i64,
        text: &str,
        source: &str,
    ) -> Result<Uuid, sqlx::Error> {
        let id = Uuid::new_v4();
        sqlx::query(
            r#"INSERT INTO telegram_outbox
               (id, chat_id, text, source, status, error_message, sent_at, created_at, updated_at)
               VALUES ($1, $2, $3, $4, 0, NULL, NULL, now(), now())"#,
        )
        .bind(id)
        .bind(chat_id)
        .bind(text)
        .bind(source)
        .execute(&self.pool)
        .await?;
        Ok(id)
    }

    /// Mark an outbox item as sent and attach `sent_at`.
    pub(crate) async fn mark_sent(&self, id: Uuid) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"UPDATE telegram_outbox
               SET status = 1, sent_at = now(), error_message = NULL, updated_at = now()
               WHERE id = $1"#,
        )
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Mark an outbox item as failed and persist provider error message.
    pub(crate) async fn mark_failed(&self, id: Uuid, err: &str) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"UPDATE telegram_outbox
               SET status = 2, error_message = $2, updated_at = now()
               WHERE id = $1"#,
        )
        .bind(id)
        .bind(err)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}
