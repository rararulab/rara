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

//! REST client for [Memos](https://www.usememos.com/) — a lightweight,
//! self-hosted Markdown note service.
//!
//! Memos is the **storage layer** of the memory system. It provides:
//!
//! - **Human-readable Markdown notes** — agents can write meeting notes,
//!   summaries, and daily exchange logs that are easily browsable in the Memos
//!   web UI.
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
use snafu::ResultExt;

use crate::error::{HttpSnafu, MemoryResult, MemosSnafu};

/// Client for the Memos v1 REST API.
///
/// Uses the gRPC-gateway REST interface exposed by the Memos server.
/// All requests include a `Bearer {token}` authorization header.
pub struct MemosClient {
    /// Shared HTTP client (connection pooling, keep-alive).
    client:   reqwest::Client,
    /// Base URL without trailing slash, e.g. `http://localhost:5230`.
    base_url: String,
    /// Bearer token for authentication (created in Memos Settings → Tokens).
    token:    String,
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
    pub async fn create_memo(&self, content: &str, visibility: &str) -> MemoryResult<MemoEntry> {
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
    pub async fn update_memo(&self, id: &str, content: &str) -> MemoryResult<MemoEntry> {
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
    pub name:        String,
    /// Unique identifier.
    pub uid:         String,
    /// Markdown content.
    pub content:     String,
    /// Visibility level (e.g. `"PRIVATE"`, `"PUBLIC"`).
    pub visibility:  String,
    /// Whether the memo is pinned.
    #[serde(default)]
    pub pinned:      bool,
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

    fn client() -> Option<MemosClient> {
        let url = std::env::var("MEMOS_BASE_URL").ok()?;
        let token = std::env::var("MEMOS_TOKEN").unwrap_or_default();
        Some(MemosClient::new(url, token))
    }

    #[tokio::test]
    #[ignore = "requires running Memos service (set MEMOS_BASE_URL, MEMOS_TOKEN)"]
    async fn create_list_get_update_delete_memo() {
        let c = client().expect("MEMOS_BASE_URL required");

        // Create
        let entry = c
            .create_memo("integration test memo #rara-test", "PRIVATE")
            .await
            .expect("create_memo failed");
        println!("created memo: {} (uid={})", entry.name, entry.uid);
        assert!(entry.content.contains("integration test memo"));

        // Extract numeric id from name "memos/123"
        let id = entry.name.strip_prefix("memos/").unwrap_or(&entry.name);

        // List
        let memos = c.list_memos(10, None).await.expect("list_memos failed");
        println!("list_memos returned {} entries", memos.len());
        assert!(
            memos.iter().any(|m| m.uid == entry.uid),
            "created memo not found in list"
        );

        // Get
        let fetched = c.get_memo(id).await.expect("get_memo failed");
        assert_eq!(fetched.uid, entry.uid);

        // Update
        let updated = c
            .update_memo(id, "updated integration test memo #rara-test")
            .await
            .expect("update_memo failed");
        assert!(updated.content.contains("updated"));

        // Delete
        c.delete_memo(id).await.expect("delete_memo failed");
        println!("deleted memo {id}");
    }

    #[tokio::test]
    #[ignore = "requires running Memos service (set MEMOS_BASE_URL, MEMOS_TOKEN)"]
    async fn list_with_filter() {
        let c = client().expect("MEMOS_BASE_URL required");
        // This may return empty but should not error
        let memos = c
            .list_memos(5, Some("tag == 'rara-test'"))
            .await
            .expect("list_memos with filter failed");
        println!("filtered list returned {} entries", memos.len());
    }

    // Keep urlencoded unit tests — these don't need infrastructure
    #[test]
    fn urlencoded_spaces() {
        assert_eq!(urlencoded("hello world"), "hello%20world");
    }

    #[test]
    fn urlencoded_special_chars() {
        assert_eq!(urlencoded("a&b=c%d+e#f"), "a%26b%3Dc%25d%2Be%23f");
    }

    #[test]
    fn urlencoded_passthrough() {
        assert_eq!(urlencoded("hello"), "hello");
    }
}
