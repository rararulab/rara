use serde_json::Value;
use snafu::ResultExt;
use tracing::debug;

use crate::error::{RalphRequestSnafu, RalphSnafu, Result};

/// Task record returned by ralph API.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskRecord {
    pub id: String,
    pub title: String,
    pub status: String,
    pub priority: u8,
    #[serde(default)]
    pub blocked_by: Option<String>,
    #[serde(default)]
    pub archived_at: Option<String>,
    #[serde(default)]
    pub error_message: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub completed_at: Option<String>,
}

/// HTTP client for ralph's JSON-RPC API.
#[derive(Debug, Clone)]
pub struct RalphClient {
    http: reqwest::Client,
    endpoint: String,
}

impl RalphClient {
    /// Create a new client pointing at the given ralph API base URL.
    ///
    /// The endpoint should be the base URL (e.g. `http://127.0.0.1:3000`).
    /// `/rpc/v1` is appended automatically.
    #[must_use]
    pub fn new(base_url: &str) -> Self {
        Self {
            http: reqwest::Client::new(),
            endpoint: format!("{}/rpc/v1", base_url.trim_end_matches('/')),
        }
    }

    /// Create a task from an issue. If `auto_execute` is true, ralph will
    /// immediately queue the task for execution.
    pub async fn task_create(
        &self,
        id: &str,
        title: &str,
        priority: u8,
        auto_execute: bool,
    ) -> Result<TaskRecord> {
        let result = self
            .call(
                "task.create",
                serde_json::json!({
                    "id": id,
                    "title": title,
                    "priority": priority,
                    "autoExecute": auto_execute,
                }),
            )
            .await?;

        serde_json::from_value(result).map_err(|e| {
            RalphSnafu {
                message: format!("failed to parse task.create response: {e}"),
            }
            .build()
        })
    }

    /// List tasks, optionally filtered by status.
    pub async fn task_list(&self, status: Option<&str>) -> Result<Vec<TaskRecord>> {
        let mut params = serde_json::json!({});
        if let Some(s) = status {
            params["status"] = Value::String(s.to_owned());
        }

        let result = self.call("task.list", params).await?;

        serde_json::from_value(result).map_err(|e| {
            RalphSnafu {
                message: format!("failed to parse task.list response: {e}"),
            }
            .build()
        })
    }

    /// Get a single task by ID.
    pub async fn task_get(&self, id: &str) -> Result<TaskRecord> {
        let result = self
            .call("task.get", serde_json::json!({ "id": id }))
            .await?;

        serde_json::from_value(result).map_err(|e| {
            RalphSnafu {
                message: format!("failed to parse task.get response: {e}"),
            }
            .build()
        })
    }

    /// Cancel a running or pending task.
    pub async fn task_cancel(&self, id: &str) -> Result<TaskRecord> {
        let result = self
            .call("task.cancel", serde_json::json!({ "id": id }))
            .await?;

        serde_json::from_value(result).map_err(|e| {
            RalphSnafu {
                message: format!("failed to parse task.cancel response: {e}"),
            }
            .build()
        })
    }

    /// Health check — returns true if ralph API is responsive.
    pub async fn health(&self) -> bool {
        let url = self.endpoint.replace("/rpc/v1", "/health");
        self.http
            .get(&url)
            .send()
            .await
            .is_ok_and(|r| r.status().is_success())
    }

    /// Build a JSON-RPC request envelope.
    #[must_use]
    pub fn build_request(&self, method: &str, params: Value) -> Value {
        serde_json::json!({
            "apiVersion": "v1",
            "id": uuid::Uuid::new_v4().to_string(),
            "method": method,
            "params": params,
        })
    }

    /// Send an RPC call and return the `result` field on success.
    async fn call(&self, method: &str, params: Value) -> Result<Value> {
        let body = self.build_request(method, params);

        debug!(method, "ralph RPC call");

        let response = self
            .http
            .post(&self.endpoint)
            .json(&body)
            .send()
            .await
            .context(RalphRequestSnafu)?;

        let response_body: Value = response.json().await.context(RalphRequestSnafu)?;

        parse_rpc_response(&response_body)
    }
}

/// Extract the `result` from a ralph RPC response, or return an error.
fn parse_rpc_response(response: &Value) -> Result<Value> {
    if let Some(result) = response.get("result") {
        return Ok(result.clone());
    }

    if let Some(error) = response.get("error") {
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("unknown ralph error");
        let code = error
            .get("code")
            .and_then(Value::as_str)
            .unwrap_or("UNKNOWN");
        return Err(RalphSnafu {
            message: format!("[{code}] {message}"),
        }
        .build());
    }

    Err(RalphSnafu {
        message: "ralph response missing both 'result' and 'error' fields".to_owned(),
    }
    .build())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_rpc_request_creates_valid_envelope() {
        let client = RalphClient::new("http://127.0.0.1:3000");
        let body = client.build_request(
            "task.create",
            serde_json::json!({
                "id": "RAR-42",
                "title": "Add widget support",
                "autoExecute": true,
            }),
        );

        assert_eq!(body["apiVersion"], "v1");
        assert_eq!(body["method"], "task.create");
        assert_eq!(body["params"]["id"], "RAR-42");
        assert!(body["id"].as_str().is_some_and(|id| !id.is_empty()));
    }

    #[test]
    fn parse_rpc_success_extracts_result() {
        let response = serde_json::json!({
            "apiVersion": "v1",
            "id": "test-id",
            "method": "task.create",
            "result": {
                "id": "RAR-42",
                "title": "Add widget support",
                "status": "open",
                "priority": 2,
                "createdAt": "2026-03-08T00:00:00Z",
                "updatedAt": "2026-03-08T00:00:00Z",
            },
            "meta": { "servedBy": "ralph-api", "servedAt": "2026-03-08T00:00:00Z" }
        });

        let result = parse_rpc_response(&response).unwrap();
        assert_eq!(result["id"], "RAR-42");
    }

    #[test]
    fn parse_rpc_error_returns_ralph_error() {
        let response = serde_json::json!({
            "apiVersion": "v1",
            "id": "test-id",
            "error": {
                "code": "CONFLICT",
                "message": "Task already exists",
            },
            "meta": { "servedBy": "ralph-api", "servedAt": "2026-03-08T00:00:00Z" }
        });

        let err = parse_rpc_response(&response).unwrap_err();
        assert!(err.to_string().contains("Task already exists"));
    }
}
