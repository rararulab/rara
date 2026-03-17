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

//! Structured task report types for background/scheduled task results.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::session::SessionKey;

/// Structured result from a background or scheduled task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskReport {
    /// Unique task identifier.
    pub task_id:        Uuid,
    /// Fixed category, e.g. "pr_review", "deploy_check".
    pub task_type:      String,
    /// Routing labels. Automatically includes task_type.
    /// Additional dimensions like "repo:rararulab/rara", "critical".
    pub tags:           Vec<String>,
    /// Completion status.
    pub status:         TaskReportStatus,
    /// Human-readable one-line summary.
    pub summary:        String,
    /// Task-type-specific structured result.
    pub result:         serde_json::Value,
    /// Action already taken by the task agent, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_taken:   Option<String>,
    /// Session that produced this report (set automatically by the kernel).
    #[serde(default)]
    pub source_session: SessionKey,
}

/// Status of a completed task report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskReportStatus {
    /// Task completed successfully.
    Completed,
    /// Task failed.
    Failed,
    /// Requires user decision before proceeding.
    NeedsApproval,
}

/// PR review result (task_type = "pr_review").
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrReviewResult {
    /// Pull request number.
    pub pr_number:        u64,
    /// Repository full name (e.g. "rararulab/rara").
    pub repo:             String,
    /// Review verdict.
    pub verdict:          ReviewVerdict,
    /// Confidence score 1-10 from codex review.
    pub confidence_score: u8,
    /// Risk level derived from diff size, file types, critical paths.
    pub risk_level:       RiskLevel,
    /// Inline review comments.
    pub comments:         Vec<ReviewComment>,
}

/// Review verdict for a PR review.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewVerdict {
    /// PR is approved.
    Approved,
    /// Changes requested.
    ChangesRequested,
    /// Needs human discussion.
    NeedsDiscussion,
}

/// Risk level for a PR change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    /// diff < 50 lines, no migration/config.
    Low,
    /// diff 50-300 lines, or touches config.
    Medium,
    /// diff > 300 lines, or touches migration/security/CI.
    High,
}

/// A single review comment on a file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewComment {
    /// File path relative to repo root.
    pub file:     String,
    /// Line number (if applicable).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line:     Option<u64>,
    /// Severity of the comment.
    pub severity: CommentSeverity,
    /// Comment body text.
    pub body:     String,
}

/// Severity of a review comment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommentSeverity {
    /// Must fix before merge.
    Critical,
    /// Should fix, potential issue.
    Warning,
    /// Improvement suggestion.
    Suggestion,
    /// Style or minor preference.
    Nitpick,
}
