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

//! Integration tests for TaskReport types, SubscriptionRegistry, and
//! end-to-end publish → subscribe → tape delivery flow.

use rara_kernel::{
    identity::UserId,
    memory::{FileTapeStore, TapEntryKind, TapeService},
    notification::{NotifyAction, SubscriptionRegistry, TaskNotification, TaskReportRef},
    session::SessionKey,
    task_report::{TaskReport, TaskReportStatus},
};

/// Create a temp-file-backed SubscriptionRegistry for tests.
fn test_registry() -> (SubscriptionRegistry, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("subscriptions.json");
    (SubscriptionRegistry::load(path), tmp)
}

#[tokio::test]
async fn test_subscription_registry_tag_matching() {
    let (registry, _tmp) = test_registry();
    let session_a = SessionKey::new();
    let session_b = SessionKey::new();
    let user = UserId("alice".into());

    // Subscribe session_a to "pr_review".
    let sub_a = registry
        .subscribe(
            session_a,
            user.clone(),
            vec!["pr_review".into()],
            NotifyAction::ProactiveTurn,
        )
        .await;

    // Subscribe session_b to "repo:rararulab/rara".
    let _sub_b = registry
        .subscribe(
            session_b,
            user.clone(),
            vec!["repo:rararulab/rara".into()],
            NotifyAction::SilentAppend,
        )
        .await;

    // Match with tags ["pr_review", "repo:rararulab/rara"] — both match (same
    // owner).
    let matched = registry
        .match_tags(&["pr_review".into(), "repo:rararulab/rara".into()], &user)
        .await;
    assert_eq!(matched.len(), 2);

    // Match with only "pr_review" — only session_a matches.
    let matched = registry.match_tags(&["pr_review".into()], &user).await;
    assert_eq!(matched.len(), 1);
    assert_eq!(matched[0].subscriber, session_a);

    // Unsubscribe session_a.
    assert!(registry.unsubscribe(sub_a, &user).await);
    let matched = registry.match_tags(&["pr_review".into()], &user).await;
    assert!(matched.is_empty());

    // Remove session_b.
    registry.remove_session(&session_b).await;
    let matched = registry
        .match_tags(&["repo:rararulab/rara".into()], &user)
        .await;
    assert!(matched.is_empty());
}

#[tokio::test]
async fn test_subscription_no_match() {
    let (registry, _tmp) = test_registry();
    let session = SessionKey::new();
    let user = UserId("alice".into());

    registry
        .subscribe(
            session,
            user.clone(),
            vec!["deploy".into()],
            NotifyAction::SilentAppend,
        )
        .await;

    // No matching tags.
    let matched = registry.match_tags(&["pr_review".into()], &user).await;
    assert!(matched.is_empty());
}

#[tokio::test]
async fn test_unsubscribe_nonexistent() {
    let (registry, _tmp) = test_registry();
    assert!(
        !registry
            .unsubscribe(uuid::Uuid::new_v4(), &UserId("nobody".into()))
            .await
    );
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

// ===========================================================================
// End-to-end: publish TaskReport → subscription match → tape delivery
// ===========================================================================

/// Helper: create a TapeService backed by a temp directory.
async fn setup_tape() -> (TapeService, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    let store = FileTapeStore::new(tmp.path(), tmp.path()).await.unwrap();
    let tape = TapeService::new(store);
    (tape, tmp)
}

/// Simulate what `handle_publish_task_report` does: write report to source
/// tape, match subscriptions, deliver via SilentAppend to subscriber tapes.
///
/// This exercises the full data flow without needing a running kernel.
#[tokio::test]
async fn test_publish_report_silent_append_e2e() {
    let (tape, _tape_tmp) = setup_tape().await;
    let (registry, _reg_tmp) = test_registry();

    let source_session = SessionKey::new();
    let subscriber_a = SessionKey::new();
    let subscriber_b = SessionKey::new();
    let user = UserId("alice".into());

    // 1. Subscribe: session A watches "pr_review", session B watches "deploy".
    let _sub_a = registry
        .subscribe(
            subscriber_a,
            user.clone(),
            vec!["pr_review".into()],
            NotifyAction::SilentAppend,
        )
        .await;
    let _sub_b = registry
        .subscribe(
            subscriber_b,
            user.clone(),
            vec!["deploy".into()],
            NotifyAction::SilentAppend,
        )
        .await;

    // 2. Publish a TaskReport with tags ["pr_review", "repo:rararulab/rara"].
    let report = TaskReport {
        task_id: uuid::Uuid::new_v4(),
        task_type: "pr_review".into(),
        tags: vec!["pr_review".into(), "repo:rararulab/rara".into()],
        status: TaskReportStatus::Completed,
        summary: "PR #42 approved".into(),
        result: serde_json::json!({"verdict": "approved", "confidence": 9}),
        action_taken: Some("left approval comment".into()),
        source_session,
    };

    // 2. Build notification (kernel no longer writes to source tape; results go to
    //    per-job result store instead).
    let notification = TaskNotification {
        task_id:      report.task_id,
        task_type:    report.task_type.clone(),
        tags:         report.tags.clone(),
        status:       report.status,
        summary:      report.summary.clone(),
        result:       report.result.clone(),
        action_taken: report.action_taken.clone(),
        report_ref:   TaskReportRef {
            source_session,
            job_id: None,
        },
    };

    // 2c. Match subscriptions and deliver (scoped to same user).
    let matched = registry.match_tags(&report.tags, &user).await;
    assert_eq!(matched.len(), 1, "only subscriber_a should match pr_review");
    assert_eq!(matched[0].subscriber, subscriber_a);

    for sub in &matched {
        let notif_json = serde_json::to_value(&notification).unwrap();
        let sub_tape = sub.subscriber.to_string();
        tape.store()
            .append(&sub_tape, TapEntryKind::TaskReport, notif_json, None)
            .await
            .unwrap();
    }

    // 3. Verify: subscriber_a's tape has the notification entry.
    let sub_a_tape = subscriber_a.to_string();
    let sub_a_entries = tape.entries(&sub_a_tape).await.unwrap();
    assert_eq!(sub_a_entries.len(), 1);
    assert_eq!(sub_a_entries[0].kind, TapEntryKind::TaskReport);
    let delivered_notif: TaskNotification =
        serde_json::from_value(sub_a_entries[0].payload.clone()).unwrap();
    assert_eq!(delivered_notif.task_type, "pr_review");
    assert_eq!(delivered_notif.summary, "PR #42 approved");
    assert_eq!(
        delivered_notif.result,
        serde_json::json!({"verdict": "approved", "confidence": 9})
    );
    assert_eq!(
        delivered_notif.action_taken.as_deref(),
        Some("left approval comment")
    );
    assert_eq!(delivered_notif.report_ref.source_session, source_session);
    assert!(delivered_notif.report_ref.job_id.is_none());

    // 5. Verify: subscriber_b's tape is empty (tags didn't match).
    let sub_b_tape = subscriber_b.to_string();
    let sub_b_entries = tape.entries(&sub_b_tape).await.unwrap();
    assert!(
        sub_b_entries.is_empty(),
        "subscriber_b should not have received anything"
    );
}

/// Multiple subscribers match the same report — all get notified.
#[tokio::test]
async fn test_publish_report_multiple_subscribers() {
    let (tape, _tape_tmp) = setup_tape().await;
    let (registry, _reg_tmp) = test_registry();

    let source = SessionKey::new();
    let sub_1 = SessionKey::new();
    let sub_2 = SessionKey::new();
    let sub_3 = SessionKey::new();
    let user = UserId("alice".into());

    // All three subscribe to "critical".
    for sub in [sub_1, sub_2, sub_3] {
        registry
            .subscribe(
                sub,
                user.clone(),
                vec!["critical".into()],
                NotifyAction::SilentAppend,
            )
            .await;
    }

    // Publish with tag "critical".
    let report = TaskReport {
        task_id:        uuid::Uuid::new_v4(),
        task_type:      "deploy_check".into(),
        tags:           vec!["deploy_check".into(), "critical".into()],
        status:         TaskReportStatus::Failed,
        summary:        "deploy to prod failed".into(),
        result:         serde_json::json!({"error": "timeout"}),
        action_taken:   None,
        source_session: source,
    };

    let notification = TaskNotification {
        task_id:      report.task_id,
        task_type:    report.task_type.clone(),
        tags:         report.tags.clone(),
        status:       report.status,
        summary:      report.summary.clone(),
        result:       report.result.clone(),
        action_taken: report.action_taken.clone(),
        report_ref:   TaskReportRef {
            source_session: source,
            job_id:         None,
        },
    };

    let matched = registry.match_tags(&report.tags, &user).await;
    assert_eq!(
        matched.len(),
        3,
        "all three subscribers should match 'critical'"
    );

    for sub in &matched {
        let notif_json = serde_json::to_value(&notification).unwrap();
        tape.store()
            .append(
                &sub.subscriber.to_string(),
                TapEntryKind::TaskReport,
                notif_json,
                None,
            )
            .await
            .unwrap();
    }

    // All three should have exactly one entry.
    for sub in [sub_1, sub_2, sub_3] {
        let entries = tape.entries(&sub.to_string()).await.unwrap();
        assert_eq!(
            entries.len(),
            1,
            "each subscriber should have one notification"
        );
        let notif: TaskNotification = serde_json::from_value(entries[0].payload.clone()).unwrap();
        assert_eq!(notif.status, TaskReportStatus::Failed);
        assert_eq!(notif.summary, "deploy to prod failed");
    }
}

/// Unsubscribing before publish means no delivery.
#[tokio::test]
async fn test_unsubscribe_before_publish_no_delivery() {
    let (tape, _tape_tmp) = setup_tape().await;
    let (registry, _reg_tmp) = test_registry();

    let source = SessionKey::new();
    let subscriber = SessionKey::new();
    let user = UserId("alice".into());

    let sub_id = registry
        .subscribe(
            subscriber,
            user.clone(),
            vec!["pr_review".into()],
            NotifyAction::SilentAppend,
        )
        .await;

    // Unsubscribe before publish.
    assert!(registry.unsubscribe(sub_id, &user).await);

    // Publish — no subscribers should match.
    let matched = registry.match_tags(&["pr_review".into()], &user).await;
    assert!(matched.is_empty());

    // Subscriber tape should be empty.
    let entries = tape.entries(&subscriber.to_string()).await.unwrap();
    assert!(entries.is_empty());

    // Source tape should also be empty (we didn't write anything).
    let entries = tape.entries(&source.to_string()).await.unwrap();
    assert!(entries.is_empty());
}

/// Subscriptions are scoped by owner — a different user's publish does not
/// match, even if tags overlap.
#[tokio::test]
async fn test_cross_user_isolation() {
    let (registry, _tmp) = test_registry();
    let alice = UserId("alice".into());
    let bob = UserId("bob".into());

    let alice_session = SessionKey::new();

    // Alice subscribes to "pr_review".
    registry
        .subscribe(
            alice_session,
            alice.clone(),
            vec!["pr_review".into()],
            NotifyAction::SilentAppend,
        )
        .await;

    // Bob publishes with tag "pr_review" — Alice must NOT receive it.
    let matched = registry.match_tags(&["pr_review".into()], &bob).await;
    assert!(
        matched.is_empty(),
        "cross-user: Alice's subscription must not match Bob's publish"
    );

    // Alice publishes with the same tag — should match.
    let matched = registry.match_tags(&["pr_review".into()], &alice).await;
    assert_eq!(matched.len(), 1);
    assert_eq!(matched[0].subscriber, alice_session);
}

/// Another user cannot unsubscribe a subscription they don't own.
#[tokio::test]
async fn test_cross_user_unsubscribe_rejected() {
    let (registry, _tmp) = test_registry();
    let alice = UserId("alice".into());
    let bob = UserId("bob".into());

    let sub_id = registry
        .subscribe(
            SessionKey::new(),
            alice.clone(),
            vec!["pr_review".into()],
            NotifyAction::SilentAppend,
        )
        .await;

    // Bob tries to unsubscribe Alice's subscription — must fail.
    assert!(
        !registry.unsubscribe(sub_id, &bob).await,
        "bob must not be able to cancel alice's subscription"
    );

    // Alice's subscription should still be active.
    let matched = registry.match_tags(&["pr_review".into()], &alice).await;
    assert_eq!(matched.len(), 1);

    // Alice can unsubscribe her own.
    assert!(registry.unsubscribe(sub_id, &alice).await);
    let matched = registry.match_tags(&["pr_review".into()], &alice).await;
    assert!(matched.is_empty());
}

/// In-flight jobs survive a simulated kernel crash and are re-fired on
/// startup.
#[test]
fn test_in_flight_recovery() {
    use rara_kernel::schedule::{JobEntry, JobId, JobWheel, Trigger};

    let tmp = tempfile::tempdir().unwrap();
    let jobs_path = tmp.path().join("jobs.json");
    let in_flight_path = tmp.path().join("in_flight.json");

    let test_user = rara_kernel::identity::KernelUser {
        name:        "test-user".into(),
        role:        rara_kernel::identity::Role::User,
        permissions: vec![rara_kernel::identity::Permission::Spawn],
        enabled:     true,
    };
    let principal = rara_kernel::identity::Principal::from_user(&test_user);
    let session = SessionKey::new();
    let now = jiff::Timestamp::now();
    let past = now
        .checked_sub(jiff::SignedDuration::from_secs(10))
        .unwrap();

    // 1. Create a wheel with one Once job and one Interval job, both expired.
    let mut wheel = JobWheel::load(jobs_path.clone());
    let once_job = JobEntry {
        id:          JobId::new(),
        trigger:     Trigger::Once { run_at: past },
        message:     "once task".into(),
        session_key: session,
        principal:   principal.clone(),
        created_at:  past,
        tags:        vec!["test".into()],
    };
    let interval_job = JobEntry {
        id:          JobId::new(),
        trigger:     Trigger::Interval {
            anchor_at:  Some(past),
            every_secs: 60,
            next_at:    past,
        },
        message:     "interval task".into(),
        session_key: session,
        principal:   principal.clone(),
        created_at:  past,
        tags:        vec![],
    };
    let once_id = once_job.id;
    let interval_id = interval_job.id;
    wheel.add(once_job);
    wheel.add(interval_job);

    // 2. Drain expired — both should be returned and tracked in-flight.
    let expired = wheel.drain_expired(now);
    assert_eq!(expired.len(), 2);
    wheel.persist();

    // in_flight.json should exist and contain 2 entries.
    assert!(in_flight_path.exists(), "in_flight.json should be written");
    let ifl_content = std::fs::read_to_string(&in_flight_path).unwrap();
    let ifl_entries: Vec<serde_json::Value> = serde_json::from_str(&ifl_content).unwrap();
    assert_eq!(ifl_entries.len(), 2);

    // The interval job should also be rescheduled in the wheel.
    let listed = wheel.list(None);
    assert_eq!(listed.len(), 1, "interval job should be rescheduled");
    assert_eq!(listed[0].id, interval_id);

    // 3. Simulate kernel crash — drop the wheel, load fresh.
    drop(wheel);
    let mut wheel2 = JobWheel::load(jobs_path.clone());

    // 4. take_in_flight should return the 2 jobs from the previous run.
    let recovered = wheel2.take_in_flight();
    assert_eq!(recovered.len(), 2);
    let recovered_ids: std::collections::HashSet<_> = recovered.iter().map(|j| j.id).collect();
    assert!(recovered_ids.contains(&once_id));
    assert!(recovered_ids.contains(&interval_id));

    // The ledger should still contain the entries on disk (crash-safe: they
    // are only removed individually by complete_in_flight after the agent
    // session ends, so a second crash won't lose them).
    let ifl_content_before_complete = std::fs::read_to_string(&in_flight_path).unwrap();
    let ifl_entries: Vec<serde_json::Value> =
        serde_json::from_str(&ifl_content_before_complete).unwrap();
    assert_eq!(
        ifl_entries.len(),
        2,
        "in_flight ledger should be preserved until complete_in_flight"
    );

    // 5. A second take_in_flight returns nothing (flag prevents re-fire).
    let recovered2 = wheel2.take_in_flight();
    assert!(recovered2.is_empty());

    // 6. complete_in_flight removes entries individually and persists.
    assert!(wheel2.complete_in_flight(&once_id));
    assert!(wheel2.complete_in_flight(&interval_id));
    let ifl_content = std::fs::read_to_string(&in_flight_path).unwrap();
    let ifl_entries: Vec<serde_json::Value> = serde_json::from_str(&ifl_content).unwrap();
    assert!(
        ifl_entries.is_empty(),
        "in_flight should be empty after all jobs completed"
    );

    // 7. Simulate another crash+restart after recovery — ledger should still
    //    contain entries if complete_in_flight was never called.
    drop(wheel2);
    // Restore the 2-entry ledger to simulate crash before completion.
    std::fs::write(&in_flight_path, &ifl_content_before_complete).unwrap();
    let mut wheel3 = JobWheel::load(jobs_path);
    let recovered3 = wheel3.take_in_flight();
    assert_eq!(
        recovered3.len(),
        2,
        "crash before complete_in_flight should re-recover jobs"
    );
}

/// complete_in_flight removes a job from the ledger.
#[test]
fn test_complete_in_flight() {
    use rara_kernel::schedule::{JobEntry, JobId, JobWheel, Trigger};

    let tmp = tempfile::tempdir().unwrap();
    let jobs_path = tmp.path().join("jobs.json");

    let test_user = rara_kernel::identity::KernelUser {
        name:        "test-user".into(),
        role:        rara_kernel::identity::Role::User,
        permissions: vec![rara_kernel::identity::Permission::Spawn],
        enabled:     true,
    };
    let principal = rara_kernel::identity::Principal::from_user(&test_user);
    let session = SessionKey::new();
    let past = jiff::Timestamp::now()
        .checked_sub(jiff::SignedDuration::from_secs(10))
        .unwrap();

    let mut wheel = JobWheel::load(jobs_path);
    let job = JobEntry {
        id: JobId::new(),
        trigger: Trigger::Once { run_at: past },
        message: "task".into(),
        session_key: session,
        principal,
        created_at: past,
        tags: vec![],
    };
    let job_id = job.id;
    wheel.add(job);
    wheel.drain_expired(jiff::Timestamp::now());
    wheel.persist();

    // Complete the job — should remove from in-flight.
    assert!(wheel.complete_in_flight(&job_id));
    assert!(
        !wheel.complete_in_flight(&job_id),
        "second call should return false"
    );

    // Simulate restart — no in-flight jobs should remain.
    drop(wheel);
    let mut wheel2 = JobWheel::load(tmp.path().join("jobs.json"));
    let recovered = wheel2.take_in_flight();
    assert!(recovered.is_empty());
}

/// JobResultStore writes per-job result files and reads them back.
#[tokio::test]
async fn test_job_result_store_roundtrip() {
    use rara_kernel::{
        schedule::{JobId, JobResult, JobResultStore},
        task_report::TaskReportStatus,
    };

    let tmp = tempfile::tempdir().unwrap();
    let store = JobResultStore::new(tmp.path().join("results"));

    let job_id = JobId::new();

    // Append two results for the same job (simulates recurring execution).
    let r1 = JobResult {
        job_id,
        task_id: uuid::Uuid::new_v4(),
        task_type: "deploy_check".into(),
        tags: vec!["deploy".into()],
        status: TaskReportStatus::Completed,
        summary: "deploy ok".into(),
        result: serde_json::json!({"version": "1.0"}),
        action_taken: None,
        completed_at: jiff::Timestamp::from_second(1000).unwrap(),
    };
    let r2 = JobResult {
        job_id,
        task_id: uuid::Uuid::new_v4(),
        task_type: "deploy_check".into(),
        tags: vec!["deploy".into()],
        status: TaskReportStatus::Failed,
        summary: "deploy failed".into(),
        result: serde_json::json!({"error": "timeout"}),
        action_taken: None,
        completed_at: jiff::Timestamp::from_second(2000).unwrap(),
    };

    store.append(&r1).await.unwrap();
    store.append(&r2).await.unwrap();

    // Read back — should return both, ordered by completion time.
    let results = store.read(&job_id).await;
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].summary, "deploy ok");
    assert_eq!(results[1].summary, "deploy failed");
    assert_eq!(results[1].status, TaskReportStatus::Failed);

    // A different job_id returns empty.
    let other_id = JobId::new();
    let results = store.read(&other_id).await;
    assert!(results.is_empty());
}

/// Subscriptions survive a simulated restart via file persistence.
#[tokio::test]
async fn test_subscription_persistence_roundtrip() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("subscriptions.json");
    let user = UserId("alice".into());
    let session = SessionKey::new();

    // 1. Create registry, subscribe, let it persist.
    let sub_id = {
        let registry = SubscriptionRegistry::load(path.clone());
        let id = registry
            .subscribe(
                session,
                user.clone(),
                vec!["pr_review".into(), "critical".into()],
                NotifyAction::ProactiveTurn,
            )
            .await;
        // File should exist now.
        assert!(path.exists(), "subscriptions.json should be written");
        id
    };
    // Registry dropped here — simulates kernel shutdown.

    // 2. Load a fresh registry from the same file.
    let registry2 = SubscriptionRegistry::load(path.clone());

    // Subscription should be restored.
    let matched = registry2.match_tags(&["pr_review".into()], &user).await;
    assert_eq!(matched.len(), 1);
    assert_eq!(matched[0].id, sub_id);
    assert_eq!(matched[0].subscriber, session);
    assert_eq!(matched[0].on_receive, NotifyAction::ProactiveTurn);

    // Also matches on the other tag.
    let matched = registry2.match_tags(&["critical".into()], &user).await;
    assert_eq!(matched.len(), 1);

    // 3. Unsubscribe and verify file is updated.
    assert!(registry2.unsubscribe(sub_id, &user).await);
    let matched = registry2.match_tags(&["pr_review".into()], &user).await;
    assert!(matched.is_empty());

    // 4. Load again — should be empty.
    let registry3 = SubscriptionRegistry::load(path);
    let matched = registry3.match_tags(&["pr_review".into()], &user).await;
    assert!(matched.is_empty());
}
