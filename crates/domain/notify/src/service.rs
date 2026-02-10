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

use std::{collections::HashMap, sync::Arc};

use jiff::Timestamp;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::{
    error::NotifyError,
    repository::NotificationRepository,
    types::{
        Notification, NotificationChannel, NotificationFilter, NotificationStatistics,
        NotificationStatus, ProcessResult, SendNotificationRequest,
    },
};

/// Trait for notification channel backends (e.g. Telegram, Email).
#[async_trait::async_trait]
pub trait NotificationSender: Send + Sync {
    async fn send(&self, notification: &Notification) -> Result<(), NotifyError>;
}

pub struct NotificationService {
    repo:    Arc<dyn NotificationRepository>,
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

    pub async fn send(&self, req: SendNotificationRequest) -> Result<Notification, NotifyError> {
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
            id:             Uuid::new_v4(),
            channel:        req.channel,
            recipient:      req.recipient,
            subject:        req.subject,
            body:           req.body,
            status:         NotificationStatus::Pending,
            priority:       req.priority,
            retry_count:    0,
            max_retries:    3,
            error_message:  None,
            reference_type: req.reference_type,
            reference_id:   req.reference_id,
            metadata:       req.metadata,
            trace_id:       None,
            sent_at:        None,
            created_at:     Timestamp::now(),
        };

        let saved = self.repo.save(&notification).await?;
        info!(id = %saved.id, channel = ?saved.channel, "notification queued");
        Ok(saved)
    }

    pub async fn process_pending(&self, batch_size: i64) -> Result<ProcessResult, NotifyError> {
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
    use std::sync::Arc;

    use sqlx::postgres::PgPoolOptions;
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::postgres::Postgres;

    use super::*;
    use crate::pg_repository::PgNotificationRepository;

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
            channel:        NotificationChannel::Telegram,
            recipient:      "user123".to_string(),
            subject:        Some("Test".to_string()),
            body:           "Hello world".to_string(),
            priority:       crate::types::NotificationPriority::Normal,
            reference_type: None,
            reference_id:   None,
            metadata:       None,
        }
    }

    async fn setup_pool() -> (sqlx::PgPool, testcontainers::ContainerAsync<Postgres>) {
        let container = Postgres::default().start().await.unwrap();
        let host = container.get_host().await.unwrap();
        let port = container.get_host_port_ipv4(5432).await.unwrap();
        let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(&url)
            .await
            .unwrap();

        sqlx::migrate!("../../job-model/migrations")
            .run(&pool)
            .await
            .unwrap();

        (pool, container)
    }

    async fn make_service() -> (
        NotificationService,
        testcontainers::ContainerAsync<Postgres>,
    ) {
        let (pool, container) = setup_pool().await;
        let repo = Arc::new(PgNotificationRepository::new(pool));
        (NotificationService::new(repo), container)
    }

    #[tokio::test]
    async fn test_send_creates_pending_notification() {
        let (service, _container) = make_service().await;

        let result = service.send(make_request()).await.unwrap();
        assert_eq!(result.status, NotificationStatus::Pending);
        assert_eq!(result.channel, NotificationChannel::Telegram);
        assert_eq!(result.recipient, "user123");
    }

    #[tokio::test]
    async fn test_send_validates_empty_recipient() {
        let (service, _container) = make_service().await;

        let mut req = make_request();
        req.recipient = String::new();
        let result = service.send(req).await;
        assert!(matches!(result, Err(NotifyError::ValidationError { .. })));
    }

    #[tokio::test]
    async fn test_send_validates_empty_body() {
        let (service, _container) = make_service().await;

        let mut req = make_request();
        req.body = String::new();
        let result = service.send(req).await;
        assert!(matches!(result, Err(NotifyError::ValidationError { .. })));
    }

    #[tokio::test]
    async fn test_process_pending_with_successful_sender() {
        let (mut service, _container) = make_service().await;
        service.register_sender(
            NotificationChannel::Telegram,
            Arc::new(MockSender { should_fail: false }),
        );

        service.send(make_request()).await.unwrap();
        let result = service.process_pending(10).await.unwrap();
        assert_eq!(result.processed, 1);
        assert_eq!(result.succeeded, 1);
        assert_eq!(result.failed, 0);
    }

    #[tokio::test]
    async fn test_process_pending_with_failing_sender() {
        let (mut service, _container) = make_service().await;
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
        let (service, _container) = make_service().await;

        // Send via Email but no email sender registered
        let mut req = make_request();
        req.channel = NotificationChannel::Email;
        service.send(req).await.unwrap();

        let result = service.process_pending(10).await.unwrap();
        assert_eq!(result.failed, 1);
    }

    #[tokio::test]
    async fn test_retry_only_failed_notifications() {
        let (service, _container) = make_service().await;

        let notification = service.send(make_request()).await.unwrap();
        // Notification is Pending, not Failed — retry should fail
        let result = service.retry(notification.id).await;
        assert!(matches!(result, Err(NotifyError::ValidationError { .. })));
    }

    #[tokio::test]
    async fn test_get_statistics() {
        let (service, _container) = make_service().await;

        service.send(make_request()).await.unwrap();
        service.send(make_request()).await.unwrap();

        let stats = service.get_statistics().await.unwrap();
        assert_eq!(stats.total, 2);
        assert_eq!(stats.pending, 2);
    }
}
