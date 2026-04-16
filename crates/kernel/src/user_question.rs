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

//! User question manager — allows agent tools to block on user input.
//!
//! Follows the same oneshot-channel pattern as
//! [`crate::security::ApprovalManager`]: the `ask-user` tool calls
//! [`UserQuestionManager::ask`] which blocks on a oneshot channel until an
//! external listener (e.g. Telegram adapter) calls
//! [`UserQuestionManager::resolve`] with the user's answer.

use std::sync::Arc;

use dashmap::DashMap;
use serde::Serialize;
use snafu::Snafu;
use tracing::{info, warn};
use uuid::Uuid;

use crate::io::Endpoint;

/// A question submitted by the agent to the user.
#[derive(Debug, Clone, Serialize)]
pub struct UserQuestion {
    /// Unique identifier for this question.
    pub id:       Uuid,
    /// The question text to present to the user.
    pub question: String,
    /// Originating endpoint of the agent turn that raised this question.
    ///
    /// Channel adapters route the rendered prompt back to this endpoint (e.g.
    /// the same Telegram `(chat_id, thread_id)` the user was chatting in).
    /// `None` for origins that do not carry an endpoint (background tasks,
    /// legacy callers), in which case adapters fall back to a default
    /// destination such as Telegram's `primary_chat_id`.
    pub endpoint: Option<Endpoint>,
}

/// Error from user question operations.
#[derive(Debug, Clone, Snafu)]
#[snafu(visibility(pub))]
pub enum UserQuestionError {
    /// The question timed out before the user responded.
    #[snafu(display("user question timed out after {timeout_secs}s"))]
    TimedOut {
        /// Timeout duration in seconds.
        timeout_secs: u64,
    },
    /// No channel adapter is listening for user questions.
    #[snafu(display("no channel adapter is subscribed to user questions"))]
    NoSubscribers,
    /// The question ID was never seen.
    #[snafu(display("no pending user question: {id}"))]
    NotFound {
        /// The question ID that was not found.
        id: Uuid,
    },
}

/// Internal pending question holding the oneshot sender.
struct PendingQuestion {
    question: UserQuestion,
    sender:   tokio::sync::oneshot::Sender<String>,
}

/// Manages user questions with oneshot channels for blocking resolution.
///
/// When an agent calls the `ask-user` tool, execution blocks on a oneshot
/// channel until a channel adapter (e.g. Telegram) resolves it via
/// [`resolve()`](Self::resolve) or the request times out.
pub struct UserQuestionManager {
    pending:     DashMap<Uuid, PendingQuestion>,
    /// Broadcast channel for notifying external listeners (e.g. Telegram
    /// adapter) when a new question is submitted.
    question_tx: tokio::sync::broadcast::Sender<UserQuestion>,
}

/// Shared reference to a [`UserQuestionManager`].
pub type UserQuestionManagerRef = Arc<UserQuestionManager>;

impl UserQuestionManager {
    /// Create a new question manager.
    pub fn new() -> Self {
        let (question_tx, _) = tokio::sync::broadcast::channel(16);
        Self {
            pending: DashMap::new(),
            question_tx,
        }
    }

    /// Subscribe to new user questions. Channel adapters (e.g. Telegram)
    /// use this to render interactive question prompts to users.
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<UserQuestion> {
        self.question_tx.subscribe()
    }

    /// Submit a question and block until the user responds or the timeout
    /// expires.
    ///
    /// `endpoint` is the originating endpoint of the current turn (typically
    /// `ToolContext::origin_endpoint`). Channel adapters use it to route the
    /// question back to the same conversation surface (e.g. a Telegram forum
    /// topic) rather than a default destination.
    ///
    /// Fails immediately with [`NoSubscribersSnafu`] if no channel adapter is
    /// listening, avoiding a silent 5-minute hang.
    pub async fn ask(
        &self,
        question: String,
        endpoint: Option<Endpoint>,
        timeout: std::time::Duration,
    ) -> std::result::Result<String, UserQuestionError> {
        let id = Uuid::new_v4();
        let timeout_secs = timeout.as_secs();
        let uq = UserQuestion {
            id,
            question: question.clone(),
            endpoint,
        };

        let (tx, rx) = tokio::sync::oneshot::channel();

        // Insert BEFORE broadcasting to avoid a race where a fast subscriber
        // calls resolve() before the pending entry exists.
        self.pending.insert(
            id,
            PendingQuestion {
                question: uq.clone(),
                sender:   tx,
            },
        );

        // Notify external listeners (e.g. Telegram adapter).
        // Fail fast if nobody is listening — better than hanging for 5 minutes.
        if self.question_tx.send(uq).is_err() {
            self.pending.remove(&id);
            return Err(UserQuestionError::NoSubscribers);
        }

        info!(question_id = %id, %question, "user question submitted, waiting for answer");

        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(answer)) => {
                info!(question_id = %id, "user question answered");
                Ok(answer)
            }
            _ => {
                self.pending.remove(&id);
                warn!(question_id = %id, timeout_secs, "user question timed out");
                Err(UserQuestionError::TimedOut { timeout_secs })
            }
        }
    }

    /// Resolve a pending question with the user's answer.
    pub fn resolve(
        &self,
        question_id: Uuid,
        answer: String,
    ) -> std::result::Result<(), UserQuestionError> {
        match self.pending.remove(&question_id) {
            Some((_, pending)) => {
                let _ = pending.sender.send(answer);
                info!(question_id = %question_id, "user question resolved");
                Ok(())
            }
            None => Err(UserQuestionError::NotFound { id: question_id }),
        }
    }

    /// List all pending questions (for dashboard / API).
    pub fn list_pending(&self) -> Vec<UserQuestion> {
        self.pending
            .iter()
            .map(|r| r.value().question.clone())
            .collect()
    }

    /// Number of pending questions.
    pub fn pending_count(&self) -> usize { self.pending.len() }
}

impl Default for UserQuestionManager {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn ask_and_resolve() {
        let mgr = UserQuestionManager::new();
        let mut rx = mgr.subscribe();

        let mgr_ref = Arc::new(mgr);
        let mgr_clone = Arc::clone(&mgr_ref);

        let ask_handle = tokio::spawn(async move {
            mgr_clone
                .ask(
                    "What is your API key?".into(),
                    None,
                    std::time::Duration::from_secs(5),
                )
                .await
        });

        // Wait for the broadcast.
        let question = rx.recv().await.expect("should receive question");
        assert_eq!(question.question, "What is your API key?");

        // Resolve it.
        mgr_ref
            .resolve(question.id, "sk-12345".into())
            .expect("resolve should succeed");

        let answer = ask_handle.await.expect("task should complete");
        assert_eq!(answer.unwrap(), "sk-12345");
    }

    #[tokio::test]
    async fn ask_times_out() {
        let mgr = UserQuestionManager::new();
        // Need a subscriber so ask() doesn't fail with NoSubscribers.
        let _rx = mgr.subscribe();
        let result = mgr
            .ask(
                "question".into(),
                None,
                std::time::Duration::from_millis(10),
            )
            .await;
        assert!(matches!(result, Err(UserQuestionError::TimedOut { .. })));
        assert_eq!(mgr.pending_count(), 0);
    }

    #[tokio::test]
    async fn ask_no_subscribers() {
        let mgr = UserQuestionManager::new();
        // No subscriber — should fail immediately.
        let result = mgr
            .ask("question".into(), None, std::time::Duration::from_secs(5))
            .await;
        assert!(matches!(result, Err(UserQuestionError::NoSubscribers)));
    }

    #[tokio::test]
    async fn resolve_unknown_id() {
        let mgr = UserQuestionManager::new();
        let result = mgr.resolve(Uuid::new_v4(), "answer".into());
        assert!(matches!(result, Err(UserQuestionError::NotFound { .. })));
    }
}
