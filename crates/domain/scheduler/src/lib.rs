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

//! # job-domain-scheduler
//!
//! Cron and task scheduling orchestration for the Job Automation platform.
//!
//! This crate provides the scheduling layer that drives periodic and
//! one-shot tasks across all domains.  Responsibilities include:
//!
//! - Defining schedulable tasks via the [`SchedulableTask`] trait.
//! - Cron expression parsing and next-run calculation.
//! - Task lifecycle tracking (pending -> running -> completed/failed).
//!
//! The existing `job-common-worker` crate provides low-level worker
//! primitives; this crate adds the domain-aware orchestration layer on top.

use job_domain_core::id::SchedulerTaskId;
use serde::{Deserialize, Serialize};

/// Metadata for a scheduled task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledTask {
    /// Unique identifier.
    pub id:        SchedulerTaskId,
    /// Human-readable name.
    pub name:      String,
    /// Cron expression (e.g. "0 */5 * * * *").
    pub cron_expr: String,
    /// Whether the task is currently enabled.
    pub enabled:   bool,
}

/// Trait for tasks that can be executed by the scheduler.
#[async_trait::async_trait]
pub trait SchedulableTask: Send + Sync {
    /// Human-readable name for logging.
    fn name(&self) -> &str;

    /// Execute the task. Returns `Ok(())` on success.
    async fn run(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
}
