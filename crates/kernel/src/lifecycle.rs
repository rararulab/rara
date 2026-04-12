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

//! Lifecycle hooks for turn, session, and fold events.
//!
//! Inspired by Hermes Agent's `MemoryProvider` lifecycle hooks.
//! These are **internal Rust trait hooks** (not shell command hooks —
//! those live in [`super::hooks`]).  Memory providers, analytics, and
//! skill auto-creation implement this trait and register with the
//! [`LifecycleHookRegistry`].
//!
//! All hook methods have default no-op implementations so consumers
//! only override what they need.  Failures are logged, never block
//! the kernel.

use std::sync::Arc;

use async_trait::async_trait;

use crate::session::SessionKey;

// ---------------------------------------------------------------------------
// TurnContext — data passed to pre/post turn hooks
// ---------------------------------------------------------------------------

/// Context available to turn-level hooks.
#[derive(Debug, Clone)]
pub struct TurnContext {
    /// Session this turn belongs to.
    pub session_key: SessionKey,
    /// The user's message text that triggered this turn.
    pub user_text:   String,
    /// LLM model used for this turn.
    pub model:       String,
}

/// Result of a completed turn, passed to `post_turn`.
#[derive(Debug, Clone)]
pub struct TurnResult {
    /// Whether the turn completed successfully.
    pub success:     bool,
    /// Number of LLM iterations in this turn.
    pub iterations:  usize,
    /// Number of tool calls executed.
    pub tool_calls:  usize,
    /// The agent's final text output (may be empty for tool-only turns).
    pub output:      String,
    /// Error message if the turn failed.
    pub error:       Option<String>,
    /// Turn duration in milliseconds.
    pub duration_ms: u64,
}

/// Context for fold (context compression) hooks.
#[derive(Debug, Clone)]
pub struct FoldContext {
    /// Session being folded.
    pub session_key:   SessionKey,
    /// Context pressure ratio that triggered the fold (0.0–1.0).
    pub pressure:      f64,
    /// Number of tape entries being folded.
    pub entries_count: usize,
}

/// Context for delegation (child agent) hooks.
#[derive(Debug, Clone)]
pub struct DelegationResult {
    /// Parent session.
    pub parent_session: SessionKey,
    /// Child session that completed.
    pub child_session:  SessionKey,
    /// The task that was delegated.
    pub task:           String,
    /// Whether the child succeeded.
    pub success:        bool,
    /// Child's output text.
    pub output:         String,
}

// ---------------------------------------------------------------------------
// LifecycleHook trait
// ---------------------------------------------------------------------------

/// Trait for kernel lifecycle event consumers.
///
/// Implement the hooks you care about; the rest default to no-ops.
/// All methods receive shared references and must not block — long work
/// should be spawned as background tasks.
#[async_trait]
pub trait LifecycleHook: Send + Sync + 'static {
    /// Human-readable name for logging.
    fn name(&self) -> &str;

    /// Called before the agent loop starts for a turn.
    ///
    /// Use this for prefetching (e.g., recall relevant memory before the
    /// LLM sees the user message).
    async fn pre_turn(&self, _ctx: &TurnContext) {}

    /// Called after the agent loop completes a turn.
    ///
    /// Return an optional nudge message. The kernel persists it to the
    /// session tape as a system message so the LLM sees it on the next
    /// turn. Hooks never write to tape directly — all writes go through
    /// the kernel event pipeline.
    async fn post_turn(&self, _ctx: &TurnContext, _result: &TurnResult) -> Option<String> { None }

    /// Called before tape auto-fold compresses context.
    ///
    /// Use this to extract knowledge from messages about to be discarded.
    async fn pre_fold(&self, _ctx: &FoldContext) {}

    /// Called after fold completes with the summary.
    async fn post_fold(&self, _ctx: &FoldContext, _summary: &str) {}

    /// Called when a delegated child agent finishes.
    async fn delegation_done(&self, _result: &DelegationResult) {}
}

/// Shared reference to a lifecycle hook.
pub type LifecycleHookRef = Arc<dyn LifecycleHook>;

// ---------------------------------------------------------------------------
// HookRegistry
// ---------------------------------------------------------------------------

/// Registry of lifecycle hooks, called by the kernel at each event point.
///
/// Thread-safe and cheaply cloneable (inner `Arc`).
#[derive(Clone, Default)]
pub struct LifecycleHookRegistry {
    hooks: Arc<Vec<LifecycleHookRef>>,
}

impl LifecycleHookRegistry {
    /// Create an empty registry.
    pub fn new() -> Self { Self::default() }

    /// Create a registry with pre-registered hooks.
    pub fn with_hooks(hooks: Vec<LifecycleHookRef>) -> Self {
        Self {
            hooks: Arc::new(hooks),
        }
    }

    /// Fire `pre_turn` on all registered hooks.
    pub async fn fire_pre_turn(&self, ctx: &TurnContext) {
        for hook in self.hooks.iter() {
            if let Err(e) =
                tokio::time::timeout(std::time::Duration::from_secs(5), hook.pre_turn(ctx)).await
            {
                tracing::warn!(hook = hook.name(), "pre_turn hook timed out: {e}");
            }
        }
    }

    /// Fire `post_turn` on all registered hooks and collect nudge messages.
    ///
    /// The kernel writes returned nudges to the session tape so the LLM
    /// sees them on the next turn.
    pub async fn fire_post_turn(&self, ctx: &TurnContext, result: &TurnResult) -> Vec<String> {
        let mut nudges = Vec::new();
        for hook in self.hooks.iter() {
            match tokio::time::timeout(
                std::time::Duration::from_secs(5),
                hook.post_turn(ctx, result),
            )
            .await
            {
                Ok(Some(msg)) => nudges.push(msg),
                Ok(None) => {}
                Err(e) => {
                    tracing::warn!(hook = hook.name(), "post_turn hook timed out: {e}");
                }
            }
        }
        nudges
    }

    /// Fire `pre_fold` on all registered hooks.
    pub async fn fire_pre_fold(&self, ctx: &FoldContext) {
        for hook in self.hooks.iter() {
            if let Err(e) =
                tokio::time::timeout(std::time::Duration::from_secs(10), hook.pre_fold(ctx)).await
            {
                tracing::warn!(hook = hook.name(), "pre_fold hook timed out: {e}");
            }
        }
    }

    /// Fire `post_fold` on all registered hooks.
    pub async fn fire_post_fold(&self, ctx: &FoldContext, summary: &str) {
        for hook in self.hooks.iter() {
            if let Err(e) = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                hook.post_fold(ctx, summary),
            )
            .await
            {
                tracing::warn!(hook = hook.name(), "post_fold hook timed out: {e}");
            }
        }
    }

    /// Fire `delegation_done` on all registered hooks.
    pub async fn fire_delegation_done(&self, result: &DelegationResult) {
        for hook in self.hooks.iter() {
            if let Err(e) = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                hook.delegation_done(result),
            )
            .await
            {
                tracing::warn!(hook = hook.name(), "delegation_done hook timed out: {e}");
            }
        }
    }
}

impl std::fmt::Debug for LifecycleHookRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LifecycleHookRegistry")
            .field("hook_count", &self.hooks.len())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Built-in hooks
// ---------------------------------------------------------------------------

/// Nudges the agent to create a skill after complex turns (5+ tool calls).
pub struct SkillNudgeHook;

#[async_trait]
impl LifecycleHook for SkillNudgeHook {
    fn name(&self) -> &str { "skill-nudge" }

    async fn post_turn(&self, _ctx: &TurnContext, result: &TurnResult) -> Option<String> {
        if !result.success || result.tool_calls < 5 {
            return None;
        }
        Some(
            "[System nudge] You just completed a complex task with multiple tool calls. If you \
             discovered a non-trivial workflow, solved a tricky problem, or built something \
             reusable, save the approach as a skill with `create-skill` so you can reuse it next \
             time. Only create skills for genuinely reusable patterns — not for one-off tasks."
                .to_owned(),
        )
    }
}

/// Periodically nudges the agent to persist durable facts to memory.
///
/// Uses a simple turn counter — fires every `interval` successful turns.
pub struct MemoryNudgeHook {
    interval: std::sync::atomic::AtomicUsize,
    counter:  std::sync::atomic::AtomicUsize,
}

impl MemoryNudgeHook {
    /// Create a nudge hook that fires every `every_n_turns` successful turns.
    pub fn new(every_n_turns: usize) -> Self {
        Self {
            interval: std::sync::atomic::AtomicUsize::new(every_n_turns),
            counter:  std::sync::atomic::AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl LifecycleHook for MemoryNudgeHook {
    fn name(&self) -> &str { "memory-nudge" }

    async fn post_turn(&self, _ctx: &TurnContext, result: &TurnResult) -> Option<String> {
        if !result.success {
            return None;
        }
        let count = self
            .counter
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            + 1;
        let interval = self.interval.load(std::sync::atomic::Ordering::Relaxed);
        if interval == 0 || count % interval != 0 {
            return None;
        }
        Some(
            "[System nudge] Periodic memory check: review what you learned in recent turns. Save \
             durable facts using the memory tool — user preferences, environment details, tool \
             quirks, stable conventions. Prioritize what reduces future user corrections. Do NOT \
             save task progress or session outcomes — only facts that will still matter in future \
             sessions."
                .to_owned(),
        )
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    struct CountingHook {
        pre_count:  AtomicUsize,
        post_count: AtomicUsize,
    }

    impl CountingHook {
        fn new() -> Self {
            Self {
                pre_count:  AtomicUsize::new(0),
                post_count: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl LifecycleHook for CountingHook {
        fn name(&self) -> &str { "counting" }

        async fn pre_turn(&self, _ctx: &TurnContext) {
            self.pre_count.fetch_add(1, Ordering::Relaxed);
        }

        async fn post_turn(&self, _ctx: &TurnContext, _result: &TurnResult) -> Option<String> {
            self.post_count.fetch_add(1, Ordering::Relaxed);
            None
        }
    }

    fn test_turn_ctx() -> TurnContext {
        TurnContext {
            session_key: SessionKey::default(),
            user_text:   "hello".to_owned(),
            model:       "test-model".to_owned(),
        }
    }

    fn test_turn_result() -> TurnResult {
        TurnResult {
            success:     true,
            iterations:  1,
            tool_calls:  0,
            output:      "hi".to_owned(),
            error:       None,
            duration_ms: 100,
        }
    }

    #[tokio::test]
    async fn empty_registry_fires_without_error() {
        let registry = LifecycleHookRegistry::new();
        registry.fire_pre_turn(&test_turn_ctx()).await;
        registry
            .fire_post_turn(&test_turn_ctx(), &test_turn_result())
            .await;
    }

    #[tokio::test]
    async fn hooks_are_called_in_order() {
        let hook = Arc::new(CountingHook::new());
        let registry = LifecycleHookRegistry::with_hooks(vec![hook.clone()]);

        registry.fire_pre_turn(&test_turn_ctx()).await;
        assert_eq!(hook.pre_count.load(Ordering::Relaxed), 1);

        registry
            .fire_post_turn(&test_turn_ctx(), &test_turn_result())
            .await;
        assert_eq!(hook.post_count.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn multiple_hooks_all_fire() {
        let h1 = Arc::new(CountingHook::new());
        let h2 = Arc::new(CountingHook::new());
        let registry = LifecycleHookRegistry::with_hooks(vec![h1.clone() as _, h2.clone() as _]);

        registry.fire_pre_turn(&test_turn_ctx()).await;
        assert_eq!(h1.pre_count.load(Ordering::Relaxed), 1);
        assert_eq!(h2.pre_count.load(Ordering::Relaxed), 1);
    }
}
