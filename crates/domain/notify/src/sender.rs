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

//! Notification sender implementations.

use async_trait::async_trait;
use tracing::info;

use crate::{error::NotifyError, service::NotificationSender, types::Notification};

/// A no-op sender that logs but does not actually deliver notifications.
/// Useful for development and testing environments.
pub struct NoopSender;

#[async_trait]
impl NotificationSender for NoopSender {
    async fn send(&self, notification: &Notification) -> Result<(), NotifyError> {
        info!(
            id = %notification.id,
            channel = ?notification.channel,
            recipient = %notification.recipient,
            "noop: notification would be sent"
        );
        Ok(())
    }
}
