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

//! Test utilities for building kernel instances with sensible defaults.
//!
//! Only available when the `testing` feature is enabled or in `#[cfg(test)]`.
//!
//! # Example
//!
//! ```rust,ignore
//! use rara_kernel::testing::TestKernelBuilder;
//! use rara_kernel::provider::ProviderRegistryBuilder;
//!
//! let registry = Arc::new(
//!     ProviderRegistryBuilder::new("openrouter", "openai/gpt-4o-mini")
//!         .provider("openrouter", Arc::new(my_provider))
//!         .build(),
//! );
//! let kernel = TestKernelBuilder::new()
//!     .provider_registry(registry)
//!     .max_concurrency(4)
//!     .build();
//! ```

use std::sync::Arc;

use dashmap::DashMap;
use tokio::sync::Semaphore;

use crate::{
    audit::{AuditLog, InMemoryAuditLog},
    defaults::{
        noop::{NoopEventBus, NoopGuard, NoopMemory, NoopSettingsProvider, NoopSessionRepository},
        noop_user_store::NoopUserStore,
    },
    device_registry::DeviceRegistry,
    event_queue::InMemoryEventQueue,
    io::{pipe::PipeRegistry, stream::StreamHub},
    kernel::{Kernel, KernelConfig, KernelInner},
    process::{AgentManifest, ProcessTable, agent_registry::AgentRegistry},
    provider::ProviderRegistry,
    session::SessionRepository,
    tool::{AgentToolRef, ToolRegistry},
};

/// Builder for constructing a [`Kernel`] with sensible test defaults.
///
/// All Noop implementations are used by default. The caller only needs to
/// provide the components relevant to their test (typically just the LLM
/// provider registry).
pub struct TestKernelBuilder {
    config:            KernelConfig,
    provider_registry: Option<Arc<ProviderRegistry>>,
    tool_registry:     ToolRegistry,
    agent_registry:    AgentRegistry,
}

impl TestKernelBuilder {
    pub fn new() -> Self {
        let agent_registry = AgentRegistry::new(
            test_manifests(),
            std::env::temp_dir().join("test_kernel_agents"),
        );

        Self {
            config: KernelConfig {
                max_concurrency:        16,
                default_child_limit:    8,
                default_max_iterations: 25,
                memory_quota_per_agent: 1000,
                ..Default::default()
            },
            provider_registry: None,
            tool_registry:     ToolRegistry::new(),
            agent_registry,
        }
    }

    /// Set the provider registry.
    pub fn provider_registry(mut self, registry: Arc<ProviderRegistry>) -> Self {
        self.provider_registry = Some(registry);
        self
    }

    /// Register a tool in the kernel's tool registry.
    pub fn tool(mut self, tool: AgentToolRef) -> Self {
        self.tool_registry.register_builtin(tool);
        self
    }

    /// Set the maximum global concurrency for agent processes.
    pub fn max_concurrency(mut self, n: usize) -> Self {
        self.config.max_concurrency = n;
        self
    }

    /// Set the default maximum LLM iterations for spawned agents.
    pub fn max_iterations(mut self, n: usize) -> Self {
        self.config.default_max_iterations = n;
        self
    }

    /// Set the default maximum number of children per agent.
    pub fn default_child_limit(mut self, n: usize) -> Self {
        self.config.default_child_limit = n;
        self
    }

    /// Build the [`Kernel`] with the configured components.
    ///
    /// # Panics
    ///
    /// Panics if no provider registry has been set. Use
    /// [`provider_registry`](Self::provider_registry) to provide one.
    pub fn build(self) -> Kernel {
        let provider_registry = self.provider_registry.expect(
            "TestKernelBuilder requires a ProviderRegistry — call .provider_registry() first",
        );

        let inner = Arc::new(KernelInner {
            process_table:          Arc::new(ProcessTable::new()),
            global_semaphore:       Arc::new(Semaphore::new(self.config.max_concurrency)),
            default_child_limit:    self.config.default_child_limit,
            default_max_iterations: self.config.default_max_iterations,
            provider_registry,
            tool_registry:          Arc::new(self.tool_registry),
            memory:                 Arc::new(NoopMemory),
            event_bus:              Arc::new(NoopEventBus),
            guard:                  Arc::new(NoopGuard),
            agent_registry:         Arc::new(self.agent_registry),
            shared_kv:              DashMap::new(),
            memory_quota_per_agent: self.config.memory_quota_per_agent,
            user_store:             Arc::new(NoopUserStore),
            session_repo:           Arc::new(NoopSessionRepository)
                as Arc<dyn SessionRepository>,
            settings:               Arc::new(NoopSettingsProvider)
                as Arc<dyn rara_domain_shared::settings::SettingsProvider>,
            stream_hub:             Arc::new(StreamHub::new(16)),
            pipe_registry:          Arc::new(PipeRegistry::new()),
            device_registry:        Arc::new(DeviceRegistry::new()),
            audit_log:              Arc::new(InMemoryAuditLog::default())
                as Arc<dyn AuditLog>,
            approval:               Arc::new(crate::approval::ApprovalManager::new(
                crate::approval::ApprovalPolicy::default(),
            )),
            event_queue:            Arc::new(InMemoryEventQueue::new(4096)),
        });

        // Use private constructor approach: build Kernel from its inner field.
        // We need access to the Kernel struct fields which are `pub(crate)`.
        Kernel::from_inner(inner, self.config)
    }
}

impl Default for TestKernelBuilder {
    fn default() -> Self { Self::new() }
}

/// Create minimal test manifests (no external YAML dependencies).
pub fn test_manifests() -> Vec<AgentManifest> {
    vec![
        AgentManifest {
            name:               "rara".to_string(),
            role:               None,
            description:        "Test chat agent".to_string(),
            model:              None,
            system_prompt:      "You are a helpful assistant.".to_string(),
            soul_prompt:        None,
            provider_hint:      None,
            max_iterations:     Some(25),
            tools:              vec![],
            max_children:       None,
            max_context_tokens: None,
            priority:           Default::default(),
            metadata:           Default::default(),
            sandbox:            None,
        },
        AgentManifest {
            name:               "scout".to_string(),
            role:               None,
            description:        "Test scout agent".to_string(),
            model:              Some("deepseek/deepseek-chat".to_string()),
            system_prompt:      "You are a scout agent.".to_string(),
            soul_prompt:        None,
            provider_hint:      None,
            max_iterations:     Some(15),
            tools:              vec!["read_file".to_string(), "grep".to_string()],
            max_children:       None,
            max_context_tokens: None,
            priority:           Default::default(),
            metadata:           Default::default(),
            sandbox:            None,
        },
    ]
}
