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
//! - **Scenario 4** (`boot_migration_is_idempotent`) — legacy JSON sessions
//!   migrate into SQLite once; a second pass is a no-op and the source files
//!   end up under `index_dir/legacy/`.
//! - **Scenario 6** (`list_sessions_uses_updated_at_index`) — `EXPLAIN QUERY
//!   PLAN` for `list_sessions(50, 0)` confirms SQLite uses
//!   `idx_sessions_updated_at`, not a full-table TEMP B-TREE sort.

use chrono::Utc;
use diesel_async::RunQueryDsl;
use rara_kernel::{
    channel::types::ChannelType,
    session::{ChannelBinding, SessionEntry, SessionIndex, SessionKey},
};
use rara_sessions::{file_index::FileSessionIndex, sqlite_index::SqliteSessionIndex};
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
        status                       TEXT NOT NULL DEFAULT 'active'
            CHECK (status IN ('active', 'archived')),
        created_at                   TEXT NOT NULL,
        updated_at                   TEXT NOT NULL
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
    "CREATE INDEX idx_session_channel_bindings_session_key
        ON session_channel_bindings (session_key)",
];

async fn fresh_pools() -> DieselSqlitePools {
    let db_path = std::env::temp_dir().join(format!(
        "rara-issue2025-{}.sqlite",
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

fn sample_entry(title: &str) -> SessionEntry {
    let now = Utc::now();
    SessionEntry {
        key: SessionKey::new(),
        title: Some(title.to_owned()),
        model: Some("test-model".to_owned()),
        model_provider: Some("test".to_owned()),
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

#[tokio::test]
async fn boot_migration_is_idempotent() {
    // Stage: a legacy JSON index dir with N session files and M
    // bindings, populated via `FileSessionIndex` so we know the layout
    // matches what production wrote on disk before this PR.
    let tmp = tempfile::tempdir().expect("tempdir");
    let index_dir = tmp.path().to_path_buf();
    let file_idx = FileSessionIndex::new(index_dir.clone())
        .await
        .expect("file idx");

    let entry_a = sample_entry("alpha");
    let entry_b = sample_entry("beta");
    file_idx.create_session(&entry_a).await.expect("seed a");
    file_idx.create_session(&entry_b).await.expect("seed b");

    let binding = ChannelBinding {
        channel_type: ChannelType::Cli,
        chat_id:      "lane-1".to_owned(),
        thread_id:    None,
        session_key:  entry_a.key,
        created_at:   Utc::now(),
        updated_at:   Utc::now(),
    };
    file_idx.bind_channel(&binding).await.expect("bind");
    drop(file_idx);

    // First migration pass — moves both files into SQLite, then moves
    // the source JSON into `legacy/`.
    let pools = fresh_pools().await;
    let sqlite_idx = SqliteSessionIndex::new(pools.clone());
    let migrated = sqlite_idx
        .ensure_migrated_from(&index_dir)
        .await
        .expect("migrate");
    assert_eq!(migrated, 2, "two sessions migrated");

    let listed = sqlite_idx
        .list_sessions(100, 0, rara_kernel::session::SessionListFilter::All)
        .await
        .expect("list after migration");
    assert_eq!(listed.len(), 2);
    assert!(listed.iter().any(|e| e.title.as_deref() == Some("alpha")));
    assert!(listed.iter().any(|e| e.title.as_deref() == Some("beta")));

    let resolved = sqlite_idx
        .get_channel_binding(ChannelType::Cli, "lane-1", None)
        .await
        .expect("binding lookup")
        .expect("binding present");
    assert_eq!(resolved.session_key, entry_a.key);

    // The original JSON files must have moved into legacy/.
    let legacy = index_dir.join("legacy");
    assert!(legacy.exists(), "legacy/ created");
    let json_remaining = std::fs::read_dir(&index_dir)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| {
            e.path().is_file() && e.path().extension().and_then(|s| s.to_str()) == Some("json")
        })
        .count();
    assert_eq!(json_remaining, 0, "no top-level JSON files remain");

    // Second pass: SQLite is non-empty, so the migration is a no-op
    // (returns 0 migrated, doesn't touch the filesystem).
    let migrated_again = sqlite_idx
        .ensure_migrated_from(&index_dir)
        .await
        .expect("idempotent migrate");
    assert_eq!(migrated_again, 0, "second pass is a no-op");
    let listed_again = sqlite_idx
        .list_sessions(100, 0, rara_kernel::session::SessionListFilter::All)
        .await
        .expect("re-list");
    assert_eq!(listed_again.len(), 2, "no duplicate rows");
}

#[tokio::test]
async fn list_sessions_uses_updated_at_index() {
    let pools = fresh_pools().await;
    let idx = SqliteSessionIndex::new(pools.clone());

    // Seed enough rows that a missing index would be visible in the
    // plan as a temp-sort step.
    for i in 0..50 {
        let mut entry = sample_entry(&format!("session-{i}"));
        entry.updated_at = Utc::now();
        idx.create_session(&entry).await.expect("create");
    }

    let mut conn = pools.reader.get().await.expect("conn");
    let rows: Vec<(String, String, String, String, Option<String>)> = diesel::sql_query(
        "EXPLAIN QUERY PLAN SELECT * FROM sessions ORDER BY updated_at DESC LIMIT 50 OFFSET 0",
    )
    .load::<ExplainRow>(&mut *conn)
    .await
    .expect("explain")
    .into_iter()
    .map(|r| {
        (
            r.id.unwrap_or_default(),
            r.parent.unwrap_or_default(),
            r.notused.unwrap_or_default(),
            r.detail.clone(),
            Some(r.detail),
        )
    })
    .collect();

    let plan_text = rows
        .iter()
        .map(|r| r.3.clone())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        plan_text.contains("idx_sessions_updated_at"),
        "EXPLAIN QUERY PLAN must mention idx_sessions_updated_at; got:\n{plan_text}"
    );
    assert!(
        !plan_text.contains("USE TEMP B-TREE FOR ORDER BY"),
        "ORDER BY must not require a temp B-tree; got:\n{plan_text}"
    );
}

#[derive(diesel::QueryableByName, Debug)]
struct ExplainRow {
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
    id:      Option<String>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
    parent:  Option<String>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
    notused: Option<String>,
    #[diesel(sql_type = diesel::sql_types::Text)]
    detail:  String,
}
