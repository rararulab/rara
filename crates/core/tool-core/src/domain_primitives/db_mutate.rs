// Copyright 2025 Crrow
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

//! Generic database write tool (INSERT / UPDATE only, no DELETE).
//!
//! Table and column names are validated against compile-time whitelists.
//! Values are always bound as query parameters.

use async_trait::async_trait;
use serde_json::json;
use sqlx::{PgPool, Row};

use crate::AgentTool;

/// Allowed tables and their mutable columns.
///
/// `id`, `created_at`, and `updated_at` are excluded from mutation — the
/// database manages those via defaults and triggers.
const MUTABLE_WHITELIST: &[(&str, &[&str])] = &[
    (
        "saved_job",
        &[
            "url",
            "title",
            "company",
            "status",
            "markdown_s3_key",
            "markdown_preview",
            "match_score",
            "error_message",
            "crawled_at",
            "analyzed_at",
            "expires_at",
        ],
    ),
    (
        "application",
        &[
            "job_id",
            "resume_id",
            "channel",
            "status",
            "priority",
            "cover_letter",
            "notes",
            "submitted_at",
        ],
    ),
    (
        "resume",
        &[
            "title",
            "version_tag",
            "content_hash",
            "source",
            "content",
            "target_job_id",
            "parent_resume_id",
            "customization_notes",
        ],
    ),
    (
        "interview_plan",
        &[
            "application_id",
            "title",
            "description",
            "company",
            "position",
            "job_description",
            "round",
            "scheduled_at",
            "task_status",
            "notes",
        ],
    ),
];

/// Layer 1 primitive: safe INSERT / UPDATE against whitelisted tables.
pub struct DbMutateTool {
    pool: PgPool,
}

impl DbMutateTool {
    pub fn new(pool: PgPool) -> Self { Self { pool } }
}

fn mutable_columns(table: &str) -> Option<&'static [&'static str]> {
    MUTABLE_WHITELIST
        .iter()
        .find(|(t, _)| *t == table)
        .map(|(_, cols)| *cols)
}

#[async_trait]
impl AgentTool for DbMutateTool {
    fn name(&self) -> &str { "db_mutate" }

    fn description(&self) -> &str {
        "Create or update records in database tables. Allowed tables: saved_job, application, \
         resume, interview_plan. Actions: \"create\" (INSERT) or \"update\" (UPDATE by id). DELETE \
         is not supported. Returns the created/updated record id."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "table": {
                    "type": "string",
                    "description": "Table to mutate: saved_job, application, resume, interview_plan"
                },
                "action": {
                    "type": "string",
                    "enum": ["create", "update"],
                    "description": "create = INSERT, update = UPDATE"
                },
                "data": {
                    "type": "object",
                    "description": "Column-value pairs to write"
                },
                "id": {
                    "type": "string",
                    "description": "Record UUID (required for update)"
                }
            },
            "required": ["table", "action", "data"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        let table = params
            .get("table")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: table"))?;

        let action = params
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: action"))?;

        let data = params
            .get("data")
            .and_then(|v| v.as_object())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: data"))?;

        let columns = mutable_columns(table)
            .ok_or_else(|| anyhow::anyhow!("table not allowed: {table}"))?;

        // Validate all data keys.
        for col in data.keys() {
            if !columns.contains(&col.as_str()) {
                return Ok(json!({
                    "error": format!("column not allowed for table {table}: {col}")
                }));
            }
        }

        if data.is_empty() {
            return Ok(json!({ "error": "data must not be empty" }));
        }

        match action {
            "create" => execute_insert(&self.pool, table, data).await,
            "update" => {
                let id = params
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        anyhow::anyhow!("missing required parameter: id (for update)")
                    })?;
                execute_update(&self.pool, table, id, data).await
            }
            other => Ok(json!({ "error": format!("unknown action: {other}") })),
        }
    }
}

async fn execute_insert(
    pool: &PgPool,
    table: &str,
    data: &serde_json::Map<String, serde_json::Value>,
) -> anyhow::Result<serde_json::Value> {
    let cols: Vec<&str> = data.keys().map(|k| k.as_str()).collect();
    let placeholders: Vec<String> = (1..=cols.len()).map(|i| format!("${i}")).collect();

    let sql = format!(
        "INSERT INTO {} ({}) VALUES ({}) RETURNING id",
        table,
        cols.join(", "),
        placeholders.join(", ")
    );

    let mut query = sqlx::query(&sql);
    for val in data.values() {
        query = bind_json_value(query, val);
    }

    match query.fetch_one(pool).await {
        Ok(row) => {
            let id: uuid::Uuid = row.try_get("id").unwrap_or_default();
            Ok(json!({ "id": id.to_string(), "action": "created" }))
        }
        Err(e) => Ok(json!({ "error": format!("{e}") })),
    }
}

async fn execute_update(
    pool: &PgPool,
    table: &str,
    id: &str,
    data: &serde_json::Map<String, serde_json::Value>,
) -> anyhow::Result<serde_json::Value> {
    let id_uuid =
        uuid::Uuid::parse_str(id).map_err(|e| anyhow::anyhow!("invalid UUID: {e}"))?;

    let set_clauses: Vec<String> = data
        .keys()
        .enumerate()
        .map(|(i, col)| format!("{col} = ${}", i + 1))
        .collect();

    let sql = format!(
        "UPDATE {} SET {} WHERE id = ${} RETURNING id",
        table,
        set_clauses.join(", "),
        data.len() + 1
    );

    let mut query = sqlx::query(&sql);
    for val in data.values() {
        query = bind_json_value(query, val);
    }
    query = query.bind(id_uuid);

    match query.fetch_one(pool).await {
        Ok(row) => {
            let id: uuid::Uuid = row.try_get("id").unwrap_or_default();
            Ok(json!({ "id": id.to_string(), "action": "updated" }))
        }
        Err(e) => Ok(json!({ "error": format!("{e}") })),
    }
}

/// Bind a JSON value to a sqlx query as the appropriate Rust type.
fn bind_json_value<'q>(
    query: sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments>,
    val: &'q serde_json::Value,
) -> sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments> {
    match val {
        serde_json::Value::String(s) => query.bind(s.as_str()),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                query.bind(i as i32)
            } else if let Some(f) = n.as_f64() {
                query.bind(f as f32)
            } else {
                query.bind(n.to_string())
            }
        }
        serde_json::Value::Bool(b) => query.bind(*b),
        serde_json::Value::Null => query.bind(None::<String>),
        other => query.bind(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whitelist_rejects_unknown_table() {
        assert!(mutable_columns("users").is_none());
        assert!(mutable_columns("saved_job").is_some());
    }

    #[test]
    fn whitelist_excludes_id_and_timestamps() {
        let cols = mutable_columns("saved_job").unwrap();
        assert!(!cols.contains(&"id"));
        assert!(!cols.contains(&"created_at"));
        assert!(!cols.contains(&"updated_at"));
    }

    #[test]
    fn all_mutable_tables_have_columns() {
        for (table, cols) in MUTABLE_WHITELIST {
            assert!(!table.is_empty());
            assert!(!cols.is_empty(), "table {table} has no mutable columns");
        }
    }
}
