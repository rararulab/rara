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

//! Notification service: queuing, sending, and retry logic.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::error::NotifyError;
use crate::repository::NotificationRepository;
use crate::types::{
    Notification, NotificationChannel, NotificationFilter, NotificationStatistics,
    NotificationStatus, ProcessResult, SendNotificationRequest,
};

/// Trait for notification channel backends (e.g. Telegram, Email).
#[async_trait::async_trait]
pub trait NotificationSender: Send + Sync {
    async fn send(&self, notification: &Notification) -> Result<(), NotifyError>;
}

pub struct NotificationService {
    repo: Arc<dyn NotificationRepository>,
    senders: HashMap<NotificationChannel, Arc<dyn NotificationSender>>,
}

impl NotificationService {
    pub fn new(repo: Arc<dyn NotificationRepository>) -> Self {
        Self {
            repo,
            senders: HashMap::new(),
        }
    }

    pub fn register_sender(
        &mut self,
        channel: NotificationChannel,
        sender: Arc<dyn NotificationSender>,
    ) {
        self.senders.insert(channel, sender);
    }

    pub async fn send(
        &self,
        req: SendNotificationRequest,
    ) -> Result<Notification, NotifyError> {
        if req.recipient.is_empty() {
            return Err(NotifyError::ValidationError {
                message: "recipient must not be empty".to_string(),
            });
        }
        if req.body.is_empty() {
            return Err(NotifyError::ValidationError {
                message: "body must not be empty".to_string(),
            });
        }

        let notification = Notification {
            id: Uuid::new_v4(),
            channel: req.channel,
            recipient: req.recipient,
            subject: req.subject,
            body: req.body,
            status: NotificationStatus::Pending,
            priority: req.priority,
            retry_count: 0,
            max_retries: 3,
            error_message: None,
            reference_type: req.reference_type,
            reference_id: req.reference_id,
            metadata: req.metadata,
            trace_id: None,
            sent_at: None,
            created_at: Utc::now(),
        };

        let saved = self.repo.save(&notification).await?;
        info!(id = %saved.id, channel = ?saved.channel, "notification queued");
        Ok(saved)
    }

    pub async fn process_pending(
        &self,
        batch_size: i64,
    ) -> Result<ProcessResult, NotifyError> {
        let pending = self.repo.find_pending(batch_size).await?;
        let mut result = ProcessResult::default();

        for notification in &pending {
            result.processed += 1;

            let sender = match self.senders.get(&notification.channel) {
                Some(s) => s,
                None => {
                    warn!(channel = ?notification.channel, "no sender registered for channel");
                    self.repo
                        .mark_failed(notification.id, "no sender registered for channel")
                        .await?;
                    result.failed += 1;
                    continue;
                }
            };

            match sender.send(notification).await {
                Ok(()) => {
                    self.repo.mark_sent(notification.id).await?;
                    result.succeeded += 1;
                    info!(id = %notification.id, "notification sent");
                }
                Err(e) => {
                    error!(id = %notification.id, error = %e, "notification send failed");
                    if notification.retry_count + 1 >= notification.max_retries {
                        self.repo
                            .mark_failed(notification.id, &e.to_string())
                            .await?;
                    } else {
                        self.repo.increment_retry(notification.id).await?;
                    }
                    result.failed += 1;
                }
            }
        }

        Ok(result)
    }

    pub async fn retry(&self, id: Uuid) -> Result<Notification, NotifyError> {
        let notification = self
            .repo
            .find_by_id(id)
            .await?
            .ok_or(NotifyError::NotFound { id })?;

        if notification.status != NotificationStatus::Failed {
            return Err(NotifyError::ValidationError {
                message: format!(
                    "can only retry failed notifications, current status: {:?}",
                    notification.status
                ),
            });
        }

        // Reset to pending for reprocessing
        let mut updated = notification;
        updated.status = NotificationStatus::Retrying;
        updated.error_message = None;

        self.repo.update(&updated).await
    }

    pub async fn list(
        &self,
        filter: &NotificationFilter,
    ) -> Result<Vec<Notification>, NotifyError> {
        self.repo.find_all(filter).await
    }

    pub async fn get_by_id(&self, id: Uuid) -> Result<Option<Notification>, NotifyError> {
        self.repo.find_by_id(id).await
    }

    pub async fn get_statistics(&self) -> Result<NotificationStatistics, NotifyError> {
        self.repo.get_statistics().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Mock repository for testing
    struct MockNotificationRepo {
        notifications: Mutex<Vec<Notification>>,
    }

    impl MockNotificationRepo {
        fn new() -> Self {
            Self {
                notifications: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait::async_trait]
    impl NotificationRepository for MockNotificationRepo {
        async fn save(&self, notification: &Notification) -> Result<Notification, NotifyError> {
            let mut store = self.notifications.lock().unwrap();
            store.push(notification.clone());
            Ok(notification.clone())
        }

        async fn find_by_id(&self, id: Uuid) -> Result<Option<Notification>, NotifyError> {
            let store = self.notifications.lock().unwrap();
            Ok(store.iter().find(|n| n.id == id).cloned())
        }

        async fn find_all(
            &self,
            filter: &NotificationFilter,
        ) -> Result<Vec<Notification>, NotifyError> {
            let store = self.notifications.lock().unwrap();
            let mut results: Vec<Notification> = store.clone();
            if let Some(ref channel) = filter.channel {
                results.retain(|n| n.channel == *channel);
            }
            if let Some(ref status) = filter.status {
                results.retain(|n| n.status == *status);
            }
            Ok(results)
        }

        async fn update(&self, notification: &Notification) -> Result<Notification, NotifyError> {
            let mut store = self.notifications.lock().unwrap();
            if let Some(existing) = store.iter_mut().find(|n| n.id == notification.id) {
                *existing = notification.clone();
                Ok(notification.clone())
            } else {
                Err(NotifyError::NotFound {
                    id: notification.id,
                })
            }
        }

        async fn find_pending(&self, limit: i64) -> Result<Vec<Notification>, NotifyError> {
            let store = self.notifications.lock().unwrap();
            Ok(store
                .iter()
                .filter(|n| {
                    n.status == NotificationStatus::Pending
                        || n.status == NotificationStatus::Retrying
                })
                .take(limit as usize)
                .cloned()
                .collect())
        }

        async fn mark_sent(&self, id: Uuid) -> Result<(), NotifyError> {
            let mut store = self.notifications.lock().unwrap();
            if let Some(n) = store.iter_mut().find(|n| n.id == id) {
                n.status = NotificationStatus::Sent;
                n.sent_at = Some(Utc::now());
                Ok(())
            } else {
                Err(NotifyError::NotFound { id })
            }
        }

        async fn mark_failed(&self, id: Uuid, error: &str) -> Result<(), NotifyError> {
            let mut store = self.notifications.lock().unwrap();
            if let Some(n) = store.iter_mut().find(|n| n.id == id) {
                n.status = NotificationStatus::Failed;
                n.error_message = Some(error.to_string());
                Ok(())
            } else {
                Err(NotifyError::NotFound { id })
            }
        }

        async fn increment_retry(&self, id: Uuid) -> Result<Notification, NotifyError> {
            let mut store = self.notifications.lock().unwrap();
            if let Some(n) = store.iter_mut().find(|n| n.id == id) {
                n.retry_count += 1;
                n.status = NotificationStatus::Retrying;
                Ok(n.clone())
            } else {
                Err(NotifyError::NotFound { id })
            }
        }

        async fn get_statistics(&self) -> Result<NotificationStatistics, NotifyError> {
            let store = self.notifications.lock().unwrap();
            let mut stats = NotificationStatistics::default();
            stats.total = store.len() as i64;
            for n in store.iter() {
                match n.status {
                    NotificationStatus::Pending => stats.pending += 1,
                    NotificationStatus::Sent => stats.sent += 1,
                    NotificationStatus::Failed => stats.failed += 1,
                    NotificationStatus::Retrying => stats.retrying += 1,
                }
            }
            Ok(stats)
        }
    }

    // Mock sender
    struct MockSender {
        should_fail: bool,
    }

    #[async_trait::async_trait]
    impl NotificationSender for MockSender {
        async fn send(&self, _notification: &Notification) -> Result<(), NotifyError> {
            if self.should_fail {
                Err(NotifyError::SendFailed {
                    channel: "mock".to_string(),
                    message: "mock failure".to_string(),
                })
            } else {
                Ok(())
            }
        }
    }

    fn make_request() -> SendNotificationRequest {
        SendNotificationRequest {
            channel: NotificationChannel::Telegram,
            recipient: "user123".to_string(),
            subject: Some("Test".to_string()),
            body: "Hello world".to_string(),
            priority: crate::types::NotificationPriority::Normal,
            reference_type: None,
            reference_id: None,
            metadata: None,
        }
    }

    #[tokio::test]
    async fn test_send_creates_pending_notification() {
        let repo = Arc::new(MockNotificationRepo::new());
        let service = NotificationService::new(repo);

        let result = service.send(make_request()).await.unwrap();
        assert_eq!(result.status, NotificationStatus::Pending);
        assert_eq!(result.channel, NotificationChannel::Telegram);
        assert_eq!(result.recipient, "user123");
    }

    #[tokio::test]
    async fn test_send_validates_empty_recipient() {
        let repo = Arc::new(MockNotificationRepo::new());
        let service = NotificationService::new(repo);

        let mut req = make_request();
        req.recipient = String::new();
        let result = service.send(req).await;
        assert!(matches!(result, Err(NotifyError::ValidationError { .. })));
    }

    #[tokio::test]
    async fn test_send_validates_empty_body() {
        let repo = Arc::new(MockNotificationRepo::new());
        let service = NotificationService::new(repo);

        let mut req = make_request();
        req.body = String::new();
        let result = service.send(req).await;
        assert!(matches!(result, Err(NotifyError::ValidationError { .. })));
    }

    #[tokio::test]
    async fn test_process_pending_with_successful_sender() {
        let repo = Arc::new(MockNotificationRepo::new());
        let mut service = NotificationService::new(repo.clone());
        service.register_sender(
            NotificationChannel::Telegram,
            Arc::new(MockSender {
                should_fail: false,
            }),
        );

        service.send(make_request()).await.unwrap();
        let result = service.process_pending(10).await.unwrap();
        assert_eq!(result.processed, 1);
        assert_eq!(result.succeeded, 1);
        assert_eq!(result.failed, 0);
    }

    #[tokio::test]
    async fn test_process_pending_with_failing_sender() {
        let repo = Arc::new(MockNotificationRepo::new());
        let mut service = NotificationService::new(repo.clone());
        service.register_sender(
            NotificationChannel::Telegram,
            Arc::new(MockSender { should_fail: true }),
        );

        service.send(make_request()).await.unwrap();
        let result = service.process_pending(10).await.unwrap();
        assert_eq!(result.processed, 1);
        assert_eq!(result.succeeded, 0);
        assert_eq!(result.failed, 1);
    }

    #[tokio::test]
    async fn test_process_pending_no_sender_registered() {
        let repo = Arc::new(MockNotificationRepo::new());
        let service = NotificationService::new(repo.clone());

        // Send via Email but no email sender registered
        let mut req = make_request();
        req.channel = NotificationChannel::Email;
        service.send(req).await.unwrap();

        let result = service.process_pending(10).await.unwrap();
        assert_eq!(result.failed, 1);
    }

    #[tokio::test]
    async fn test_retry_only_failed_notifications() {
        let repo = Arc::new(MockNotificationRepo::new());
        let service = NotificationService::new(repo);

        let notification = service.send(make_request()).await.unwrap();
        // Notification is Pending, not Failed — retry should fail
        let result = service.retry(notification.id).await;
        assert!(matches!(result, Err(NotifyError::ValidationError { .. })));
    }

    #[tokio::test]
    async fn test_get_statistics() {
        let repo = Arc::new(MockNotificationRepo::new());
        let service = NotificationService::new(repo);

        service.send(make_request()).await.unwrap();
        service.send(make_request()).await.unwrap();

        let stats = service.get_statistics().await.unwrap();
        assert_eq!(stats.total, 2);
        assert_eq!(stats.pending, 2);
    }
}
