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

//! Shared completion helper for task agents.
//!
//! Constructs a `CreateChatCompletionRequest` with system + user messages
//! and sends it through an [`LlmProvider`](agent_core::provider::LlmProvider).

use async_openai::types::chat::{
    ChatCompletionRequestSystemMessageArgs, ChatCompletionRequestUserMessageArgs,
    CreateChatCompletionRequestArgs,
};

use agent_core::provider::LlmProvider;

use crate::builtin::tasks::error::TaskAgentError;

/// Send a single system + user message completion request and return the
/// assistant's text response.
pub(crate) async fn run_completion(
    provider: &dyn LlmProvider,
    model: &str,
    system_prompt: &str,
    user_input: &str,
) -> Result<String, TaskAgentError> {
    let request = CreateChatCompletionRequestArgs::default()
        .model(model)
        .messages(vec![
            ChatCompletionRequestSystemMessageArgs::default()
                .content(system_prompt)
                .build()
                .expect("system message")
                .into(),
            ChatCompletionRequestUserMessageArgs::default()
                .content(user_input)
                .build()
                .expect("user message")
                .into(),
        ])
        .build()
        .expect("chat completion request");

    let response = provider
        .chat_completion(request)
        .await
        .map_err(|e| TaskAgentError::RequestFailed {
            message: e.to_string(),
        })?;

    let choice = response
        .choices
        .first()
        .ok_or(TaskAgentError::EmptyResponse)?;

    choice
        .message
        .content
        .clone()
        .ok_or(TaskAgentError::EmptyResponse)
}
