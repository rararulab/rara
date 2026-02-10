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
//!
//! [`NotificationService`] is the core orchestrator for the notification
//! domain:
//!
//! - **Queuing**: [`send()`](NotificationService::send) validates the request
//!   and persists a `Pending` notification to the database — no actual delivery
//!   happens.
//! - **Batch processing**:
//!   [`process_pending()`](NotificationService::process_pending) pulls pending
//!   notifications from the database and dispatches them through registered
//!   [`NotificationSender`] backends.
//! - **Retry**: [`retry()`](NotificationService::retry) resets a `Failed`
//!   notification to `Retrying` so the next worker cycle picks it up again.
//!
//! Channel backends (Telegram / Email / Webhook) are registered at startup via
//! [`register_sender()`](NotificationService::register_sender); the actual
//! delivery logic lives in each [`NotificationSender`] implementation.

use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

use jiff::Timestamp;
use job_common_worker::{Notifiable, NotifyHandle};
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
///
/// Each channel must implement this trait.
/// [`NotificationService::process_pending()`] dispatches to the appropriate
/// sender based on `notification.channel`.
///
/// Built-in implementations:
/// - [`NoopSender`](crate::sender::NoopSender) — no-op, used for unconfigured
///   channels
/// - [`TelegramService`](job_domain_shared::telegram_service::TelegramService)
///   — delivers via teloxide
#[async_trait::async_trait]
pub trait NotificationSender: Send + Sync {
    async fn send(&self, notification: &Notification) -> Result<(), NotifyError>;
}

/// Core notification domain service.
///
/// Holds two dependencies:
/// - `repo` — persistence layer (PostgreSQL impl:
///   [`PgNotificationRepository`](crate::pg_repository::PgNotificationRepository))
/// - `senders` — channel-indexed map of delivery backends, injected by
///   `job-app` at startup
pub struct NotificationService {
    repo:           Arc<dyn NotificationRepository>,
    senders:        HashMap<NotificationChannel, Arc<dyn NotificationSender>>,
    notify_trigger: RwLock<Option<NotifyHandle>>,
}

impl NotificationService {
    pub fn new(repo: Arc<dyn NotificationRepository>) -> Self {
        Self {
            repo,
            senders: HashMap::new(),
            notify_trigger: RwLock::new(None),
        }
    }

    /// Registers the runtime notify handle used to trigger immediate
    /// notification processing when new items are queued.
    pub fn set_notify_trigger(&self, handle: NotifyHandle) {
        if let Ok(mut guard) = self.notify_trigger.write() {
            *guard = Some(handle);
        } else {
            warn!("failed to acquire notification trigger write lock");
        }
    }

    /// Register a delivery backend for a channel.
    ///
    /// Registering the same channel twice overwrites the previous sender.
    /// Called once per channel by `job-app` at startup.
    pub fn register_sender(
        &mut self,
        channel: NotificationChannel,
        sender: Arc<dyn NotificationSender>,
    ) {
        self.senders.insert(channel, sender);
    }

    /// Queue a notification (persists to database with `status = Pending`).
    ///
    /// Does **not** send immediately — actual delivery is handled by the
    /// background worker calling [`process_pending()`](Self::process_pending).
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
        self.trigger_processing_now();
        Ok(saved)
    }

    /// Process pending notifications in batch.
    ///
    /// 1. Pull up to `batch_size` notifications with `Pending`/`Retrying`
    ///    status
    /// 2. Look up the [`NotificationSender`] for each notification's channel
    /// 3. Attempt delivery for each notification:
    ///    - Success → mark as `Sent`
    ///    - Failure with `retry_count < max_retries` → increment `retry_count`
    ///    - Failure at max retries → mark as `Failed` with error message
    /// 4. Return a [`ProcessResult`] summarizing the batch outcome
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

    /// Retry a failed notification.
    ///
    /// Only allowed when the notification's current status is `Failed`.
    /// Resets status to `Retrying` and clears the error message so the
    /// next worker cycle picks it up for re-delivery.
    ///
    /// Exposed via REST API: `POST /api/notifications/:id/retry`
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

        let saved = self.repo.update(&updated).await?;
        self.trigger_processing_now();
        Ok(saved)
    }

    /// List notifications matching the given filter.
    pub async fn list(
        &self,
        filter: &NotificationFilter,
    ) -> Result<Vec<Notification>, NotifyError> {
        self.repo.find_all(filter).await
    }

    /// Find a notification by ID.
    pub async fn get_by_id(&self, id: Uuid) -> Result<Option<Notification>, NotifyError> {
        self.repo.find_by_id(id).await
    }

    /// Get notification statistics (counts by status).
    pub async fn get_statistics(&self) -> Result<NotificationStatistics, NotifyError> {
        self.repo.get_statistics().await
    }

    fn trigger_processing_now(&self) {
        if let Ok(guard) = self.notify_trigger.read() {
            if let Some(handle) = guard.as_ref() {
                handle.notify();
            }
        } else {
            warn!("failed to acquire notification trigger read lock");
        }
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
