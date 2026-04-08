//! End-to-end tests for anchor tree checkout flow.
//!
//! These tests exercise the full TapeService + SessionIndex integration
//! to verify that checkout, fork, and tree operations are correct and
//! don't corrupt tape data.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use dashmap::DashMap;
use rara_kernel::{
    memory::{
        AnchorTree, FileTapeStore, HandoffState, TapEntryKind, TapeService, get_fork_metadata,
        set_fork_metadata,
    },
    session::{ChannelBinding, SessionEntry, SessionError, SessionIndex, SessionKey},
};
use serde_json::json;

// ---------------------------------------------------------------------------
// Test SessionIndex (in-memory, for integration tests)
// ---------------------------------------------------------------------------

#[derive(Default)]
struct TestSessionIndex {
    sessions: DashMap<String, SessionEntry>,
}

#[async_trait]
impl SessionIndex for TestSessionIndex {
    async fn create_session(&self, entry: &SessionEntry) -> Result<SessionEntry, SessionError> {
        let key = entry.key.to_string();
        if self.sessions.contains_key(&key) {
            return Err(SessionError::AlreadyExists { key });
        }
        self.sessions.insert(key, entry.clone());
        Ok(entry.clone())
    }

    async fn get_session(&self, key: &SessionKey) -> Result<Option<SessionEntry>, SessionError> {
        Ok(self.sessions.get(&key.to_string()).map(|r| r.clone()))
    }

    async fn list_sessions(
        &self,
        limit: i64,
        _offset: i64,
    ) -> Result<Vec<SessionEntry>, SessionError> {
        let mut entries: Vec<SessionEntry> =
            self.sessions.iter().map(|r| r.value().clone()).collect();
        entries.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        entries.truncate(limit as usize);
        Ok(entries)
    }

    async fn update_session(&self, entry: &SessionEntry) -> Result<SessionEntry, SessionError> {
        let key = entry.key.to_string();
        if !self.sessions.contains_key(&key) {
            return Err(SessionError::NotFound { key });
        }
        self.sessions.insert(key, entry.clone());
        Ok(entry.clone())
    }

    async fn delete_session(&self, key: &SessionKey) -> Result<(), SessionError> {
        let raw = key.to_string();
        self.sessions
            .remove(&raw)
            .ok_or(SessionError::NotFound { key: raw })?;
        Ok(())
    }

    async fn bind_channel(
        &self,
        _binding: &ChannelBinding,
    ) -> Result<ChannelBinding, SessionError> {
        unimplemented!("not needed for anchor tree tests")
    }

    async fn get_channel_binding(
        &self,
        _channel_type: rara_kernel::channel::types::ChannelType,
        _chat_id: &str,
    ) -> Result<Option<ChannelBinding>, SessionError> {
        Ok(None)
    }

    async fn unbind_session(&self, _key: &SessionKey) -> Result<(), SessionError> { Ok(()) }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

async fn setup() -> (TapeService, Arc<TestSessionIndex>, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    let store = FileTapeStore::new(tmp.path(), tmp.path()).await.unwrap();
    let tape = TapeService::new(store);
    let sessions = Arc::new(TestSessionIndex::default());
    (tape, sessions, tmp)
}

async fn create_session(
    sessions: &TestSessionIndex,
    key: &SessionKey,
    metadata: Option<serde_json::Value>,
) {
    let now = Utc::now();
    sessions
        .create_session(&SessionEntry {
            key: key.clone(),
            title: None,
            model: None,
            system_prompt: None,
            message_count: 0,
            preview: None,
            metadata,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
}

// ---------------------------------------------------------------------------
// Test 1: Basic checkout preserves entries up to anchor, excludes post-anchor
// ---------------------------------------------------------------------------

#[tokio::test]
async fn checkout_preserves_pre_anchor_excludes_post_anchor() {
    let (tape, _sessions, _tmp) = setup().await;
    let source = "source-tape";

    // Build tape: bootstrap -> handoff topic/a -> message after
    tape.ensure_bootstrap_anchor(source).await.unwrap();
    tape.handoff(
        source,
        "topic/a",
        HandoffState {
            summary: Some("discussed topic A".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    tape.append_message(
        source,
        json!({"role":"user","content":"post-anchor msg"}),
        None,
    )
    .await
    .unwrap();

    // Checkout at topic/a
    let target = "target-tape";
    tape.checkout_anchor(source, "topic/a", target)
        .await
        .unwrap();

    let target_entries = tape.entries(target).await.unwrap();
    let source_entries = tape.entries(source).await.unwrap();

    // Target has the topic/a anchor
    assert!(
        target_entries.iter().any(|e| e.kind == TapEntryKind::Anchor
            && e.payload.get("name").and_then(|v| v.as_str()) == Some("topic/a")),
        "target should contain the topic/a anchor"
    );
    // Target does NOT have post-anchor message
    assert!(
        !target_entries
            .iter()
            .any(|e| e.payload.get("content").and_then(|v| v.as_str()) == Some("post-anchor msg")),
        "target should not contain post-anchor messages"
    );
    // Source is unchanged (still has the post-anchor message)
    assert!(
        source_entries
            .iter()
            .any(|e| e.payload.get("content").and_then(|v| v.as_str()) == Some("post-anchor msg")),
        "source should still contain post-anchor messages"
    );
}

// ---------------------------------------------------------------------------
// Test 2: Multi-level fork chain builds correct tree
// ---------------------------------------------------------------------------

#[tokio::test]
async fn multi_level_fork_chain_builds_correct_tree() {
    let (tape, sessions, _tmp) = setup().await;

    // Root session
    let root = SessionKey::new();
    let root_raw = root.to_string();
    create_session(&sessions, &root, None).await;
    tape.ensure_bootstrap_anchor(&root_raw).await.unwrap();
    tape.handoff(&root_raw, "topic/a", HandoffState::default())
        .await
        .unwrap();

    // Fork 1 from root at topic/a
    let fork1 = SessionKey::new();
    let fork1_raw = fork1.to_string();
    let mut meta1 = None;
    set_fork_metadata(&mut meta1, &root_raw, "topic/a");
    create_session(&sessions, &fork1, meta1).await;
    tape.ensure_bootstrap_anchor(&fork1_raw).await.unwrap();
    tape.handoff(&fork1_raw, "topic/b", HandoffState::default())
        .await
        .unwrap();

    // Fork 2 from fork1 at topic/b
    let fork2 = SessionKey::new();
    let fork2_raw = fork2.to_string();
    let mut meta2 = None;
    set_fork_metadata(&mut meta2, &fork1_raw, "topic/b");
    create_session(&sessions, &fork2, meta2).await;
    tape.ensure_bootstrap_anchor(&fork2_raw).await.unwrap();

    // Build tree from deepest fork
    let tree: AnchorTree = tape
        .build_anchor_tree(&fork2_raw, &*sessions)
        .await
        .unwrap();

    // Tree root should be the original root session
    assert_eq!(tree.root.session_key, root_raw);
    assert_eq!(tree.current_session, fork2_raw);
    // Root has 1 fork
    assert_eq!(tree.root.forks.len(), 1, "root should have exactly 1 fork");
    assert_eq!(tree.root.forks[0].at_anchor, "topic/a");
    // Fork1 has 1 fork
    assert_eq!(
        tree.root.forks[0].branch.forks.len(),
        1,
        "fork1 should have exactly 1 fork"
    );
    assert_eq!(tree.root.forks[0].branch.forks[0].at_anchor, "topic/b");
    // Fork2 is the leaf
    assert_eq!(
        tree.root.forks[0].branch.forks[0].branch.session_key,
        fork2_raw
    );
}

// ---------------------------------------------------------------------------
// Test 3: Checkout + continued conversation doesn't pollute parent
// ---------------------------------------------------------------------------

#[tokio::test]
async fn checkout_then_continue_does_not_pollute_parent() {
    let (tape, _, _tmp) = setup().await;
    let parent = "parent-tape";
    let child = "child-tape";

    tape.ensure_bootstrap_anchor(parent).await.unwrap();
    tape.handoff(parent, "topic/a", HandoffState::default())
        .await
        .unwrap();

    let parent_count_before = tape.entries(parent).await.unwrap().len();

    // Checkout
    tape.checkout_anchor(parent, "topic/a", child)
        .await
        .unwrap();

    // Continue conversation in child
    tape.append_message(child, json!({"role":"user","content":"child msg 1"}), None)
        .await
        .unwrap();
    tape.append_message(
        child,
        json!({"role":"assistant","content":"child reply"}),
        None,
    )
    .await
    .unwrap();

    // Parent unchanged
    let parent_count_after = tape.entries(parent).await.unwrap().len();
    assert_eq!(
        parent_count_before, parent_count_after,
        "parent tape entry count should not change after child append"
    );

    // Child has extra messages
    let child_entries = tape.entries(child).await.unwrap();
    assert!(
        child_entries
            .iter()
            .any(|e| e.payload.get("content").and_then(|v| v.as_str()) == Some("child msg 1")),
        "child should contain its own messages"
    );
}

// ---------------------------------------------------------------------------
// Test 4: Checkout nonexistent anchor returns error
// ---------------------------------------------------------------------------

#[tokio::test]
async fn checkout_nonexistent_anchor_returns_error() {
    let (tape, _, _tmp) = setup().await;
    let source = "source-tape";
    tape.ensure_bootstrap_anchor(source).await.unwrap();

    let result = tape
        .checkout_anchor(source, "does/not/exist", "target")
        .await;
    assert!(
        result.is_err(),
        "checkout of nonexistent anchor should fail"
    );
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("anchor not found"),
        "error should mention 'anchor not found', got: {err}"
    );
}

// ---------------------------------------------------------------------------
// Test 5: Checkout empty tape returns error
// ---------------------------------------------------------------------------

#[tokio::test]
async fn checkout_empty_tape_returns_error() {
    let (tape, _, _tmp) = setup().await;

    let result = tape
        .checkout_anchor("empty-tape", "any-anchor", "target")
        .await;
    assert!(result.is_err(), "checkout from empty tape should fail");
}

// ---------------------------------------------------------------------------
// Test 6: Multiple checkouts from same anchor create independent forks
// ---------------------------------------------------------------------------

#[tokio::test]
async fn multiple_checkouts_create_independent_forks() {
    let (tape, _, _tmp) = setup().await;
    let source = "source-tape";
    tape.ensure_bootstrap_anchor(source).await.unwrap();
    tape.handoff(source, "topic/a", HandoffState::default())
        .await
        .unwrap();

    // Two checkouts from same anchor
    tape.checkout_anchor(source, "topic/a", "fork-1")
        .await
        .unwrap();
    tape.checkout_anchor(source, "topic/a", "fork-2")
        .await
        .unwrap();

    // Add different messages to each fork
    tape.append_message("fork-1", json!({"role":"user","content":"fork1 msg"}), None)
        .await
        .unwrap();
    tape.append_message("fork-2", json!({"role":"user","content":"fork2 msg"}), None)
        .await
        .unwrap();

    let fork1_entries = tape.entries("fork-1").await.unwrap();
    let fork2_entries = tape.entries("fork-2").await.unwrap();

    // Each fork has only its own message
    assert!(
        fork1_entries
            .iter()
            .any(|e| e.payload.get("content").and_then(|v| v.as_str()) == Some("fork1 msg"))
    );
    assert!(
        !fork1_entries
            .iter()
            .any(|e| e.payload.get("content").and_then(|v| v.as_str()) == Some("fork2 msg"))
    );
    assert!(
        fork2_entries
            .iter()
            .any(|e| e.payload.get("content").and_then(|v| v.as_str()) == Some("fork2 msg"))
    );
    assert!(
        !fork2_entries
            .iter()
            .any(|e| e.payload.get("content").and_then(|v| v.as_str()) == Some("fork1 msg"))
    );
}

// ---------------------------------------------------------------------------
// Test 7: Fork metadata round-trip
// ---------------------------------------------------------------------------

#[tokio::test]
async fn fork_metadata_round_trip() {
    // Basic write-then-read
    let mut metadata = None;
    set_fork_metadata(&mut metadata, "parent-key", "topic/design");

    let fm = get_fork_metadata(&metadata).unwrap();
    assert_eq!(fm.forked_from, "parent-key");
    assert_eq!(fm.forked_at_anchor, "topic/design");

    // Preserves other metadata fields
    let mut metadata = Some(json!({"custom": 42}));
    set_fork_metadata(&mut metadata, "p", "a");
    let v = metadata.unwrap();
    assert_eq!(v["custom"], 42);
    assert_eq!(v["forked_from"], "p");
    assert_eq!(v["forked_at_anchor"], "a");
}
