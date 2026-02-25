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

//! REST client for [Memos](https://www.usememos.com/) — a lightweight,
//! self-hosted Markdown note service.
//!
//! Memos is the **storage layer** of the memory system. It provides:
//!
//! - **Human-readable Markdown notes** — agents can write meeting notes,
//!   summaries, and daily exchange logs that are easily browsable in the
//!   Memos web UI.
//! - **Tag-based organisation** — notes can be tagged with `#hashtag` syntax.
//! - **Filter queries** — the API supports Google AIP-160 filter syntax for
//!   listing memos (e.g. `filter=tag == 'daily-log'`).
//!
//! ## Authentication
//!
//! All requests use Bearer token authentication. Create an API token in the
//! Memos web UI under Settings → Tokens.
//!
//! ## API Reference
//!
//! | Method        | HTTP                          | Purpose                       |
//! |---------------|-------------------------------|-------------------------------|
//! | `create_memo` | `POST /api/v1/memos`          | Create a new Markdown memo    |
//! | `list_memos`  | `GET /api/v1/memos`           | List memos with filter/paging |
//! | `get_memo`    | `GET /api/v1/memos/{id}`      | Retrieve a memo by ID         |
//! | `update_memo` | `PATCH /api/v1/memos/{id}`    | Update a memo's content       |
//! | `delete_memo` | `DELETE /api/v1/memos/{id}`   | Delete a memo                 |
//!
//! ## Deployment
//!
//! Memos is deployed as `neosmemo/memos:stable` with a dedicated PostgreSQL
//! instance (not shared with the main rara database).

use serde::{Deserialize, Serialize};

use crate::error::{HttpSnafu, MemosSnafu, MemoryResult};
use snafu::ResultExt;

/// Client for the Memos v1 REST API.
///
/// Uses the gRPC-gateway REST interface exposed by the Memos server.
/// All requests include a `Bearer {token}` authorization header.
pub struct MemosClient {
    /// Shared HTTP client (connection pooling, keep-alive).
    client: reqwest::Client,
    /// Base URL without trailing slash, e.g. `http://localhost:5230`.
    base_url: String,
    /// Bearer token for authentication (created in Memos Settings → Tokens).
    token: String,
}

impl MemosClient {
    /// Create a new Memos client.
    ///
    /// - `base_url`: scheme + host, e.g. `http://localhost:5230`.
    /// - `token`: Bearer token for authentication.
    pub fn new(base_url: String, token: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.trim_end_matches('/').to_owned(),
            token,
        }
    }

    /// Create a new memo.
    ///
    /// `POST /api/v1/memos`
    pub async fn create_memo(
        &self,
        content: &str,
        visibility: &str,
    ) -> MemoryResult<MemoEntry> {
        let url = format!("{}/api/v1/memos", self.base_url);
        let body = serde_json::json!({
            "content": content,
            "visibility": visibility,
        });

        let resp = self
            .client
            .post(&url)
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .context(HttpSnafu)?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return MemosSnafu {
                message: format!("POST /api/v1/memos returned {status}: {text}"),
            }
            .fail();
        }

        let entry: MemoEntry = resp.json().await.context(HttpSnafu)?;
        Ok(entry)
    }

    /// List memos with optional filter.
    ///
    /// The `filter` parameter uses Google AIP-160 syntax, for example:
    /// - `tag == 'daily-log'` — memos with a specific tag
    /// - `visibilities == ['PRIVATE']` — only private memos
    ///
    /// `GET /api/v1/memos?pageSize=N&filter=...`
    pub async fn list_memos(
        &self,
        page_size: usize,
        filter: Option<&str>,
    ) -> MemoryResult<Vec<MemoEntry>> {
        let mut url = format!("{}/api/v1/memos?pageSize={page_size}", self.base_url);
        if let Some(f) = filter {
            url.push_str(&format!("&filter={}", urlencoded(f)));
        }

        let resp = self
            .client
            .get(&url)
            .bearer_auth(&self.token)
            .send()
            .await
            .context(HttpSnafu)?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return MemosSnafu {
                message: format!("GET /api/v1/memos returned {status}: {text}"),
            }
            .fail();
        }

        let wrapper: MemosListResponse = resp.json().await.context(HttpSnafu)?;
        Ok(wrapper.memos.unwrap_or_default())
    }

    /// Get a single memo by name.
    ///
    /// `GET /api/v1/memos/{id}`
    pub async fn get_memo(&self, id: &str) -> MemoryResult<MemoEntry> {
        let url = format!("{}/api/v1/memos/{id}", self.base_url);

        let resp = self
            .client
            .get(&url)
            .bearer_auth(&self.token)
            .send()
            .await
            .context(HttpSnafu)?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return MemosSnafu {
                message: format!("GET /api/v1/memos/{id} returned {status}: {text}"),
            }
            .fail();
        }

        let entry: MemoEntry = resp.json().await.context(HttpSnafu)?;
        Ok(entry)
    }

    /// Update a memo's content.
    ///
    /// `PATCH /api/v1/memos/{id}`
    pub async fn update_memo(
        &self,
        id: &str,
        content: &str,
    ) -> MemoryResult<MemoEntry> {
        let url = format!("{}/api/v1/memos/{id}", self.base_url);
        let body = serde_json::json!({
            "content": content,
        });

        let resp = self
            .client
            .patch(&url)
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .context(HttpSnafu)?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return MemosSnafu {
                message: format!("PATCH /api/v1/memos/{id} returned {status}: {text}"),
            }
            .fail();
        }

        let entry: MemoEntry = resp.json().await.context(HttpSnafu)?;
        Ok(entry)
    }

    /// Delete a memo.
    ///
    /// `DELETE /api/v1/memos/{id}`
    pub async fn delete_memo(&self, id: &str) -> MemoryResult<()> {
        let url = format!("{}/api/v1/memos/{id}", self.base_url);

        let resp = self
            .client
            .delete(&url)
            .bearer_auth(&self.token)
            .send()
            .await
            .context(HttpSnafu)?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return MemosSnafu {
                message: format!("DELETE /api/v1/memos/{id} returned {status}: {text}"),
            }
            .fail();
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// A memo entry from the Memos API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoEntry {
    /// Resource name, e.g. `"memos/123"`.
    pub name: String,
    /// Unique identifier.
    pub uid: String,
    /// Markdown content.
    pub content: String,
    /// Visibility level (e.g. `"PRIVATE"`, `"PUBLIC"`).
    pub visibility: String,
    /// Whether the memo is pinned.
    #[serde(default)]
    pub pinned: bool,
    /// Creation timestamp (RFC 3339).
    #[serde(rename = "createTime", default)]
    pub create_time: String,
    /// Last update timestamp (RFC 3339).
    #[serde(rename = "updateTime", default)]
    pub update_time: String,
}

/// Internal response wrapper for `GET /api/v1/memos`.
#[derive(Debug, Deserialize)]
struct MemosListResponse {
    memos: Option<Vec<MemoEntry>>,
}

/// Minimal percent-encoding for query parameter values.
///
/// Only encodes characters that are unsafe in URL query strings. This is
/// sufficient for AIP-160 filter expressions; for general-purpose encoding
/// consider using the `percent-encoding` crate instead.
fn urlencoded(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            ' ' => out.push_str("%20"),
            '&' => out.push_str("%26"),
            '=' => out.push_str("%3D"),
            '%' => out.push_str("%25"),
            '+' => out.push_str("%2B"),
            '#' => out.push_str("%23"),
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::MemoryError;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn sample_memo_entry() -> serde_json::Value {
        serde_json::json!({
            "name": "memos/1",
            "uid": "uid-123",
            "content": "test content",
            "visibility": "PRIVATE",
            "pinned": false,
            "createTime": "2025-01-01T00:00:00Z",
            "updateTime": "2025-01-01T00:00:00Z"
        })
    }

    #[tokio::test]
    async fn create_memo_success() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/api/v1/memos"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&sample_memo_entry()))
            .mount(&server)
            .await;

        let client = MemosClient::new(server.uri(), "test-token".into());
        let entry = client.create_memo("test content", "PRIVATE").await.unwrap();
        assert_eq!(entry.name, "memos/1");
        assert_eq!(entry.uid, "uid-123");
        assert_eq!(entry.content, "test content");
    }

    #[tokio::test]
    async fn create_memo_unauthorized() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/api/v1/memos"))
            .respond_with(ResponseTemplate::new(401).set_body_string("unauthorized"))
            .mount(&server)
            .await;

        let client = MemosClient::new(server.uri(), "bad-token".into());
        let err = client.create_memo("content", "PRIVATE").await.unwrap_err();
        assert!(matches!(err, MemoryError::Memos { .. }));
    }

    #[tokio::test]
    async fn create_memo_sends_bearer_token() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/api/v1/memos"))
            .and(header("Authorization", "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&sample_memo_entry()))
            .mount(&server)
            .await;

        let client = MemosClient::new(server.uri(), "test-token".into());
        let entry = client.create_memo("test content", "PRIVATE").await.unwrap();
        assert_eq!(entry.name, "memos/1");
    }

    #[tokio::test]
    async fn list_memos_success() {
        let server = MockServer::start().await;
        let response = serde_json::json!({
            "memos": [sample_memo_entry()]
        });

        Mock::given(method("GET"))
            .and(path("/api/v1/memos"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&response))
            .mount(&server)
            .await;

        let client = MemosClient::new(server.uri(), "test-token".into());
        let memos = client.list_memos(10, None).await.unwrap();
        assert_eq!(memos.len(), 1);
        assert_eq!(memos[0].name, "memos/1");
    }

    #[tokio::test]
    async fn list_memos_empty() {
        let server = MockServer::start().await;
        let response = serde_json::json!({
            "memos": null
        });

        Mock::given(method("GET"))
            .and(path("/api/v1/memos"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&response))
            .mount(&server)
            .await;

        let client = MemosClient::new(server.uri(), "test-token".into());
        let memos = client.list_memos(10, None).await.unwrap();
        assert!(memos.is_empty());
    }

    #[tokio::test]
    async fn list_memos_with_filter() {
        let server = MockServer::start().await;
        let response = serde_json::json!({
            "memos": [sample_memo_entry()]
        });

        // The filter "tag == 'daily'" should be URL-encoded in the query string.
        // wiremock path matcher ignores query params, so the request will match.
        Mock::given(method("GET"))
            .and(path("/api/v1/memos"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&response))
            .mount(&server)
            .await;

        let client = MemosClient::new(server.uri(), "test-token".into());
        let memos = client
            .list_memos(10, Some("tag == 'daily'"))
            .await
            .unwrap();
        assert_eq!(memos.len(), 1);
    }

    #[tokio::test]
    async fn list_memos_error() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/v1/memos"))
            .respond_with(ResponseTemplate::new(500).set_body_string("server error"))
            .mount(&server)
            .await;

        let client = MemosClient::new(server.uri(), "test-token".into());
        let err = client.list_memos(10, None).await.unwrap_err();
        assert!(matches!(err, MemoryError::Memos { .. }));
    }

    #[tokio::test]
    async fn get_memo_success() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/v1/memos/42"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&sample_memo_entry()))
            .mount(&server)
            .await;

        let client = MemosClient::new(server.uri(), "test-token".into());
        let entry = client.get_memo("42").await.unwrap();
        assert_eq!(entry.name, "memos/1");
    }

    #[tokio::test]
    async fn get_memo_not_found() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/v1/memos/999"))
            .respond_with(ResponseTemplate::new(404).set_body_string("not found"))
            .mount(&server)
            .await;

        let client = MemosClient::new(server.uri(), "test-token".into());
        let err = client.get_memo("999").await.unwrap_err();
        assert!(matches!(err, MemoryError::Memos { .. }));
    }

    #[tokio::test]
    async fn update_memo_success() {
        let server = MockServer::start().await;
        let mut updated = sample_memo_entry();
        updated["content"] = serde_json::json!("updated content");

        Mock::given(method("PATCH"))
            .and(path("/api/v1/memos/42"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&updated))
            .mount(&server)
            .await;

        let client = MemosClient::new(server.uri(), "test-token".into());
        let entry = client.update_memo("42", "updated content").await.unwrap();
        assert_eq!(entry.content, "updated content");
    }

    #[tokio::test]
    async fn update_memo_not_found() {
        let server = MockServer::start().await;

        Mock::given(method("PATCH"))
            .and(path("/api/v1/memos/999"))
            .respond_with(ResponseTemplate::new(404).set_body_string("not found"))
            .mount(&server)
            .await;

        let client = MemosClient::new(server.uri(), "test-token".into());
        let err = client.update_memo("999", "content").await.unwrap_err();
        assert!(matches!(err, MemoryError::Memos { .. }));
    }

    #[tokio::test]
    async fn delete_memo_success() {
        let server = MockServer::start().await;

        Mock::given(method("DELETE"))
            .and(path("/api/v1/memos/42"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let client = MemosClient::new(server.uri(), "test-token".into());
        client.delete_memo("42").await.unwrap();
    }

    #[tokio::test]
    async fn delete_memo_not_found() {
        let server = MockServer::start().await;

        Mock::given(method("DELETE"))
            .and(path("/api/v1/memos/999"))
            .respond_with(ResponseTemplate::new(404).set_body_string("not found"))
            .mount(&server)
            .await;

        let client = MemosClient::new(server.uri(), "test-token".into());
        let err = client.delete_memo("999").await.unwrap_err();
        assert!(matches!(err, MemoryError::Memos { .. }));
    }

    #[test]
    fn urlencoded_spaces() {
        assert_eq!(urlencoded("hello world"), "hello%20world");
    }

    #[test]
    fn urlencoded_special_chars() {
        assert_eq!(urlencoded("&"), "%26");
        assert_eq!(urlencoded("="), "%3D");
        assert_eq!(urlencoded("%"), "%25");
        assert_eq!(urlencoded("+"), "%2B");
        assert_eq!(urlencoded("#"), "%23");
    }

    #[test]
    fn urlencoded_passthrough() {
        assert_eq!(urlencoded("abcABC123"), "abcABC123");
    }
}
