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

//! Local filesystem storage for WeChat iLink Bot credentials and state.
//!
//! Ported from
//! [wechat-agent-rs](https://github.com/rararulab/wechat-agent-rs).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use snafu::ResultExt;

use super::errors::{IoSnafu, JsonSnafu, Result};

/// Default WeChat iLink API base URL.
pub const DEFAULT_BASE_URL: &str = "https://ilinkai.weixin.qq.com";

/// Base URL for downloading encrypted media from the WeChat CDN.
pub const CDN_BASE_URL: &str = "https://novac2c.cdn.weixin.qq.com/c2c";

fn storage_root() -> PathBuf {
    dirs::home_dir()
        .expect("no home directory")
        .join(".openclaw")
        .join("openclaw-weixin")
}

/// Persisted authentication credentials for a single WeChat account.
#[derive(Debug, Serialize, Deserialize)]
pub struct AccountData {
    /// Bearer token used to authenticate API requests.
    pub token:    String,
    /// ISO-8601 timestamp of when this data was saved.
    #[serde(rename = "savedAt")]
    pub saved_at: String,
    /// API base URL for this account.
    #[serde(rename = "baseUrl")]
    pub base_url: String,
    /// The iLink user ID associated with this account.
    #[serde(rename = "userId")]
    pub user_id:  String,
}

/// Per-account configuration loaded from local storage.
#[derive(Debug, Serialize, Deserialize)]
pub struct AccountConfig {
    /// Optional routing tag sent as a header on every API request.
    #[serde(default)]
    pub route_tag: Option<String>,
}

/// Returns the list of saved account IDs from local storage.
pub fn get_account_ids() -> Result<Vec<String>> {
    let path = storage_root().join("accounts.json");
    if !path.exists() {
        return Ok(vec![]);
    }
    let data = std::fs::read_to_string(&path).context(IoSnafu)?;
    let ids: Vec<String> = serde_json::from_str(&data).context(JsonSnafu)?;
    Ok(ids)
}

/// Persists the given list of account IDs to local storage.
pub fn save_account_ids(ids: &[String]) -> Result<()> {
    let path = storage_root().join("accounts.json");
    std::fs::create_dir_all(path.parent().expect("parent dir")).context(IoSnafu)?;
    let json = serde_json::to_string_pretty(ids).context(JsonSnafu)?;
    std::fs::write(&path, json).context(IoSnafu)?;
    Ok(())
}

/// Loads the saved credentials for the given account.
pub fn get_account_data(account_id: &str) -> Result<AccountData> {
    let path = storage_root()
        .join("accounts")
        .join(format!("{account_id}.json"));
    let data = std::fs::read_to_string(&path).context(IoSnafu)?;
    serde_json::from_str(&data).context(JsonSnafu)
}

/// Saves credentials for the given account to local storage.
pub fn save_account_data(account_id: &str, data: &AccountData) -> Result<()> {
    let path = storage_root()
        .join("accounts")
        .join(format!("{account_id}.json"));
    std::fs::create_dir_all(path.parent().expect("parent dir")).context(IoSnafu)?;
    let json = serde_json::to_string_pretty(data).context(JsonSnafu)?;
    std::fs::write(&path, json).context(IoSnafu)?;
    Ok(())
}

/// Returns the saved long-poll continuation buffer for the given account.
pub fn get_updates_buf(account_id: &str) -> Option<String> {
    let path = storage_root()
        .join("get_updates_buf")
        .join(format!("{account_id}.txt"));
    std::fs::read_to_string(&path).ok()
}

/// Saves the long-poll continuation buffer for the given account.
pub fn save_updates_buf(account_id: &str, buf: &str) -> Result<()> {
    let path = storage_root()
        .join("get_updates_buf")
        .join(format!("{account_id}.txt"));
    std::fs::create_dir_all(path.parent().expect("parent dir")).context(IoSnafu)?;
    std::fs::write(&path, buf).context(IoSnafu)?;
    Ok(())
}

/// Loads the optional per-account configuration, returning `None` if absent.
pub fn get_account_config(account_id: &str) -> Option<AccountConfig> {
    let path = storage_root()
        .join("config")
        .join(format!("{account_id}.json"));
    let data = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&data).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_account_data_serde() {
        let data = AccountData {
            token:    "tok_abc".to_string(),
            saved_at: "2025-01-01T00:00:00Z".to_string(),
            base_url: "https://example.com".to_string(),
            user_id:  "user_123".to_string(),
        };
        let json = serde_json::to_string(&data).unwrap();
        let deserialized: AccountData = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.token, data.token);
        assert_eq!(deserialized.saved_at, data.saved_at);
        assert_eq!(deserialized.base_url, data.base_url);
        assert_eq!(deserialized.user_id, data.user_id);
    }

    #[test]
    fn test_account_config_serde() {
        let config = AccountConfig {
            route_tag: Some("tag_xyz".to_string()),
        };
        let json = serde_json::to_string(&config).unwrap();
        let deserialized: AccountConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.route_tag, Some("tag_xyz".to_string()));
    }

    #[test]
    fn test_account_config_default_route_tag() {
        let json = r"{}";
        let config: AccountConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.route_tag, None);
    }
}
