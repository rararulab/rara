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

//! Repository trait for notification persistence.

use uuid::Uuid;

use crate::{
    error::NotifyError,
    types::{Notification, NotificationFilter, NotificationStatistics},
};

#[async_trait::async_trait]
pub trait NotificationRepository: Send + Sync {
    async fn save(&self, notification: &Notification) -> Result<Notification, NotifyError>;
    async fn find_by_id(&self, id: Uuid) -> Result<Option<Notification>, NotifyError>;
    async fn find_all(&self, filter: &NotificationFilter)
    -> Result<Vec<Notification>, NotifyError>;
    async fn update(&self, notification: &Notification) -> Result<Notification, NotifyError>;
    async fn find_pending(&self, limit: i64) -> Result<Vec<Notification>, NotifyError>;
    async fn mark_sent(&self, id: Uuid) -> Result<(), NotifyError>;
    async fn mark_failed(&self, id: Uuid, error: &str) -> Result<(), NotifyError>;
    async fn increment_retry(&self, id: Uuid) -> Result<Notification, NotifyError>;
    async fn get_statistics(&self) -> Result<NotificationStatistics, NotifyError>;
}
