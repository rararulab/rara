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

        // Check both "errcode" (getupdates) and "ret" (sendmessage, sendtyping)
        // error fields — different iLink endpoints use different conventions.
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
        if let Some(ret) = resp.get("ret").and_then(serde_json::Value::as_i64) {
            if ret != 0 {
                let msg = resp
                    .get("errmsg")
                    .or_else(|| resp.get("err_msg"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown error")
                    .to_string();
                return Err(ApiSnafu {
                    code:    ret,
                    message: msg,
                }
                .build());
            }
        }
        Ok(resp)
    }

    /// Sends a form-encoded POST request and checks the `ret` error field.
    ///
    /// Login endpoints use form encoding + `ret`/`err_msg` instead of JSON +
    /// `errcode`/`errmsg` used by messaging endpoints.
    async fn post_form(&self, path: &str, params: &[(&str, &str)]) -> Result<Value> {
        self.post_form_with_timeout(path, params, Duration::from_secs(30))
            .await
    }

    /// Same as [`post_form`](Self::post_form) but with a custom timeout.
    async fn post_form_with_timeout(
        &self,
        path: &str,
        params: &[(&str, &str)],
        timeout: Duration,
    ) -> Result<Value> {
        let url = format!("{}/{}", self.base_url, path);
        let resp = self
            .client
            .post(&url)
            .headers(self.headers())
            .form(params)
            .timeout(timeout)
            .send()
            .await
            .context(HttpSnafu)?
            .json::<Value>()
            .await
            .context(HttpSnafu)?;

        if let Some(ret) = resp.get("ret").and_then(Value::as_i64) {
            if ret != 0 {
                let msg = resp
                    .get("err_msg")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown error")
                    .to_string();
                return Err(ApiSnafu {
                    code:    ret,
                    message: msg,
                }
                .build());
            }
        }
        Ok(resp)
    }

    /// Requests a new login QR code from the API.
    pub async fn fetch_qr_code(&self) -> Result<Value> {
        self.post_form("ilink/bot/get_bot_qrcode", &[("bot_type", "3")])
            .await
    }

    /// Polls the current scan status for the given `qrcode_id`.
    ///
    /// Uses a longer timeout than the default because this endpoint
    /// long-polls until the user scans the QR code.
    pub async fn get_qr_code_status(&self, qrcode_id: &str) -> Result<Value> {
        self.post_form_with_timeout(
            "ilink/bot/get_qrcode_status",
            &[("qrcode", qrcode_id), ("bot_type", "3")],
            Duration::from_secs(60),
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
    ///
    /// The request body follows the iLink protocol: the message is wrapped
    /// in a `msg` object with `from_user_id` (bot), `to_user_id` (recipient),
    /// `message_type: 2` (bot message), and `message_state: 2` (finished).
    pub async fn send_text_message(
        &self,
        from_user_id: &str,
        to_user_id: &str,
        context_token: &str,
        text: &str,
    ) -> Result<Value> {
        let body = serde_json::json!({
            "msg": {
                "from_user_id": from_user_id,
                "to_user_id": to_user_id,
                "client_id": uuid::Uuid::new_v4().to_string(),
                "message_type": 2,
                "message_state": 2,
                "item_list": [{
                    "type": 1,
                    "text_item": { "text": text }
                }],
                "context_token": context_token
            }
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

    /// Fetches a `typing_ticket` for the given user via the iLink `getconfig`
    /// endpoint. The ticket is required by `send_typing`.
    pub async fn get_config(&self, ilink_user_id: &str, context_token: &str) -> Result<Value> {
        let body = serde_json::json!({
            "ilink_user_id": ilink_user_id,
            "context_token": context_token,
            "base_info": {}
        });
        self.post("ilink/bot/getconfig", &body).await
    }

    /// Sends a typing indicator for the given user.
    ///
    /// Requires a `typing_ticket` obtained from
    /// [`get_config`](Self::get_config). `status` should be `1` (typing) or
    /// `2` (cancel).
    pub async fn send_typing(
        &self,
        ilink_user_id: &str,
        typing_ticket: &str,
        status: u8,
    ) -> Result<Value> {
        let body = serde_json::json!({
            "ilink_user_id": ilink_user_id,
            "typing_ticket": typing_ticket,
            "status": status,
            "base_info": {}
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
