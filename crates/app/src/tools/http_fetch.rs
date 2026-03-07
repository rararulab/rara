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

//! HTTP fetch primitive.
//!
//! Issues an HTTP GET or POST request and returns the response status, content
//! type, and body (truncated to 100 KB).

use async_trait::async_trait;
use rara_kernel::tool::AgentTool;
use serde_json::json;

/// Maximum response body size in bytes (100 KB).
const MAX_BODY_BYTES: usize = 100 * 1024;

/// Layer 1 primitive: issue an HTTP request.
pub struct HttpFetchTool {
    client: reqwest::Client,
}

impl HttpFetchTool {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
        }
    }
}

#[async_trait]
impl AgentTool for HttpFetchTool {
    fn name(&self) -> &str { "http-fetch" }

    fn description(&self) -> &str {
        "Fetch a URL via HTTP GET or POST. Returns status code, content type, and body (truncated \
         to 100KB). Useful for checking job posting pages or external APIs."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The URL to fetch"
                },
                "method": {
                    "type": "string",
                    "enum": ["GET", "POST"],
                    "description": "HTTP method (default GET)"
                }
            },
            "required": ["url"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _context: &rara_kernel::tool::ToolContext,
    ) -> anyhow::Result<serde_json::Value> {
        let url = params
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: url"))?;

        let method = params
            .get("method")
            .and_then(|v| v.as_str())
            .unwrap_or("GET");

        let request = match method {
            "POST" => self.client.post(url),
            _ => self.client.get(url),
        };

        match request.send().await {
            Ok(resp) => {
                let status = resp.status().as_u16();
                let content_type = resp
                    .headers()
                    .get(reqwest::header::CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("unknown")
                    .to_owned();

                let body_bytes = resp.bytes().await.unwrap_or_default();
                let body = if body_bytes.len() > MAX_BODY_BYTES {
                    let truncated = String::from_utf8_lossy(&body_bytes[..MAX_BODY_BYTES]);
                    format!("{truncated}... [truncated]")
                } else {
                    String::from_utf8_lossy(&body_bytes).into_owned()
                };

                Ok(json!({
                    "status": status,
                    "content_type": content_type,
                    "body": body,
                }))
            }
            Err(e) => Ok(json!({
                "error": format!("{e}"),
            })),
        }
    }
}
