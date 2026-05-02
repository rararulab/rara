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

//! Issue #2025 BDD scenarios bound here:
//!
//! - **Scenario 1** (`list_sessions_reflects_tape_state`)
//! - **Scenario 2** (`append_message_updates_index_in_band`)
//! - **Scenario 3** (`anchor_append_records_byte_offset_and_resets_segment`)
//! - **Scenario 5** (`crash_recovery_rebuild_repairs_out_of_sync_row`)
//! - **Scenario 7** (`rebuild_single_session_leaves_others_alone`)
//!
//! Every scenario must FAIL against `pre-#2025` `FileSessionIndex`
//! semantics (where `total_entries` was a static `0` set at create
//! time) and PASS with the new SQLite-backed index plus the
//! `TapeService::record_append` hook.

use std::sync::Arc;

use chrono::Utc;
use diesel_async::RunQueryDsl;
use rara_kernel::{
    memory::{FileTapeStore, TapEntryKind, TapeService},
    session::{
        AnchorRef, SessionDerivedState, SessionEntry, SessionIndex, SessionIndexRef, SessionKey,
    },
};
use rara_sessions::sqlite_index::{ReconcileTape, SqliteSessionIndex, TapeReport};
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
        updated_at                   TEXT NOT NULL
    ) WITHOUT ROWID",
    "CREATE INDEX idx_sessions_updated_at ON sessions (updated_at DESC)",
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
        "rara-issue2025-kernel-{}-{}-{}.sqlite",
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

struct Harness {
    tape:       TapeService,
    index_ref:  Arc<SqliteSessionIndex>,
    tape_root:  tempfile::TempDir,
    /// Path the tape worker's `find_tape_file` sees (a separate handle
    /// for byte-offset assertions in scenario 3).
    memory_dir: std::path::PathBuf,
}

async fn setup() -> Harness {
    let pools = fresh_pools().await;
    let index_ref = Arc::new(SqliteSessionIndex::new(pools));
    let dyn_index: SessionIndexRef = index_ref.clone();

    let tape_root = tempfile::tempdir().expect("tempdir");
    let store = FileTapeStore::new(tape_root.path(), tape_root.path())
        .await
        .expect("store");
    let tape = TapeService::new(store).with_session_index(dyn_index);

    Harness {
        tape,
        index_ref,
        memory_dir: tape_root.path().to_path_buf(),
        tape_root,
    }
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
        status: rara_kernel::session::SessionStatus::Active,
        metadata: None,
        created_at: now,
        updated_at: now,
    }
}

// ---------------------------------------------------------------------------
// Scenario 1
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_sessions_reflects_tape_state() {
    let h = setup().await;
    let entry = fresh_session_entry();
    let key = entry.key;
    h.index_ref
        .create_session(&entry)
        .await
        .expect("create session");

    // Stage: write 47 message entries, 3 anchors, 12 events (62 total),
    // mixing kinds to mirror real traffic.
    let tape_name = key.to_string();
    for i in 0..47 {
        h.tape
            .append_message(
                &tape_name,
                json!({"role": if i % 2 == 0 {"user"} else {"assistant"},
                       "content": format!("hi #{i}")}),
                None,
            )
            .await
            .expect("append message");
    }
    for i in 0..12 {
        h.tape
            .append_event(&tape_name, "noise", json!({"i": i}))
            .await
            .expect("append event");
    }
    for i in 0..3 {
        h.tape
            .handoff(&tape_name, &format!("chapter-{i}"), Default::default())
            .await
            .expect("handoff");
    }
    // 3 entries appended after the last anchor:
    for i in 0..3 {
        h.tape
            .append_message(
                &tape_name,
                json!({"role": "user", "content": format!("post-anchor #{i}")}),
                None,
            )
            .await
            .expect("post-anchor message");
    }

    let listed = h
        .index_ref
        .list_sessions(50, 0, rara_kernel::session::SessionListFilter::All)
        .await
        .expect("list sessions");
    let row = listed
        .iter()
        .find(|e| e.key == key)
        .expect("session row present");

    assert_eq!(
        row.total_entries, 65,
        "47 messages + 12 events + 3 anchors + 3 post-anchor = 65 entries"
    );
    assert_eq!(row.anchors.len(), 3, "3 anchor entries recorded");
    assert_eq!(
        row.entries_since_last_anchor, 3,
        "3 messages appended after the last anchor"
    );
    let now = Utc::now();
    let drift = (now - row.updated_at).num_seconds().abs();
    assert!(
        drift <= 5,
        "updated_at within 5s of wall clock; drift={drift}s"
    );
}

// ---------------------------------------------------------------------------
// Scenario 2
// ---------------------------------------------------------------------------

#[tokio::test]
async fn append_message_updates_index_in_band() {
    let h = setup().await;
    let entry = fresh_session_entry();
    let key = entry.key;
    h.index_ref
        .create_session(&entry)
        .await
        .expect("create session");
    let tape_name = key.to_string();
    // Seed N entries.
    for i in 0..7 {
        h.tape
            .append_message(
                &tape_name,
                json!({"role": "user", "content": format!("seed #{i}")}),
                None,
            )
            .await
            .expect("seed");
    }

    let before = h
        .index_ref
        .get_session(&key)
        .await
        .expect("get")
        .expect("present");
    let n = before.total_entries;
    let t0 = before.updated_at;

    // Sleep one tick so the timestamp comparison is meaningful.
    tokio::time::sleep(std::time::Duration::from_millis(15)).await;

    h.tape
        .append_message(
            &tape_name,
            json!({"role": "user", "content": "the next one"}),
            None,
        )
        .await
        .expect("append");

    let after = h
        .index_ref
        .get_session(&key)
        .await
        .expect("get post")
        .expect("present");
    assert_eq!(after.total_entries, n + 1);
    assert!(
        after.updated_at >= t0,
        "updated_at advanced monotonically: t0={t0}, after={}",
        after.updated_at
    );
}

// ---------------------------------------------------------------------------
// Scenario 3
// ---------------------------------------------------------------------------

#[tokio::test]
async fn anchor_append_records_byte_offset_and_resets_segment() {
    let h = setup().await;
    let entry = fresh_session_entry();
    let key = entry.key;
    h.index_ref
        .create_session(&entry)
        .await
        .expect("create session");
    let tape_name = key.to_string();

    // Seed K = 4 messages so `entries_since_last_anchor` becomes K+1
    // when we add the bootstrap anchor below; for this scenario we want
    // a baseline where we control the number of pre-anchor entries.
    for i in 0..4 {
        h.tape
            .append_message(
                &tape_name,
                json!({"role": "user", "content": format!("pre #{i}")}),
                None,
            )
            .await
            .expect("pre msg");
    }

    // Capture file size F at this moment — the anchor about to land
    // will live at byte offset F.
    let path =
        rara_kernel::memory::find_tape_file(&h.memory_dir, &tape_name).expect("tape file exists");
    let f_before = std::fs::metadata(&path).expect("stat").len();

    h.tape
        .handoff(&tape_name, "chapter-2", Default::default())
        .await
        .expect("handoff");

    let after = h
        .index_ref
        .get_session(&key)
        .await
        .expect("get")
        .expect("present");
    let last_anchor = after.anchors.last().expect("at least one anchor");
    assert_eq!(last_anchor.name, "chapter-2");
    assert_eq!(
        last_anchor.byte_offset, f_before,
        "anchor byte_offset must equal the pre-anchor file size"
    );
    assert_eq!(
        last_anchor.entry_count_in_segment, 5,
        "4 pre-anchor messages + the anchor itself = 5 entries in the segment"
    );
    assert_eq!(
        after.entries_since_last_anchor, 0,
        "since-anchor counter must reset to 0"
    );

    // Decode the JSONL line at `byte_offset` and verify it is the
    // anchor entry we just wrote.
    let bytes = std::fs::read(&path).expect("read tape");
    let tail = &bytes[last_anchor.byte_offset as usize..];
    let line_end = tail.iter().position(|b| *b == b'\n').unwrap_or(tail.len());
    let line = std::str::from_utf8(&tail[..line_end]).expect("utf8");
    let decoded: rara_kernel::memory::TapEntry =
        serde_json::from_str(line).expect("decode anchor line");
    assert_eq!(decoded.kind, TapEntryKind::Anchor);
    assert_eq!(decoded.id, last_anchor.anchor_id);
}

// ---------------------------------------------------------------------------
// Scenarios 5 & 7 — boot reconcile + rescue rebuild
// ---------------------------------------------------------------------------

/// Adapter wiring `TapeService` into `ReconcileTape` for the test —
/// a slim copy of the production `boot.rs` adapter.
struct TestReconciler {
    tape:       TapeService,
    memory_dir: std::path::PathBuf,
}

#[async_trait::async_trait]
impl ReconcileTape for TestReconciler {
    async fn read_tape(&self, key: &SessionKey) -> Option<TapeReport> {
        let tape_name = key.to_string();
        let info = self.tape.info(&tape_name).await.ok()?;
        let entries = self.tape.entries(&tape_name).await.ok()?;
        let updated_at = entries
            .last()
            .map(|e| {
                let secs = e.timestamp.as_second();
                let ns = e.timestamp.subsec_nanosecond();
                let (s, n) = if ns < 0 {
                    (secs.saturating_sub(1), ns.saturating_add(1_000_000_000))
                } else {
                    (secs, ns)
                };
                chrono::DateTime::<chrono::Utc>::from_timestamp(s, n as u32)
                    .unwrap_or_else(chrono::Utc::now)
            })
            .unwrap_or_else(chrono::Utc::now);
        let path = rara_kernel::memory::find_tape_file(&self.memory_dir, &tape_name)?;
        let bytes = tokio::fs::read(&path).await.ok()?;
        let mut anchors = Vec::new();
        let mut offset: u64 = 0;
        let mut seg: i64 = 0;
        for line in bytes.split(|b| *b == b'\n') {
            let line_start = offset;
            offset += line.len() as u64 + 1;
            if line.is_empty() {
                continue;
            }
            let entry: rara_kernel::memory::TapEntry = match serde_json::from_slice(line) {
                Ok(e) => e,
                Err(_) => continue,
            };
            seg += 1;
            if entry.kind == TapEntryKind::Anchor {
                let name = entry
                    .payload
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("-")
                    .to_owned();
                let secs = entry.timestamp.as_second();
                let ns = entry.timestamp.subsec_nanosecond();
                let (s, n) = if ns < 0 {
                    (secs.saturating_sub(1), ns.saturating_add(1_000_000_000))
                } else {
                    (secs, ns)
                };
                let ts = chrono::DateTime::<chrono::Utc>::from_timestamp(s, n as u32)
                    .unwrap_or_else(chrono::Utc::now);
                anchors.push(AnchorRef {
                    anchor_id: entry.id,
                    byte_offset: line_start,
                    name,
                    timestamp: ts,
                    entry_count_in_segment: seg,
                });
                seg = 0;
            }
        }
        Some(TapeReport {
            total_entries: info.entries as i64,
            updated_at,
            last_token_usage: info.last_token_usage.map(|x| x as i64),
            estimated_context_tokens: info.estimated_context_tokens as i64,
            entries_since_last_anchor: info.entries_since_last_anchor as i64,
            anchors,
            preview: None,
        })
    }
}

#[tokio::test]
async fn crash_recovery_rebuild_repairs_out_of_sync_row() {
    let h = setup().await;
    let entry = fresh_session_entry();
    let key = entry.key;
    h.index_ref.create_session(&entry).await.expect("create");
    let tape_name = key.to_string();
    for i in 0..10 {
        h.tape
            .append_message(
                &tape_name,
                json!({"role": "user", "content": format!("m #{i}")}),
                None,
            )
            .await
            .expect("append");
    }

    // Simulate a crash that lost the last 3 derived-state writes by
    // forcing the row to a known-bad state.
    let bad = SessionDerivedState::builder()
        .total_entries(7)
        .updated_at(Utc::now())
        .estimated_context_tokens(0)
        .entries_since_last_anchor(0)
        .anchors(Vec::new())
        .build();
    h.index_ref
        .update_session_derived(&key, &bad)
        .await
        .expect("force bad row");
    let stale = h.index_ref.get_session(&key).await.unwrap().unwrap();
    assert_eq!(stale.total_entries, 7);

    // Hash the JSONL on disk so the post-rebuild assertion can prove
    // the file was not touched.
    let path = rara_kernel::memory::find_tape_file(&h.memory_dir, &tape_name).expect("tape exists");
    let bytes_before = std::fs::read(&path).expect("read tape");

    let recon = TestReconciler {
        tape:       h.tape.clone(),
        memory_dir: h.memory_dir.clone(),
    };
    let repaired = h
        .index_ref
        .reconcile_all(recon)
        .await
        .expect("reconcile_all");
    assert!(repaired >= 1, "at least one row repaired");

    let healed = h.index_ref.get_session(&key).await.unwrap().unwrap();
    assert_eq!(healed.total_entries, 10, "row repaired to match disk");

    let bytes_after = std::fs::read(&path).expect("re-read tape");
    assert_eq!(
        bytes_before, bytes_after,
        "the rebuild must not mutate the JSONL file"
    );
}

#[tokio::test]
async fn rebuild_single_session_leaves_others_alone() {
    let h = setup().await;
    let target = fresh_session_entry();
    let target_key = target.key;
    let bystander = fresh_session_entry();
    let bystander_key = bystander.key;

    h.index_ref.create_session(&target).await.expect("create");
    h.index_ref
        .create_session(&bystander)
        .await
        .expect("create bystander");

    let target_tape = target_key.to_string();
    for _ in 0..5 {
        h.tape
            .handoff(&target_tape, "checkpoint", Default::default())
            .await
            .expect("anchor");
    }

    let bystander_tape = bystander_key.to_string();
    for i in 0..3 {
        h.tape
            .append_message(
                &bystander_tape,
                json!({"role": "user", "content": format!("by-#{i}")}),
                None,
            )
            .await
            .expect("by msg");
    }
    let bystander_before = h
        .index_ref
        .get_session(&bystander_key)
        .await
        .unwrap()
        .unwrap();

    // Corrupt the target's row.
    let bad = SessionDerivedState::builder()
        .total_entries(0)
        .updated_at(Utc::now())
        .estimated_context_tokens(0)
        .entries_since_last_anchor(0)
        .anchors(Vec::new())
        .build();
    h.index_ref
        .update_session_derived(&target_key, &bad)
        .await
        .unwrap();

    let recon = TestReconciler {
        tape:       h.tape.clone(),
        memory_dir: h.memory_dir.clone(),
    };
    let report = recon.read_tape(&target_key).await.expect("report present");
    h.index_ref
        .rebuild_session_with_report(&target_key, &report)
        .await
        .expect("rebuild");

    let target_after = h.index_ref.get_session(&target_key).await.unwrap().unwrap();
    assert_eq!(target_after.anchors.len(), 5);
    assert!(target_after.total_entries >= 5);

    let bystander_after = h
        .index_ref
        .get_session(&bystander_key)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        bystander_after.total_entries, bystander_before.total_entries,
        "bystander must be untouched by single-key rebuild"
    );
}

// Force the harness to drop in a deterministic order so the temp dir
// outlives the tape service.
impl Drop for Harness {
    fn drop(&mut self) {
        // Nothing to do — tempdir cleanup runs automatically.
        let _ = &self.tape_root;
    }
}
