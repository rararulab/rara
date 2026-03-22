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

//! HTTP client for the WeChat iLink Bot API.
//!
//! Ported from
//! [wechat-agent-rs](https://github.com/rararulab/wechat-agent-rs).

use std::time::Duration;

use reqwest::Client;
use serde_json::Value;
use snafu::ResultExt;

use super::errors::{ApiSnafu, HttpSnafu, Result, SessionExpiredSnafu};

const SESSION_EXPIRED_ERRCODE: i64 = -14;

/// HTTP client wrapper for the WeChat iLink Bot API.
///
/// Handles authentication headers, request signing, and automatic
/// session-expiry detection on every response.
pub struct WeixinApiClient {
    client:    Client,
    base_url:  String,
    token:     String,
    route_tag: Option<String>,
}

impl WeixinApiClient {
    /// Creates a new API client targeting `base_url` with the given bearer
    /// `token`.
    pub fn new(base_url: &str, token: &str, route_tag: Option<String>) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            token: token.to_string(),
            route_tag,
        }
    }

    /// Replaces the bearer token used for subsequent requests.
    pub fn set_token(&mut self, token: &str) { self.token = token.to_string(); }

    fn headers(&self) -> reqwest::header::HeaderMap {
        use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("authorizationtype"),
            HeaderValue::from_static("ilink_bot_token"),
        );
        headers.insert(
            reqwest::header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", self.token)).expect("valid bearer token"),
        );
        let uin: u64 = rand::random::<u64>() % 9_000_000_000 + 1_000_000_000;
        headers.insert(
            HeaderName::from_static("x-wechat-uin"),
            HeaderValue::from_str(&uin.to_string()).expect("valid uin header"),
        );
        if let Some(ref tag) = self.route_tag {
            headers.insert(
                HeaderName::from_static("skroutetag"),
                HeaderValue::from_str(tag).expect("valid route tag header"),
            );
        }
        headers
    }

    async fn post(&self, path: &str, body: &Value) -> Result<Value> {
        self.post_with_timeout(path, body, Duration::from_secs(30))
            .await
    }

    async fn post_with_timeout(
        &self,
        path: &str,
        body: &Value,
        timeout: Duration,
    ) -> Result<Value> {
        let url = format!("{}/{}", self.base_url, path);
        let resp = self
            .client
            .post(&url)
            .headers(self.headers())
            .json(body)
            .timeout(timeout)
            .send()
            .await
            .context(HttpSnafu)?
            .json::<Value>()
            .await
            .context(HttpSnafu)?;

        if let Some(code) = resp.get("errcode").and_then(serde_json::Value::as_i64) {
            if code == SESSION_EXPIRED_ERRCODE {
                return Err(SessionExpiredSnafu.build());
            }
            if code != 0 {
                let msg = resp
                    .get("errmsg")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown error")
                    .to_string();
                return Err(ApiSnafu { code, message: msg }.build());
            }
        }
        Ok(resp)
    }

    /// Requests a new login QR code from the API.
    pub async fn fetch_qr_code(&self) -> Result<Value> {
        self.post("ilink/bot/get_bot_qrcode", &serde_json::json!({}))
            .await
    }

    /// Polls the current scan status for the given `qrcode_id`.
    pub async fn get_qr_code_status(&self, qrcode_id: &str) -> Result<Value> {
        self.post(
            "ilink/bot/get_qrcode_status",
            &serde_json::json!({ "qrcode_id": qrcode_id }),
        )
        .await
    }

    /// Long-polls for new incoming messages, optionally resuming from `buf`.
    pub async fn get_updates(&self, buf: Option<&str>) -> Result<Value> {
        let mut body = serde_json::json!({});
        if let Some(b) = buf {
            body["get_updates_buf"] = Value::String(b.to_string());
        }
        self.post_with_timeout("ilink/bot/getupdates", &body, Duration::from_secs(40))
            .await
    }

    /// Sends a plain-text message to `to_user_id`.
    pub async fn send_text_message(
        &self,
        to_user_id: &str,
        context_token: &str,
        text: &str,
    ) -> Result<Value> {
        let body = serde_json::json!({
            "to_user_id": to_user_id,
            "context_token": context_token,
            "item_list": [{
                "type": 0,
                "body": text
            }]
        });
        self.post("ilink/bot/sendmessage", &body).await
    }

    /// Sends a media message (image, video, or file) to `to_user_id`.
    pub async fn send_media_message(
        &self,
        to_user_id: &str,
        context_token: &str,
        text: Option<&str>,
        file_info: &Value,
    ) -> Result<Value> {
        let mut item_list = vec![];
        if let Some(t) = text {
            item_list.push(serde_json::json!({ "type": 0, "body": t }));
        }
        item_list.push(file_info.clone());
        let body = serde_json::json!({
            "to_user_id": to_user_id,
            "context_token": context_token,
            "item_list": item_list
        });
        self.post("ilink/bot/sendmessage", &body).await
    }

    /// Sends a typing indicator to `to_user_id`.
    pub async fn send_typing(&self, to_user_id: &str, context_token: &str) -> Result<Value> {
        let body = serde_json::json!({
            "to_user_id": to_user_id,
            "context_token": context_token,
        });
        self.post("ilink/bot/sendtyping", &body).await
    }

    /// Requests a pre-signed upload URL for a file of the given name and size.
    pub async fn get_upload_url(&self, file_name: &str, file_size: u64) -> Result<Value> {
        let body = serde_json::json!({
            "file_name": file_name,
            "file_size": file_size,
        });
        self.post("ilink/bot/getuploadurl", &body).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_new() {
        let client = WeixinApiClient::new("https://example.com/", "tok_123", None);
        assert_eq!(client.base_url, "https://example.com");
        assert_eq!(client.token, "tok_123");
        assert!(client.route_tag.is_none());
    }

    #[test]
    fn test_client_set_token() {
        let mut client = WeixinApiClient::new("https://example.com", "old_token", None);
        assert_eq!(client.token, "old_token");
        client.set_token("new_token");
        assert_eq!(client.token, "new_token");
    }
}
