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

//! Follow-up email drafting agent.

use rig::{client::CompletionClient, completion::Prompt, providers::openrouter};

use crate::error::AiError;

const SYSTEM_PROMPT: &str = "\
You are a professional communicator. Draft a concise, polite follow-up email based on the context \
                             provided. The email should:
- Be professional but warm
- Reference specific details from the interaction
- Express continued interest
- Include a clear call to action";

/// Drafts follow-up emails after interviews or applications.
pub struct FollowUpDraftAgent<'a> {
    client: &'a openrouter::Client,
    model:  &'a str,
}

impl<'a> FollowUpDraftAgent<'a> {
    pub(crate) fn new(client: &'a openrouter::Client, model: &'a str) -> Self { Self { client, model } }

    /// Draft a follow-up email based on the given context.
    pub async fn draft(&self, context: &str) -> Result<String, AiError> {
        let agent = self
            .client
            .agent(self.model)
            .preamble(SYSTEM_PROMPT)
            .build();

        agent
            .prompt(context)
            .await
            .map_err(|e| AiError::RequestFailed {
                message: e.to_string(),
            })
    }
}
