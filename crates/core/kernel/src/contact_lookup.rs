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

//! Trait for looking up contacts from an allowlist.
//!
//! This abstracts the concrete `ContactRepository` behind a trait so that
//! tool implementations do not depend on the telegram-bot crate directly.

use async_trait::async_trait;

/// Resolved contact from the allowlist.
pub struct ResolvedContact {
    pub username: String,
    pub chat_id:  Option<i64>,
    pub enabled:  bool,
}

/// Trait for looking up contacts in the allowlist.
///
/// Implemented by telegram-bot's `ContactRepository`.
#[async_trait]
pub trait ContactLookup: Send + Sync {
    async fn find_by_username(&self, username: &str) -> anyhow::Result<Option<ResolvedContact>>;
}
