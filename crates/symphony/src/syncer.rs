use std::collections::HashMap;

use tracing::{debug, info, warn};

use crate::client::{RalphClient, TaskRecord};
use crate::error::Result;
use crate::tracker::{IssueState, IssueTracker, TrackedIssue};

/// What action to take for a given issue ↔ task pair.
#[derive(Debug, PartialEq, Eq)]
enum SyncAction {
    /// Create a new ralph task for this issue.
    CreateTask,
    /// Issue is done in ralph — transition the issue tracker to terminal.
    CompleteIssue,
    /// Issue was closed externally — cancel the running ralph task.
    CancelTask,
    /// No action needed.
    None,
}

/// Report of what happened during a sync cycle.
#[derive(Debug, Default)]
pub struct SyncReport {
    pub created: Vec<String>,
    pub completed: Vec<String>,
    pub cancelled: Vec<String>,
    pub failed: Vec<String>,
    pub unchanged: usize,
}

/// Synchronizes issue tracker state with ralph task state.
pub struct IssueSyncer {
    client: RalphClient,
}

impl IssueSyncer {
    #[must_use]
    pub fn new(client: RalphClient) -> Self {
        Self { client }
    }

    /// Run a full sync cycle: compare issues against ralph tasks and take action.
    pub async fn sync(
        &self,
        tracker: &dyn IssueTracker,
        issues: &[TrackedIssue],
    ) -> Result<SyncReport> {
        // Fetch all non-archived tasks from ralph.
        let tasks = self.client.task_list(Option::None).await?;
        let task_map: HashMap<&str, &TaskRecord> =
            tasks.iter().map(|t| (t.id.as_str(), t)).collect();

        let mut report = SyncReport::default();

        for issue in issues {
            let task_status = task_map
                .get(issue.identifier.as_str())
                .map(|t| t.status.as_str());
            let action = determine_action(&issue.state, task_status);

            match action {
                SyncAction::CreateTask => {
                    let priority = issue.priority.min(5) as u8;
                    match self
                        .client
                        .task_create(&issue.identifier, &issue.title, priority, true)
                        .await
                    {
                        Ok(_) => {
                            info!(issue = %issue.identifier, "created ralph task");
                            report.created.push(issue.identifier.clone());
                        }
                        Err(e) => {
                            // CONFLICT means task already exists — treat as idempotent success.
                            if e.to_string().contains("CONFLICT") {
                                debug!(issue = %issue.identifier, "ralph task already exists");
                                report.unchanged += 1;
                            } else {
                                warn!(issue = %issue.identifier, error = %e, "failed to create ralph task");
                                report.failed.push(issue.identifier.clone());
                            }
                        }
                    }
                }
                SyncAction::CompleteIssue => {
                    match tracker.transition_issue(issue, "Done").await {
                        Ok(()) => {
                            info!(issue = %issue.identifier, "transitioned issue to Done");
                            report.completed.push(issue.identifier.clone());
                        }
                        Err(e) => {
                            warn!(issue = %issue.identifier, error = %e, "failed to transition issue");
                            report.failed.push(issue.identifier.clone());
                        }
                    }
                }
                SyncAction::CancelTask => {
                    match self.client.task_cancel(&issue.identifier).await {
                        Ok(_) => {
                            info!(issue = %issue.identifier, "cancelled ralph task");
                            report.cancelled.push(issue.identifier.clone());
                        }
                        Err(e) => {
                            warn!(issue = %issue.identifier, error = %e, "failed to cancel ralph task");
                            report.failed.push(issue.identifier.clone());
                        }
                    }
                }
                SyncAction::None => {
                    report.unchanged += 1;
                }
            }
        }

        Ok(report)
    }
}

/// Determine the sync action based on issue state and ralph task status.
fn determine_action(issue_state: &IssueState, task_status: Option<&str>) -> SyncAction {
    match (issue_state, task_status) {
        // New issue, no task yet → create.
        (IssueState::Active, None) => SyncAction::CreateTask,

        // Active issue, task is in progress → nothing to do.
        (IssueState::Active, Some("open" | "pending" | "running")) => SyncAction::None,

        // Active issue, task completed → close the issue.
        (IssueState::Active, Some("closed")) => SyncAction::CompleteIssue,

        // Active issue, task failed → log only, wait for human.
        (IssueState::Active, Some("failed")) => SyncAction::None,

        // Terminal issue, task still running → cancel it.
        (IssueState::Terminal, Some("open" | "pending" | "running")) => SyncAction::CancelTask,

        // Terminal issue, no task or terminal task → nothing.
        (IssueState::Terminal, _) => SyncAction::None,

        // Catch-all: unknown task status → nothing.
        (_, _) => SyncAction::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_action_new_issue_creates_task() {
        let action = determine_action(&IssueState::Active, None);
        assert!(matches!(action, SyncAction::CreateTask));
    }

    #[test]
    fn sync_action_active_issue_running_task_is_noop() {
        let action = determine_action(&IssueState::Active, Some("running"));
        assert!(matches!(action, SyncAction::None));
    }

    #[test]
    fn sync_action_active_issue_closed_task_completes_issue() {
        let action = determine_action(&IssueState::Active, Some("closed"));
        assert!(matches!(action, SyncAction::CompleteIssue));
    }

    #[test]
    fn sync_action_active_issue_failed_task_is_noop() {
        let action = determine_action(&IssueState::Active, Some("failed"));
        assert!(matches!(action, SyncAction::None));
    }

    #[test]
    fn sync_action_terminal_issue_running_task_cancels() {
        let action = determine_action(&IssueState::Terminal, Some("running"));
        assert!(matches!(action, SyncAction::CancelTask));
    }

    #[test]
    fn sync_action_terminal_issue_no_task_is_noop() {
        let action = determine_action(&IssueState::Terminal, None);
        assert!(matches!(action, SyncAction::None));
    }
}
