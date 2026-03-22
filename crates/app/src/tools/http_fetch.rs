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

use async_trait::async_trait;
use rara_kernel::tool::{ToolContext, ToolExecute};
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

const MAX_BODY_BYTES: usize = 100 * 1024;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct HttpFetchParams {
    /// The URL to fetch.
    url:    String,
    /// HTTP method (default GET).
    method: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HttpFetchResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status:       Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body:         Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error:        Option<String>,
}

/// Layer 1 primitive: issue an HTTP request.
#[derive(ToolDef)]
#[tool(
    name = "http-fetch",
    description = "Fetch a URL via HTTP GET or POST; returns status, content type, and body \
                   (truncated to 100KB).",
    tier = "deferred"
)]
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
impl ToolExecute for HttpFetchTool {
    type Output = HttpFetchResult;
    type Params = HttpFetchParams;

    async fn run(
        &self,
        params: HttpFetchParams,
        _context: &ToolContext,
    ) -> anyhow::Result<HttpFetchResult> {
        let method = params.method.as_deref().unwrap_or("GET");
        let request = match method {
            "POST" => self.client.post(&params.url),
            _ => self.client.get(&params.url),
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
                Ok(HttpFetchResult {
                    status:       Some(status),
                    content_type: Some(content_type),
                    body:         Some(body),
                    error:        None,
                })
            }
            Err(e) => Ok(HttpFetchResult {
                status:       None,
                content_type: None,
                body:         None,
                error:        Some(format!("{e}")),
            }),
        }
    }
}
