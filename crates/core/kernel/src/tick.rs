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

//! TickLoop — the kernel's main event loop that drains the [`InboundBus`]
//! and routes messages to long-lived agent processes.
//!
//! The tick loop is woken by the bus's `wait_for_messages()` mechanism
//! (no polling fallback). On each tick, it drains up to a configurable
//! batch size and dispatches each message:
//!
//! - If a process already exists for the session → send via mailbox
//! - If not → spawn a new long-lived process via `Kernel::spawn()`

use std::sync::Arc;

use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::{
    io::{bus::InboundBus, types::InboundMessage},
    kernel::Kernel,
    process::{AgentManifest, ProcessMessage, principal::Principal},
};

// ---------------------------------------------------------------------------
// TickLoop
// ---------------------------------------------------------------------------

/// The kernel's main event loop.
///
/// Drains the [`InboundBus`] in batches and routes messages to existing
/// agent processes (via mailbox) or spawns new ones via the [`Kernel`].
///
/// Stops gracefully when the [`CancellationToken`] is cancelled.
pub struct TickLoop {
    /// The inbound message bus to drain.
    inbound_bus: Arc<dyn InboundBus>,
    /// The kernel for spawning new processes and accessing the process table.
    kernel:      Arc<Kernel>,
    /// Maximum number of messages to drain per tick.
    batch_size:  usize,
}

impl TickLoop {
    /// Create a new tick loop.
    pub fn new(inbound_bus: Arc<dyn InboundBus>, kernel: Arc<Kernel>) -> Self {
        Self {
            inbound_bus,
            kernel,
            batch_size: 32,
        }
    }

    /// Create a new tick loop with a custom batch size.
    #[must_use]
    pub fn with_batch_size(mut self, batch_size: usize) -> Self {
        self.batch_size = batch_size;
        self
    }

    /// Main loop -- woken by the InboundBus, no polling.
    ///
    /// Runs until the shutdown token is cancelled.
    pub async fn run(&self, shutdown: CancellationToken) {
        info!("tick loop started");
        loop {
            tokio::select! {
                _ = self.inbound_bus.wait_for_messages() => {
                    self.tick().await;
                }
                _ = shutdown.cancelled() => {
                    info!("tick loop shutting down");
                    break;
                }
            }
        }
    }

    /// Process one tick: drain messages and dispatch.
    pub async fn tick(&self) {
        let messages = self.inbound_bus.drain(self.batch_size).await;
        for msg in messages {
            self.dispatch(msg).await;
        }
    }

    /// Dispatch a single message: route to existing process or spawn new one.
    async fn dispatch(&self, msg: InboundMessage) {
        let session_id = msg.session_id.clone();
        let user_id = msg.user.0.clone();

        // Try to find an existing process for this session
        let msg = if let Some(tx) = self
            .kernel
            .process_table()
            .find_by_session(&session_id)
            .and_then(|p| self.kernel.process_table().get_mailbox(&p.agent_id))
        {
            // Deliver to existing process via mailbox
            match tx.send(ProcessMessage::UserMessage(msg)).await {
                Ok(()) => return,
                Err(e) => {
                    warn!(
                        session_id = %session_id,
                        "mailbox send failed — process terminated, spawning new one"
                    );
                    // Recover the message from the SendError and fall through to spawn
                    match e.0 {
                        ProcessMessage::UserMessage(m) => m,
                        _ => return,
                    }
                }
            }
        } else {
            msg
        };

        // No existing process — spawn a new one
        let Some(manifest) = self.resolve_manifest().await else {
            error!(
                session_id = %session_id,
                "no model configured for key '{}' — cannot spawn agent; \
                 configure a model via settings",
                crate::model_repo::model_keys::CHAT,
            );
            return;
        };
        let principal = Principal::user(user_id);

        match self
            .kernel
            .spawn(manifest, msg, principal, session_id.clone(), None)
            .await
        {
            Ok(handle) => {
                info!(
                    session_id = %session_id,
                    agent_id = %handle.agent_id,
                    "spawned new process for session"
                );
            }
            Err(e) => {
                error!(
                    session_id = %session_id,
                    error = %e,
                    "failed to spawn process"
                );
            }
        }
    }

    /// Resolve the manifest for auto-spawned agents.
    ///
    /// Returns `None` if no model is configured — the caller must handle
    /// this as an error (user needs to configure a model via settings).
    async fn resolve_manifest(&self) -> Option<AgentManifest> {
        let model = self
            .kernel
            .model_repo()
            .get(crate::model_repo::model_keys::CHAT)
            .await?;
        Some(AgentManifest {
            name:           "io-agent".to_string(),
            description:    "I/O bus agent".to_string(),
            model,
            system_prompt:  "You are a helpful assistant.".to_string(),
            provider_hint:  None,
            max_iterations: Some(25),
            tools:          vec![],
            max_children:   None,
            metadata:       serde_json::Value::Null,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Arc};

    use super::*;
    use crate::{
        channel::types::{ChannelType, MessageContent},
        defaults::{
            noop::{
                NoopEventBus, NoopGuard, NoopIdentityResolver, NoopMemory,
                NoopSessionRepository, NoopSessionResolver,
            },
            noop_user_store::NoopUserStore,
        },
        io::{
            bus::{InboundBus, OutboundBus},
            ingress::{IdentityResolver, SessionResolver},
            memory_bus::{InMemoryInboundBus, InMemoryOutboundBus},
            stream::StreamHub,
            types::{ChannelSource, MessageId},
        },
        kernel::{Kernel, KernelConfig},
        model_repo::{ModelEntry, ModelRepo, ModelRepoError},
        process::{SessionId, manifest_loader::ManifestLoader, principal::UserId},
        provider::{EnvLlmProviderLoader, LlmProviderLoaderRef},
        session::SessionRepository,
        tool::ToolRegistry,
    };

    /// A test ModelRepo that always returns a configured model.
    struct TestModelRepo;

    #[async_trait::async_trait]
    impl ModelRepo for TestModelRepo {
        async fn get(&self, _key: &str) -> Option<String> {
            Some("test-model".to_string())
        }

        async fn set(&self, _key: &str, _model: &str) -> Result<(), ModelRepoError> {
            Ok(())
        }

        async fn remove(&self, _key: &str) -> Result<(), ModelRepoError> {
            Ok(())
        }

        async fn list(&self) -> Vec<ModelEntry> {
            vec![]
        }

        async fn fallback_models(&self) -> Vec<String> {
            vec![]
        }

        async fn set_fallback_models(&self, _models: Vec<String>) -> Result<(), ModelRepoError> {
            Ok(())
        }
    }

    /// Helper: build a test InboundMessage.
    fn test_inbound(session: &str, text: &str) -> InboundMessage {
        InboundMessage {
            id:            MessageId::new(),
            source:        ChannelSource {
                channel_type:        ChannelType::Telegram,
                platform_message_id: None,
                platform_user_id:    "tg-user".to_string(),
                platform_chat_id:    None,
            },
            user:          UserId("u1".to_string()),
            session_id:    SessionId::new(session),
            content:       MessageContent::Text(text.to_string()),
            reply_context: None,
            timestamp:     jiff::Timestamp::now(),
            metadata:      HashMap::new(),
        }
    }

    fn make_test_kernel() -> Arc<Kernel> {
        let config = KernelConfig {
            max_concurrency:        16,
            default_child_limit:    5,
            default_max_iterations: 5,
        };
        let mut loader = ManifestLoader::new();
        loader.load_bundled();

        Arc::new(Kernel::new(
            config,
            Arc::new(EnvLlmProviderLoader::default()) as LlmProviderLoaderRef,
            Arc::new(ToolRegistry::new()),
            Arc::new(NoopMemory),
            Arc::new(NoopEventBus),
            Arc::new(NoopGuard),
            loader,
            Arc::new(NoopUserStore),
            Arc::new(NoopSessionRepository) as Arc<dyn SessionRepository>,
            Arc::new(TestModelRepo) as Arc<dyn crate::model_repo::ModelRepo>,
            Arc::new(InMemoryInboundBus::new(128)) as Arc<dyn InboundBus>,
            Arc::new(InMemoryOutboundBus::new(64)) as Arc<dyn OutboundBus>,
            Arc::new(StreamHub::new(16)),
            Arc::new(NoopIdentityResolver) as Arc<dyn IdentityResolver>,
            Arc::new(NoopSessionResolver) as Arc<dyn SessionResolver>,
        ))
    }

    #[tokio::test]
    async fn test_tick_loop_drain_and_dispatch() {
        let inbound_bus = Arc::new(InMemoryInboundBus::new(100));

        // Publish 3 messages for different sessions
        inbound_bus
            .publish(test_inbound("s1", "hello s1"))
            .await
            .unwrap();
        inbound_bus
            .publish(test_inbound("s2", "hello s2"))
            .await
            .unwrap();
        inbound_bus
            .publish(test_inbound("s3", "hello s3"))
            .await
            .unwrap();

        let kernel = make_test_kernel();
        let tick_loop = TickLoop::new(inbound_bus.clone() as Arc<dyn InboundBus>, kernel.clone());

        // Run one tick
        tick_loop.tick().await;

        // All 3 messages should have been drained
        assert_eq!(inbound_bus.pending_count(), 0);

        // 3 processes should have been spawned (one per session)
        assert_eq!(kernel.process_table().list().len(), 3);
    }

    #[tokio::test]
    async fn test_tick_loop_shutdown() {
        let inbound_bus = Arc::new(InMemoryInboundBus::new(100));
        let kernel = make_test_kernel();
        let tick_loop = TickLoop::new(inbound_bus as Arc<dyn InboundBus>, kernel);

        let shutdown = CancellationToken::new();
        let shutdown_clone = shutdown.clone();

        let handle = tokio::spawn(async move {
            tick_loop.run(shutdown_clone).await;
        });

        // Cancel after a short delay
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        shutdown.cancel();

        // The loop should exit within a reasonable time
        let result = tokio::time::timeout(std::time::Duration::from_secs(2), handle).await;

        assert!(result.is_ok(), "tick loop should have exited on shutdown");
        assert!(
            result.unwrap().is_ok(),
            "tick loop task should complete successfully"
        );
    }
}
