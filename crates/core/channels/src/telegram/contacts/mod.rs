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

//! Telegram contacts: allowlist, tracking, and CRUD.
//!
//! - [`ContactTracker`] — trait for recording username→chat_id mappings
//! - [`repository`] — PostgreSQL CRUD for the `telegram_contact` table
//! - [`types`] — domain types (`TelegramContact`, request DTOs)
//! - [`error`] — error types with `IntoResponse`

pub mod error;
pub mod repository;
mod tracker;
pub mod types;

pub use tracker::{ContactTracker, NoopContactTracker};
