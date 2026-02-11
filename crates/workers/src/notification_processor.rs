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

//! Background worker for processing pending notifications.
//!
//! Notification lifecycle:
//!
//! 1. **Enqueue** — Anywhere in the system calls
//!    `NotificationService::send(req)`, which creates a `status = Pending`
//!    record in the `notification_logs` table. No actual delivery happens at
//!    this point.
//!
//! 2. **Process** — This worker is woken every 30 seconds by
//!    `job-common-worker`, calling
//!    `NotificationService::process_pending(batch_size)` to pull pending
//!    notifications and deliver them via registered `NotificationSender`
//!    backends (Telegram / Email / Webhook). Successful sends are marked
//!    `Sent`; failures increment `retry_count`, and exceeding `max_retries`
//!    marks them `Failed`.
//!
//! 3. **Retry** — Failed notifications can be manually reset to `Retrying` via
//!    `POST /api/notifications/:id/retry`, and the next worker cycle picks them
//!    up.
//!
//! ```text
//! ┌──────────┐  send()  ┌─────────┐  worker  ┌──────┐
//! │ App code │ ───────→ │ Pending │ ───────→ │ Sent │
//! └──────────┘          └─────────┘          └──────┘
//!                            │ send failed         ↑
//!                            ▼                     │ retry
//!                       ┌─────────┐  retry ok  ┌──────────┐
//!                       │ Failed  │ ←──────── │ Retrying │
//!                       └─────────┘           └──────────┘
//! ```

use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use job_common_worker::{FallibleWorker, NotifyHandle, WorkError, WorkResult, WorkerContext};
use job_domain_notify::service::NotificationService;
use tracing::{error, info};

/// Background worker that periodically processes pending notifications in
/// batch.
///
/// Scheduled by `job-common-worker::Manager` at a fixed interval (default 30s),
/// pulling up to `batch_size` pending notifications per cycle.
pub struct NotificationProcessorWorker {
    batch_size: i64,
}

impl NotificationProcessorWorker {
    pub fn new(batch_size: i64) -> Self { Self { batch_size } }
}

/// Shared state for workers, injected by `job-app` at startup.
#[derive(Clone)]
pub struct WorkerState {
    pub notification_service: Arc<NotificationService>,
    pub ai_service:           Arc<job_ai::service::AiService>,
    pub job_repo:             Arc<dyn job_domain_job_source::repository::JobRepository>,
    pub saved_job_service: Arc<
        job_domain_saved_job::service::SavedJobService<
            job_domain_saved_job::pg_repository::PgSavedJobRepository,
        >,
    >,
    pub object_store:         Arc<job_object_store::ObjectStore>,
    pub crawl_client:         job_domain_saved_job::crawl4ai::Crawl4AiClient,
    pub analyze_notify:       Arc<RwLock<Option<NotifyHandle>>>,
}

#[async_trait]
impl FallibleWorker<WorkerState> for NotificationProcessorWorker {
    async fn work(&mut self, ctx: WorkerContext<WorkerState>) -> WorkResult {
        let service = &ctx.state().notification_service;

        match service.process_pending(self.batch_size).await {
            Ok(result) => {
                if result.processed > 0 {
                    info!(
                        processed = result.processed,
                        succeeded = result.succeeded,
                        failed = result.failed,
                        "notification batch processed"
                    );
                }
                Ok(())
            }
            Err(e) => {
                error!(error = %e, "notification processing failed");
                Err(WorkError::transient(format!(
                    "notification processing failed: {e}"
                )))
            }
        }
    }
}
