// Copyright 2025 Rararulab
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

//! Typed proactive signals emitted by the kernel for Mita orchestration.

use std::time::Duration;

use serde::Serialize;

use crate::session::SessionKey;

/// Proactive signal for Mita orchestration.
///
/// Each variant represents a distinct event that may warrant proactive
/// action from the Mita background agent. Signals flow through the
/// [`super::ProactiveFilter`] before reaching Mita.
#[derive(Debug, Clone, Serialize)]
pub enum ProactiveSignal {
    /// Session has been idle beyond threshold.
    SessionIdle {
        /// The session that has been idle.
        session_key:   SessionKey,
        /// How long the session has been idle.
        idle_duration: Duration,
    },
    /// Scheduled task agent failed to spawn.
    TaskFailed {
        /// Error description from the spawn failure.
        error: String,
    },
    /// Conversation naturally completed (turn ended without pending work).
    SessionCompleted {
        /// The session that completed.
        session_key: SessionKey,
        /// Brief summary of what the session accomplished.
        summary:     String,
    },
    /// Daily morning greeting trigger (fires at work hours start).
    MorningGreeting,
    /// End-of-day summary trigger (fires at work hours end).
    DailySummary,
}

impl ProactiveSignal {
    /// Returns a stable string key for this signal kind.
    ///
    /// Used as the cooldown map key in [`super::ProactiveFilter`] so that
    /// rate limiting is per-kind, not per-instance.
    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::SessionIdle { .. } => "session_idle",
            Self::TaskFailed { .. } => "task_failed",
            Self::SessionCompleted { .. } => "session_completed",
            Self::MorningGreeting => "morning_greeting",
            Self::DailySummary => "daily_summary",
        }
    }

    /// Returns a cooldown key that distinguishes per-session signals.
    ///
    /// For session-scoped signals like `SessionIdle`, the key includes
    /// the embedded session identifier so that cooldowns apply per-session
    /// rather than globally blocking all sessions of the same kind.
    pub fn cooldown_key(&self) -> String {
        match self {
            Self::SessionIdle { session_key, .. } => format!("session_idle:{session_key}"),
            Self::SessionCompleted { session_key, .. } => {
                format!("session_completed:{session_key}")
            }
            _ => self.kind_name().to_string(),
        }
    }
}
