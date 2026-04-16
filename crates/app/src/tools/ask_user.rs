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

//! Ask-user tool — blocks the agent until the user responds.
//!
//! Uses [`rara_kernel::user_question::UserQuestionManager`] to submit a
//! question and wait for the user's answer via the same oneshot-channel pattern
//! as `ApprovalManager`.

use std::time::Duration;

use async_trait::async_trait;
use rara_kernel::{
    tool::{ToolContext, ToolExecute},
    user_question::UserQuestionManagerRef,
};
use rara_tool_macro::ToolDef;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

/// Default timeout for user questions (5 minutes).
const DEFAULT_TIMEOUT_SECS: u64 = 300;

/// Parameters for the ask-user tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct AskUserParams {
    /// The question to ask the user. Be specific about what information you
    /// need and why.
    question:  String,
    /// Set to `true` when the answer is sensitive (API keys, passwords,
    /// tokens, 2FA codes, credentials, personal identifiers). Sensitive
    /// prompts are forcibly routed to the user's private chat so the text
    /// never appears in a shared group/topic, and a short notice is posted
    /// in the originating chat instead. Default: `false`.
    #[serde(default)]
    sensitive: bool,
    /// Pre-defined answer choices. When provided, channel adapters render
    /// structured controls (e.g. inline keyboard buttons) and the user's
    /// answer is guaranteed to be one of these strings. Use for
    /// yes/no/enumerated questions to avoid free-text parsing. When `None`,
    /// the user replies with free-form text.
    #[serde(default)]
    options:   Option<Vec<String>>,
}

/// Ask the user a question and wait for their response.
#[derive(ToolDef)]
#[tool(
    name = "ask-user",
    description = "Ask the user a question and wait for their response. Use when you need \
                   information that only the user can provide (e.g. API keys, preferences, \
                   clarifications). The agent will pause until the user responds or the request \
                   times out. Set `sensitive: true` when requesting secrets (keys, passwords, \
                   tokens, 2FA codes) so the prompt is forced to a private channel. Provide \
                   `options` for enumerated answers — the answer is guaranteed to be one of the \
                   supplied strings, avoiding free-text ambiguity.",
    tier = "deferred",
    user_interaction
)]
pub struct AskUserTool {
    manager: UserQuestionManagerRef,
}

impl AskUserTool {
    /// Create a new ask-user tool backed by the given question manager.
    pub fn new(manager: UserQuestionManagerRef) -> Self { Self { manager } }
}

#[async_trait]
impl ToolExecute for AskUserTool {
    type Output = Value;
    type Params = AskUserParams;

    #[tracing::instrument(skip_all)]
    async fn run(&self, params: AskUserParams, context: &ToolContext) -> anyhow::Result<Value> {
        let timeout = Duration::from_secs(DEFAULT_TIMEOUT_SECS);
        // Propagate the originating endpoint so channel adapters can route the
        // question back to the same conversation surface (e.g. a Telegram
        // forum topic) instead of a default fallback like `primary_chat_id`.
        let endpoint = context.origin_endpoint.clone();
        // Propagate the platform-native user identifier so channel adapters
        // can bind the pending question to the specific user who triggered
        // the turn — other members of a shared chat must not be able to
        // answer on their behalf.
        let expected_platform_user_id = context.origin_platform_user_id.clone();
        // Either the caller explicitly marked the prompt sensitive, or the
        // question text matches common secret-request patterns. Heuristic
        // detection is a defense-in-depth safety net, not a substitute for
        // the explicit flag.
        let sensitive = params.sensitive || looks_sensitive(&params.question);
        let answer = self
            .manager
            .ask(
                params.question,
                endpoint,
                expected_platform_user_id,
                sensitive,
                params.options,
                timeout,
            )
            .await?;
        Ok(serde_json::json!({ "answer": answer }))
    }
}

/// Heuristic detector for prompts that solicit sensitive material.
///
/// Pattern-matches common secret-request phrasings in English and Chinese.
/// This is a best-effort safety net that complements — not replaces — the
/// explicit `sensitive` flag that the caller can set. False positives are
/// preferred over false negatives: the consequence of a false positive is
/// that a mundane prompt gets routed to DM instead of the topic; the
/// consequence of a false negative is a secret leaked to a shared chat.
fn looks_sensitive(question: &str) -> bool {
    /// Case-insensitive substring markers that strongly indicate a secret
    /// is being requested. Keep this list conservative — broad words like
    /// "auth" on their own trigger too often; pair them with a verb/noun
    /// that implies "please provide X".
    const NEEDLES: &[&str] = &[
        "api key",
        "api-key",
        "apikey",
        "access token",
        "bearer token",
        "refresh token",
        "auth token",
        "password",
        "passphrase",
        "secret key",
        "private key",
        "2fa",
        "one-time code",
        "one time code",
        "otp",
        "verification code",
        "credentials",
        "credential",
        // Chinese
        "密码",
        "密钥",
        "口令",
        "验证码",
        "令牌",
        "私钥",
    ];
    let lower = question.to_lowercase();
    NEEDLES.iter().any(|needle| lower.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::looks_sensitive;

    #[test]
    fn detects_common_secret_phrases() {
        assert!(looks_sensitive("Please paste your API key here"));
        assert!(looks_sensitive("What is the OTP code?"));
        assert!(looks_sensitive("Enter your password"));
        assert!(looks_sensitive("请输入密码"));
        assert!(looks_sensitive("2FA code?"));
        assert!(looks_sensitive("share your refresh token"));
    }

    #[test]
    fn non_sensitive_prompts_pass_through() {
        assert!(!looks_sensitive("Do you want A or B?"));
        assert!(!looks_sensitive("Should I continue?"));
        assert!(!looks_sensitive("What is your favorite color?"));
    }
}
