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
//! - `list_sessions_default_filters_to_active` — scenario "list_sessions
//!   defaults to status=active and excludes archived rows".
//! - `list_sessions_status_all_returns_both` — scenario "list_sessions with
//!   status=all returns both active and archived".

use chrono::Utc;
use diesel_async::RunQueryDsl;
use rara_kernel::session::{
    SessionEntry, SessionIndex, SessionKey, SessionListFilter, SessionStatus,
};
use rara_sessions::sqlite_index::SqliteSessionIndex;
use yunara_store::diesel_pool::{DieselPoolConfig, DieselSqlitePools, build_sqlite_pools};

/// Inline DDL mirrors the production migration so the test does not
/// depend on `diesel migration run`. Update both together when the
/// schema changes.
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
    let db_path = std::env::temp_dir().join(format!(
        "rara-issue2043-{}.sqlite",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
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

/// Build a fresh `SessionEntry` with the requested status. `seed` only
/// disambiguates `updated_at` so the ORDER BY in the SQLite query has a
/// deterministic shape across the three rows.
fn entry_with_status(seed: i64, status: SessionStatus) -> SessionEntry {
    let mut now = Utc::now();
    // Compress seed into a sub-second offset so the three fixture rows
    // are distinguishable but still land in the same minute.
    now += chrono::Duration::milliseconds(seed);
    SessionEntry {
        key: SessionKey::new(),
        title: Some(format!("seed-{seed}")),
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
        status,
        metadata: None,
        created_at: now,
        updated_at: now,
    }
}

#[tokio::test]
async fn list_sessions_default_filters_to_active() {
    // Three rows: two `Active`, one `Archived` — the spec's exact
    // fixture for both scenario 1 and scenario 2.
    let pools = fresh_pools().await;
    let idx = SqliteSessionIndex::new(pools);

    idx.create_session(&entry_with_status(1, SessionStatus::Active))
        .await
        .expect("seed active 1");
    idx.create_session(&entry_with_status(2, SessionStatus::Active))
        .await
        .expect("seed active 2");
    idx.create_session(&entry_with_status(3, SessionStatus::Archived))
        .await
        .expect("seed archived");

    let listed = idx
        .list_sessions(50, 0, SessionListFilter::Active)
        .await
        .expect("list");

    assert_eq!(
        listed.len(),
        2,
        "default-filtered list must drop the archived row"
    );
    for entry in &listed {
        assert_eq!(
            entry.status,
            SessionStatus::Active,
            "every returned row must have status=Active, got {:?}",
            entry.status
        );
    }
}

#[tokio::test]
async fn list_sessions_status_all_returns_both() {
    let pools = fresh_pools().await;
    let idx = SqliteSessionIndex::new(pools);

    idx.create_session(&entry_with_status(1, SessionStatus::Active))
        .await
        .expect("seed active 1");
    idx.create_session(&entry_with_status(2, SessionStatus::Active))
        .await
        .expect("seed active 2");
    idx.create_session(&entry_with_status(3, SessionStatus::Archived))
        .await
        .expect("seed archived");

    let listed = idx
        .list_sessions(50, 0, SessionListFilter::All)
        .await
        .expect("list");

    assert_eq!(listed.len(), 3, "All filter must return every row");

    // Both statuses present.
    assert!(listed.iter().any(|e| e.status == SessionStatus::Active));
    assert!(listed.iter().any(|e| e.status == SessionStatus::Archived));

    // Ordered by updated_at DESC.
    let timestamps: Vec<_> = listed.iter().map(|e| e.updated_at).collect();
    let mut sorted = timestamps.clone();
    sorted.sort_by(|a, b| b.cmp(a));
    assert_eq!(
        timestamps, sorted,
        "All filter must keep the updated_at DESC ordering"
    );
}
