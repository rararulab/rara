// Copyright 2026 Rararulab
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

use diesel::{Connection, QueryableByName, RunQueryDsl, sql_types::Text, sqlite::SqliteConnection};
use diesel_migrations::{EmbeddedMigrations, MigrationHarness, embed_migrations};

const MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations");

#[derive(Debug, QueryableByName)]
struct IndexRow {
    #[diesel(sql_type = Text)]
    name: String,
}

#[test]
fn data_feed_event_query_indexes_are_migrated() {
    let db = tempfile::NamedTempFile::new().expect("temp sqlite db");
    let url = db.path().to_string_lossy();
    let mut conn = SqliteConnection::establish(&url).expect("open sqlite db");
    conn.run_pending_migrations(MIGRATIONS)
        .expect("run rara-model migrations");

    let indexes: Vec<IndexRow> = diesel::sql_query("PRAGMA index_list(data_feed_events)")
        .load(&mut conn)
        .expect("list data_feed_events indexes");
    let names = indexes
        .into_iter()
        .map(|index| index.name)
        .collect::<std::collections::HashSet<_>>();

    assert!(names.contains("idx_data_feed_events_source_received_created_id"));
    assert!(names.contains("idx_data_feed_events_source_type_received_created_id"));
}
