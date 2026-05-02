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

//! BDD bindings for `specs/issue-2043-session-status.spec.md`:
//!
//! - `patch_session_archive_round_trip` — scenario "PATCH
//!   /api/v1/chat/sessions/{key} archives a session via status field". Driven
//!   through the kernel's `SessionIndex::update_session` path (the same call
//!   apply_session_patch ends up making) plus the list filter, since the
//!   HTTP-level `SessionPatch` shape lives in `rara-backend-admin`.
//! - `tape_append_preserves_archived_status` — scenario "appending to an
//!   archived session does not unarchive it". Drives the TapeService append
//!   path against an archived session and asserts that the derived-state
//!   writeback never flips `status` back to `Active`.

use std::sync::Arc;

use chrono::Utc;
use diesel_async::RunQueryDsl;
use rara_kernel::{
    memory::{FileTapeStore, TapeService},
    session::{
        SessionEntry, SessionIndex, SessionIndexRef, SessionKey, SessionListFilter, SessionStatus,
    },
};
use rara_sessions::sqlite_index::SqliteSessionIndex;
use serde_json::json;
use yunara_store::diesel_pool::{DieselPoolConfig, DieselSqlitePools, build_sqlite_pools};

const SESSIONS_DDL: &[&str] = &[
    "CREATE TABLE sessions (
        key                          TEXT NOT NULL PRIMARY KEY,
        title                        TEXT,
        model                        TEXT,
        model_provider               TEXT,
        thinking_level               TEXT,
        system_prompt                TEXT,
        total_entries                INTEGER NOT NULL DEFAULT 0,
        preview                      TEXT,
        last_token_usage             INTEGER,
        estimated_context_tokens     INTEGER NOT NULL DEFAULT 0,
        entries_since_last_anchor    INTEGER NOT NULL DEFAULT 0,
        anchors_json                 TEXT NOT NULL DEFAULT '[]',
        metadata                     TEXT,
        created_at                   TEXT NOT NULL,
        updated_at                   TEXT NOT NULL,
        status                       TEXT NOT NULL DEFAULT 'active'
            CHECK (status IN ('active', 'archived'))
    ) WITHOUT ROWID",
    "CREATE INDEX idx_sessions_updated_at ON sessions (updated_at DESC)",
    "CREATE INDEX idx_sessions_status_updated_at
        ON sessions (status, updated_at DESC)",
    "CREATE TABLE session_channel_bindings (
        channel_type TEXT NOT NULL,
        chat_id      TEXT NOT NULL,
        thread_id    TEXT,
        session_key  TEXT NOT NULL,
        created_at   TEXT NOT NULL,
        updated_at   TEXT NOT NULL,
        PRIMARY KEY (channel_type, chat_id, thread_id)
    )",
];

async fn fresh_pools() -> DieselSqlitePools {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let db_path = std::env::temp_dir().join(format!(
        "rara-issue2043-kernel-{}-{}-{}.sqlite",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        seq
    ));
    let pools = build_sqlite_pools(
        &DieselPoolConfig::builder()
            .database_url(db_path.to_string_lossy().into_owned())
            .max_connections(1)
            .build(),
    )
    .await
    .expect("pool");
    let mut conn = pools.writer.get().await.expect("conn");
    for ddl in SESSIONS_DDL {
        diesel::sql_query(*ddl)
            .execute(&mut *conn)
            .await
            .expect("ddl");
    }
    drop(conn);
    pools
}

fn fresh_session_entry() -> SessionEntry {
    let now = Utc::now();
    SessionEntry {
        key: SessionKey::new(),
        title: Some("test".to_owned()),
        model: None,
        model_provider: None,
        thinking_level: None,
        system_prompt: None,
        total_entries: 0,
        preview: None,
        last_token_usage: None,
        estimated_context_tokens: 0,
        entries_since_last_anchor: 0,
        anchors: Vec::new(),
        status: SessionStatus::Active,
        metadata: None,
        created_at: now,
        updated_at: now,
    }
}

#[tokio::test]
async fn patch_session_archive_round_trip() {
    let pools = fresh_pools().await;
    let idx: SessionIndexRef = Arc::new(SqliteSessionIndex::new(pools));

    let mut entry = fresh_session_entry();
    let key = entry.key;
    idx.create_session(&entry).await.expect("create");

    let original_updated_at = entry.updated_at;
    // Simulate `apply_session_patch(status: Some(Archived))` followed
    // by the service-level `updated_at = Utc::now()` bump and the
    // index `update_session` write.
    entry.status = SessionStatus::Archived;
    entry.updated_at = Utc::now() + chrono::Duration::seconds(1);
    idx.update_session(&entry).await.expect("patch");

    // 1. `get_session` reflects the archived status and bumped timestamp.
    let fetched = idx.get_session(&key).await.expect("get").expect("present");
    assert_eq!(fetched.status, SessionStatus::Archived);
    assert!(
        fetched.updated_at > original_updated_at,
        "updated_at must advance to the patch timestamp"
    );

    // 2. Default-filtered list excludes the row.
    let active_listed = idx
        .list_sessions(50, 0, SessionListFilter::Active)
        .await
        .expect("list active");
    assert!(
        !active_listed.iter().any(|e| e.key == key),
        "archived session must drop out of the default Active list"
    );

    // 3. Archived-filtered list includes the row.
    let archived_listed = idx
        .list_sessions(50, 0, SessionListFilter::Archived)
        .await
        .expect("list archived");
    assert!(
        archived_listed.iter().any(|e| e.key == key),
        "Archived filter must surface the session"
    );
}

#[tokio::test]
async fn tape_append_preserves_archived_status() {
    let pools = fresh_pools().await;
    let index_ref = Arc::new(SqliteSessionIndex::new(pools));
    let dyn_index: SessionIndexRef = index_ref.clone();

    let tape_root = tempfile::tempdir().expect("tempdir");
    let store = FileTapeStore::new(tape_root.path(), tape_root.path())
        .await
        .expect("store");
    let tape = TapeService::new(store).with_session_index(dyn_index);

    // Stage: an archived session with 5 tape entries appended via the
    // production write path.
    let mut entry = fresh_session_entry();
    let key = entry.key;
    index_ref.create_session(&entry).await.expect("create");
    let tape_name = key.to_string();
    for i in 0..5 {
        tape.append_message(
            &tape_name,
            json!({"role": "user", "content": format!("pre-archive #{i}")}),
            None,
        )
        .await
        .expect("append pre-archive");
    }
    // Now flip to archived (mirrors what the PATCH handler does).
    entry.status = SessionStatus::Archived;
    entry.updated_at = Utc::now() + chrono::Duration::seconds(1);
    index_ref.update_session(&entry).await.expect("archive");

    // Sanity: the row is archived with 5 entries before the next append.
    let pre_append = index_ref
        .get_session(&key)
        .await
        .expect("get pre-append")
        .expect("present");
    assert_eq!(pre_append.status, SessionStatus::Archived);
    assert_eq!(pre_append.total_entries, 5);

    // Action: write a sixth message — this triggers
    // `update_session_derived` on the derived-state path that the
    // archive bit must NOT travel through.
    tape.append_message(
        &tape_name,
        json!({"role": "user", "content": "post-archive append"}),
        None,
    )
    .await
    .expect("append post-archive");

    let post_append = index_ref
        .get_session(&key)
        .await
        .expect("get post-append")
        .expect("present");
    assert_eq!(
        post_append.status,
        SessionStatus::Archived,
        "tape append must not reset status to Active"
    );
    assert_eq!(post_append.total_entries, 6, "the 6th entry was recorded");
}
