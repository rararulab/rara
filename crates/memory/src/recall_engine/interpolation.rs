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

//! Simple variable substitution for recall query templates.
//!
//! Supported variables:
//! - `{user_text}` -- the current user message text
//! - `{summary}` -- the compaction summary (empty string if None)
//! - `{session_topic}` -- the session topic (empty string if None)

use super::types::RecallContext;

/// Replace template variables in `template` with values from `ctx`.
pub fn interpolate(template: &str, ctx: &RecallContext) -> String {
    template
        .replace("{user_text}", &ctx.user_text)
        .replace("{summary}", ctx.summary.as_deref().unwrap_or(""))
        .replace(
            "{session_topic}",
            ctx.session_topic.as_deref().unwrap_or(""),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_interpolation_all_variables() {
        let ctx = RecallContext {
            user_text: "hello world".to_owned(),
            turn_count: 1,
            events: vec![],
            elapsed_since_last_secs: 0,
            summary: Some("conversation about Rust".to_owned()),
            session_topic: Some("programming".to_owned()),
        };

        let result = interpolate(
            "search for {user_text} about {session_topic}, context: {summary}",
            &ctx,
        );
        assert_eq!(
            result,
            "search for hello world about programming, context: conversation about Rust"
        );
    }

    #[test]
    fn test_interpolation_none_values() {
        let ctx = RecallContext {
            user_text: "query text".to_owned(),
            turn_count: 1,
            events: vec![],
            elapsed_since_last_secs: 0,
            summary: None,
            session_topic: None,
        };

        let result = interpolate("{user_text} | {summary} | {session_topic}", &ctx);
        assert_eq!(result, "query text |  | ");
    }

    #[test]
    fn test_interpolation_no_variables() {
        let ctx = RecallContext {
            user_text: "ignored".to_owned(),
            turn_count: 1,
            events: vec![],
            elapsed_since_last_secs: 0,
            summary: None,
            session_topic: None,
        };

        let result = interpolate("static query", &ctx);
        assert_eq!(result, "static query");
    }
}
