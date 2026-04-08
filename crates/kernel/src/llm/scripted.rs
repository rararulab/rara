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

//! Test-only LLM driver that returns pre-recorded responses in order.
//!
//! [`ScriptedLlmDriver`] implements [`LlmDriver`] by dequeuing scripted
//! [`CompletionResponse`] values. This enables deterministic, CI-ready E2E
//! tests without real LLM API keys or network access.
//!
//! # Example
//!
//! ```ignore
//! use rara_kernel::llm::{ScriptedLlmDriver, CompletionResponse, StopReason};
//!
//! let driver = ScriptedLlmDriver::new(vec![
//!     CompletionResponse {
//!         content: Some("Hello!".into()),
//!         reasoning_content: None,
//!         tool_calls: vec![],
//!         stop_reason: StopReason::Stop,
//!         usage: None,
//!         model: "scripted".into(),
//!     },
//! ]);
//! ```

use std::{collections::VecDeque, sync::Mutex};

use async_trait::async_trait;
use tokio::sync::mpsc;

use super::{
    driver::LlmDriver,
    stream::StreamDelta,
    types::{CompletionRequest, CompletionResponse},
};
use crate::error::{KernelError, Result};

/// A test-only LLM driver that returns pre-recorded responses in order.
///
/// Each call to [`complete`](LlmDriver::complete) or
/// [`stream`](LlmDriver::stream) pops the next response from the front of
/// the queue. When exhausted, returns a `KernelError::Provider` error so
/// the agent loop terminates gracefully rather than panicking.
///
/// All incoming [`CompletionRequest`]s are captured for post-hoc
/// assertions.
pub struct ScriptedLlmDriver {
    responses: Mutex<VecDeque<CompletionResponse>>,
    /// All requests received, in order.
    captured:  Mutex<Vec<CompletionRequest>>,
}

impl ScriptedLlmDriver {
    /// Create a driver pre-loaded with the given response sequence.
    pub fn new(responses: Vec<CompletionResponse>) -> Self {
        Self {
            responses: Mutex::new(VecDeque::from(responses)),
            captured:  Mutex::new(Vec::new()),
        }
    }

    /// Return a snapshot of all captured requests (for test assertions).
    pub fn captured_requests(&self) -> Vec<CompletionRequest> {
        self.captured
            .lock()
            .expect("ScriptedLlmDriver: captured lock poisoned")
            .clone()
    }

    /// How many scripted responses remain unconsumed.
    pub fn remaining(&self) -> usize {
        self.responses
            .lock()
            .expect("ScriptedLlmDriver: responses lock poisoned")
            .len()
    }
}

#[async_trait]
impl LlmDriver for ScriptedLlmDriver {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse> {
        self.captured
            .lock()
            .expect("ScriptedLlmDriver: captured lock poisoned")
            .push(request);

        let response = self
            .responses
            .lock()
            .expect("ScriptedLlmDriver: responses lock poisoned")
            .pop_front()
            .ok_or_else(|| KernelError::Provider {
                message: "ScriptedLlmDriver: no more scripted responses".into(),
            })?;
        Ok(response)
    }

    async fn stream(
        &self,
        request: CompletionRequest,
        tx: mpsc::Sender<StreamDelta>,
    ) -> Result<CompletionResponse> {
        // Emit the full text as a single delta, then Done.
        let response = self.complete(request).await?;
        if let Some(ref text) = response.content {
            let _ = tx.send(StreamDelta::TextDelta { text: text.clone() }).await;
        }
        let _ = tx
            .send(StreamDelta::Done {
                stop_reason: response.stop_reason,
                usage:       response.usage,
            })
            .await;
        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::types::StopReason;

    fn scripted_response(text: &str) -> CompletionResponse {
        CompletionResponse {
            content:           Some(text.to_string()),
            reasoning_content: None,
            tool_calls:        vec![],
            stop_reason:       StopReason::Stop,
            usage:             None,
            model:             "scripted".to_string(),
        }
    }

    #[tokio::test]
    async fn complete_returns_scripted_responses_in_order() {
        let driver = ScriptedLlmDriver::new(vec![
            scripted_response("first"),
            scripted_response("second"),
        ]);

        let request = CompletionRequest {
            model:               "test".into(),
            messages:            vec![],
            tools:               vec![],
            temperature:         None,
            max_tokens:          None,
            thinking:            None,
            tool_choice:         Default::default(),
            parallel_tool_calls: false,
            frequency_penalty:   None,
        };

        let r1 = driver.complete(request.clone()).await.unwrap();
        assert_eq!(r1.content.as_deref(), Some("first"));

        let r2 = driver.complete(request).await.unwrap();
        assert_eq!(r2.content.as_deref(), Some("second"));

        assert_eq!(driver.remaining(), 0);
        assert_eq!(driver.captured_requests().len(), 2);
    }

    #[tokio::test]
    async fn returns_error_when_exhausted() {
        let driver = ScriptedLlmDriver::new(vec![]);
        let request = CompletionRequest {
            model:               "test".into(),
            messages:            vec![],
            tools:               vec![],
            temperature:         None,
            max_tokens:          None,
            thinking:            None,
            tool_choice:         Default::default(),
            parallel_tool_calls: false,
            frequency_penalty:   None,
        };
        let result = driver.complete(request).await;
        assert!(result.is_err(), "should return error when exhausted");
    }

    #[tokio::test]
    async fn stream_sends_delta_then_done() {
        let driver = ScriptedLlmDriver::new(vec![scripted_response("hello")]);
        let (tx, mut rx) = mpsc::channel(16);

        let request = CompletionRequest {
            model:               "test".into(),
            messages:            vec![],
            tools:               vec![],
            temperature:         None,
            max_tokens:          None,
            thinking:            None,
            tool_choice:         Default::default(),
            parallel_tool_calls: false,
            frequency_penalty:   None,
        };

        let response = driver.stream(request, tx).await.unwrap();
        assert_eq!(response.content.as_deref(), Some("hello"));

        // Should have received TextDelta + Done
        let delta1 = rx.recv().await.unwrap();
        assert!(matches!(delta1, StreamDelta::TextDelta { text } if text == "hello"));
        let delta2 = rx.recv().await.unwrap();
        assert!(matches!(delta2, StreamDelta::Done { .. }));
    }
}
