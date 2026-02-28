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

//! Generic database query tool (read-only).
//!
//! Table and column names are validated against a compile-time whitelist.
//! Values are always bound as query parameters to prevent SQL injection.

use async_trait::async_trait;
use serde_json::json;
use sqlx::{PgPool, Row};

use rara_kernel::tool::AgentTool;

/// Allowed tables and their queryable columns.
const TABLE_WHITELIST: &[(&str, &[&str])] = &[
    (
        "application",
        &[
            "id",
            "job_id",
            "resume_id",
            "channel",
            "status",
            "priority",
            "notes",
            "submitted_at",
            "created_at",
            "updated_at",
        ],
    ),
    (
        "resume",
        &[
            "id",
            "title",
            "version_tag",
            "source",
            "target_job_id",
            "parent_resume_id",
            "created_at",
            "updated_at",
        ],
    ),
    (
        "interview_plan",
        &[
            "id",
            "application_id",
            "title",
            "company",
            "position",
            "round",
            "scheduled_at",
            "task_status",
            "created_at",
            "updated_at",
        ],
    ),
];

/// Layer 1 primitive: parameterized SELECT queries against whitelisted tables.
pub struct DbQueryTool {
    pool: PgPool,
}

impl DbQueryTool {
    pub fn new(pool: PgPool) -> Self { Self { pool } }
}

/// Validate that `table` is in the whitelist and return its allowed columns.
fn allowed_columns(table: &str) -> Option<&'static [&'static str]> {
    TABLE_WHITELIST
        .iter()
        .find(|(t, _)| *t == table)
        .map(|(_, cols)| *cols)
}

#[async_trait]
impl AgentTool for DbQueryTool {
    fn name(&self) -> &str { "db_query" }

    fn description(&self) -> &str {
        "Query database tables (read-only). Allowed tables: application, resume, interview_plan. \
         Use filters to narrow results. Returns a JSON array of matching rows."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "table": {
                    "type": "string",
                    "description": "Table to query: application, resume, interview_plan"
                },
                "filters": {
                    "type": "object",
                    "description": "Column-value equality filters, e.g. {\"status\": 0, \"company\": \"Acme\"}"
                },
                "limit": {
                    "type": "integer",
                    "description": "Max rows to return (default 20, max 100)"
                }
            },
            "required": ["table"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let table = params
            .get("table")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: table"))?;

        let columns =
            allowed_columns(table).ok_or_else(|| anyhow::anyhow!("table not allowed: {table}"))?;

        let limit = params
            .get("limit")
            .and_then(|v| v.as_i64())
            .unwrap_or(20)
            .clamp(1, 100);

        let filters = params
            .get("filters")
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default();

        // Validate filter column names against the whitelist.
        for col in filters.keys() {
            if !columns.contains(&col.as_str()) {
                return Ok(json!({
                    "error": format!("column not allowed for table {table}: {col}")
                }));
            }
        }

        // Build parameterized query.
        // Table and column names come from the whitelist, so they are safe to
        // interpolate. Values are always bound via $N parameters.
        let mut sql = format!("SELECT * FROM {table} WHERE 1=1");
        let mut bind_values: Vec<String> = Vec::new();

        for (idx, (col, val)) in filters.iter().enumerate() {
            sql.push_str(&format!(" AND {} = ${}", col, idx + 1));
            // Coerce the JSON value to a string representation for binding.
            let s = match val {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            bind_values.push(s);
        }

        sql.push_str(&format!(" ORDER BY created_at DESC LIMIT {limit}"));

        let mut query = sqlx::query(&sql);
        for val in &bind_values {
            query = query.bind(val);
        }

        match query.fetch_all(&self.pool).await {
            Ok(rows) => {
                let result: Vec<serde_json::Value> = rows
                    .iter()
                    .map(|row| {
                        let mut obj = serde_json::Map::new();
                        for col in columns {
                            // Try common types in order of likelihood.
                            if let Ok(v) = row.try_get::<uuid::Uuid, _>(*col) {
                                obj.insert(col.to_string(), json!(v.to_string()));
                            } else if let Ok(v) = row.try_get::<String, _>(*col) {
                                obj.insert(col.to_string(), json!(v));
                            } else if let Ok(v) = row.try_get::<i16, _>(*col) {
                                obj.insert(col.to_string(), json!(v));
                            } else if let Ok(v) = row.try_get::<i32, _>(*col) {
                                obj.insert(col.to_string(), json!(v));
                            } else if let Ok(v) = row.try_get::<f32, _>(*col) {
                                obj.insert(col.to_string(), json!(v));
                            } else if let Ok(v) =
                                row.try_get::<chrono::DateTime<chrono::Utc>, _>(*col)
                            {
                                obj.insert(col.to_string(), json!(v.to_rfc3339()));
                            } else if let Ok(v) = row.try_get::<Option<String>, _>(*col) {
                                obj.insert(col.to_string(), json!(v));
                            } else if let Ok(v) =
                                row.try_get::<Option<chrono::DateTime<chrono::Utc>>, _>(*col)
                            {
                                obj.insert(col.to_string(), json!(v.map(|dt| dt.to_rfc3339())));
                            } else if let Ok(v) = row.try_get::<Option<f32>, _>(*col) {
                                obj.insert(col.to_string(), json!(v));
                            } else if let Ok(v) = row.try_get::<serde_json::Value, _>(*col) {
                                obj.insert(col.to_string(), v);
                            }
                            // Skip columns that cannot be decoded.
                        }
                        serde_json::Value::Object(obj)
                    })
                    .collect();
                Ok(json!(result))
            }
            Err(e) => Ok(json!({ "error": format!("{e}") })),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whitelist_rejects_unknown_table() {
        assert!(allowed_columns("users").is_none());
        assert!(allowed_columns("application").is_some());
    }

    #[test]
    fn whitelist_rejects_unknown_column() {
        let cols = allowed_columns("application").unwrap();
        assert!(cols.contains(&"status"));
        assert!(!cols.contains(&"password"));
    }

    #[test]
    fn all_tables_have_columns() {
        for (table, cols) in TABLE_WHITELIST {
            assert!(!table.is_empty());
            assert!(!cols.is_empty(), "table {table} has no columns");
        }
    }
}
