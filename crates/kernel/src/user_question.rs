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
use serde::{Deserialize, Serialize};
use tracing::{info, warn};
use uuid::Uuid;

/// A question submitted by the agent to the user.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserQuestion {
    /// Unique identifier for this question.
    pub id:       Uuid,
    /// The question text to present to the user.
    pub question: String,
}

/// Error from resolving a user question.
#[derive(Debug, Clone)]
pub enum ResolveError {
    /// The question timed out before the user responded.
    TimedOut,
    /// The question ID was never seen.
    NotFound(Uuid),
}

impl std::fmt::Display for ResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TimedOut => write!(f, "user question has timed out"),
            Self::NotFound(id) => write!(f, "no pending user question: {id}"),
        }
    }
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
    /// Returns the user's answer as a `String`, or `None` on timeout.
    pub async fn ask(&self, question: String, timeout: std::time::Duration) -> Option<String> {
        let id = Uuid::new_v4();
        let uq = UserQuestion {
            id,
            question: question.clone(),
        };

        let (tx, rx) = tokio::sync::oneshot::channel();

        // Notify external listeners before blocking.
        let _ = self.question_tx.send(uq.clone());

        self.pending.insert(
            id,
            PendingQuestion {
                question: uq,
                sender:   tx,
            },
        );

        info!(question_id = %id, %question, "user question submitted, waiting for answer");

        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(answer)) => {
                info!(question_id = %id, "user question answered");
                Some(answer)
            }
            _ => {
                self.pending.remove(&id);
                warn!(question_id = %id, "user question timed out");
                None
            }
        }
    }

    /// Resolve a pending question with the user's answer.
    pub fn resolve(
        &self,
        question_id: Uuid,
        answer: String,
    ) -> std::result::Result<(), ResolveError> {
        match self.pending.remove(&question_id) {
            Some((_, pending)) => {
                let _ = pending.sender.send(answer);
                info!(question_id = %question_id, "user question resolved");
                Ok(())
            }
            None => Err(ResolveError::NotFound(question_id)),
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
        assert_eq!(answer, Some("sk-12345".to_string()));
    }

    #[tokio::test]
    async fn ask_times_out() {
        let mgr = UserQuestionManager::new();
        let result = mgr
            .ask("question".into(), std::time::Duration::from_millis(10))
            .await;
        assert!(result.is_none());
        assert_eq!(mgr.pending_count(), 0);
    }

    #[tokio::test]
    async fn resolve_unknown_id() {
        let mgr = UserQuestionManager::new();
        let result = mgr.resolve(Uuid::new_v4(), "answer".into());
        assert!(result.is_err());
    }
}
