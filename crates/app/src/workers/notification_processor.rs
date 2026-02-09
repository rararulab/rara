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

//! Background worker that processes pending notifications in batches.

use std::sync::Arc;

use async_trait::async_trait;
use job_common_worker::{FallibleWorker, WorkError, WorkResult, WorkerContext};
use job_domain_notify::service::NotificationService;
use tracing::{error, info};

/// Background worker that periodically processes pending notifications.
pub struct NotificationProcessorWorker {
    batch_size: i64,
}

impl NotificationProcessorWorker {
    /// Create a new notification processor with the given batch size.
    pub fn new(batch_size: i64) -> Self {
        Self { batch_size }
    }
}

/// Shared state for the notification processor worker.
#[derive(Clone)]
pub struct WorkerState {
    /// The notification service used to process pending notifications.
    pub notification_service: Arc<NotificationService>,
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
