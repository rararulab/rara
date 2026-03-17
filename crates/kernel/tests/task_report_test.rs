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

//! Integration tests for TaskReport types and SubscriptionRegistry.

use rara_kernel::{
    notification::{NotifyAction, SubscriptionRegistry},
    session::SessionKey,
    task_report::{TaskReport, TaskReportStatus},
};

#[tokio::test]
async fn test_subscription_registry_tag_matching() {
    let registry = SubscriptionRegistry::new();
    let session_a = SessionKey::new();
    let session_b = SessionKey::new();

    // Subscribe session_a to "pr_review".
    let sub_a = registry
        .subscribe(
            session_a,
            vec!["pr_review".into()],
            NotifyAction::ProactiveTurn,
        )
        .await;

    // Subscribe session_b to "repo:rararulab/rara".
    let _sub_b = registry
        .subscribe(
            session_b,
            vec!["repo:rararulab/rara".into()],
            NotifyAction::SilentAppend,
        )
        .await;

    // Match with tags ["pr_review", "repo:rararulab/rara"] — both match.
    let matched = registry
        .match_tags(&["pr_review".into(), "repo:rararulab/rara".into()])
        .await;
    assert_eq!(matched.len(), 2);

    // Match with only "pr_review" — only session_a matches.
    let matched = registry.match_tags(&["pr_review".into()]).await;
    assert_eq!(matched.len(), 1);
    assert_eq!(matched[0].subscriber, session_a);

    // Unsubscribe session_a.
    assert!(registry.unsubscribe(sub_a).await);
    let matched = registry.match_tags(&["pr_review".into()]).await;
    assert!(matched.is_empty());

    // Remove session_b.
    registry.remove_session(&session_b).await;
    let matched = registry.match_tags(&["repo:rararulab/rara".into()]).await;
    assert!(matched.is_empty());
}

#[tokio::test]
async fn test_subscription_no_match() {
    let registry = SubscriptionRegistry::new();
    let session = SessionKey::new();

    registry
        .subscribe(session, vec!["deploy".into()], NotifyAction::SilentAppend)
        .await;

    // No matching tags.
    let matched = registry.match_tags(&["pr_review".into()]).await;
    assert!(matched.is_empty());
}

#[tokio::test]
async fn test_unsubscribe_nonexistent() {
    let registry = SubscriptionRegistry::new();
    assert!(!registry.unsubscribe(uuid::Uuid::new_v4()).await);
}

#[test]
fn test_task_report_roundtrip() {
    let report = TaskReport {
        task_id:        uuid::Uuid::new_v4(),
        task_type:      "pr_review".into(),
        tags:           vec!["pr_review".into(), "repo:rararulab/rara".into()],
        status:         TaskReportStatus::Completed,
        summary:        "PR #42 review done".into(),
        result:         serde_json::json!({"verdict": "approved"}),
        action_taken:   Some("approved".into()),
        source_session: SessionKey::new(),
    };

    let json = serde_json::to_string(&report).unwrap();
    let deserialized: TaskReport = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.task_type, "pr_review");
    assert_eq!(deserialized.status, TaskReportStatus::Completed);
}

#[test]
fn test_task_report_without_action_taken() {
    let report = TaskReport {
        task_id:        uuid::Uuid::new_v4(),
        task_type:      "deploy_check".into(),
        tags:           vec!["deploy_check".into()],
        status:         TaskReportStatus::Failed,
        summary:        "deploy failed".into(),
        result:         serde_json::json!({"error": "timeout"}),
        action_taken:   None,
        source_session: SessionKey::new(),
    };

    let json = serde_json::to_string(&report).unwrap();
    // action_taken should be absent from JSON when None.
    assert!(!json.contains("action_taken"));

    let deserialized: TaskReport = serde_json::from_str(&json).unwrap();
    assert!(deserialized.action_taken.is_none());
    assert_eq!(deserialized.status, TaskReportStatus::Failed);
}

#[test]
fn test_tap_entry_kind_task_report() {
    use rara_kernel::memory::TapEntryKind;

    let kind = TapEntryKind::TaskReport;
    let serialized = serde_json::to_string(&kind).unwrap();
    assert_eq!(serialized, "\"task_report\"");

    let deserialized: TapEntryKind = serde_json::from_str("\"task_report\"").unwrap();
    assert_eq!(deserialized, TapEntryKind::TaskReport);
}

#[test]
fn test_notify_action_serde() {
    let proactive = serde_json::to_string(&NotifyAction::ProactiveTurn).unwrap();
    assert_eq!(proactive, "\"proactive_turn\"");

    let silent = serde_json::to_string(&NotifyAction::SilentAppend).unwrap();
    assert_eq!(silent, "\"silent_append\"");

    let deserialized: NotifyAction = serde_json::from_str("\"proactive_turn\"").unwrap();
    assert_eq!(deserialized, NotifyAction::ProactiveTurn);
}
