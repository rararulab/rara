// Copyright 2025 Crrow
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

//! Telegram contact types.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A persistent Telegram contact record used as an allowlist for outbound
/// messaging.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct TelegramContact {
    pub id:                Uuid,
    pub name:              String,
    pub telegram_username: String,
    pub chat_id:           Option<i64>,
    pub notes:             Option<String>,
    pub enabled:           bool,
    #[schema(value_type = String)]
    pub created_at:        DateTime<Utc>,
    #[schema(value_type = String)]
    pub updated_at:        DateTime<Utc>,
}

/// Request payload to create a new contact.
#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct CreateContactRequest {
    pub name:              String,
    pub telegram_username: String,
    pub chat_id:           Option<i64>,
    pub notes:             Option<String>,
    pub enabled:           Option<bool>,
}

/// Request payload to update an existing contact.
#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct UpdateContactRequest {
    pub name:              Option<String>,
    pub telegram_username: Option<String>,
    pub chat_id:           Option<i64>,
    pub notes:             Option<String>,
    pub enabled:           Option<bool>,
}
