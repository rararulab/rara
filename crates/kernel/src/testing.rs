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

//! Test utilities for building kernel instances with sensible defaults.
//!
//! Only available when the `testing` feature is enabled or in `#[cfg(test)]`.
//!
//! # Example
//!
//! ```rust,ignore
//! use rara_kernel::testing::TestKernelBuilder;
//! use rara_kernel::llm::DriverRegistryBuilder;
//!
//! let registry = Arc::new(
//!     DriverRegistryBuilder::new("test", "test-model")
//!         .driver("test", Arc::new(my_driver))
//!         .build(),
//! );
//! let kernel = TestKernelBuilder::new()
//!     .driver_registry(registry)
//!     .max_concurrency(4)
//!     .build();
//! ```

use std::sync::Arc;

use tokio::sync::Semaphore;

use crate::{
    device::DeviceRegistry,
    io::{pipe::PipeRegistry, stream::StreamHub},
    kernel::{Kernel, KernelConfig, SettingsRef, NoopSettingsProvider},
    llm::DriverRegistryRef,
    memory::NoopMemory,
    notification::NoopNotificationBus,
    process::{AgentManifest, ProcessTable, agent_registry::AgentRegistry},
    session::{NoopSessionIndex, NoopSessionRepository, SessionIndexRef, SessionRepoRef},
    tool::{AgentToolRef, ToolRegistry},
};

/// Builder for constructing a [`Kernel`] with sensible test defaults.
///
/// All Noop implementations are used by default. The caller only needs to
/// provide the components relevant to their test (typically just the LLM
/// driver registry).
pub struct TestKernelBuilder {
    config:          KernelConfig,
    driver_registry: Option<DriverRegistryRef>,
    tool_registry:   ToolRegistry,
    agent_registry:  AgentRegistry,
}

impl TestKernelBuilder {
    pub fn new() -> Self {
        let agent_registry = AgentRegistry::new(
            test_manifests(),
            std::env::temp_dir().join("test_kernel_agents"),
        );

        Self {
            config: KernelConfig {
                max_concurrency: 16,
                default_child_limit: 8,
                default_max_iterations: 25,
                memory_quota_per_agent: 1000,
                ..Default::default()
            },
            driver_registry: None,
            tool_registry: ToolRegistry::new(),
            agent_registry,
        }
    }

    /// Set the driver registry.
    pub fn driver_registry(mut self, registry: DriverRegistryRef) -> Self {
        self.driver_registry = Some(registry);
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
    /// Panics if no driver registry has been set. Use
    /// [`driver_registry`](Self::driver_registry) to provide one.
    pub fn build(self) -> Kernel {
        let driver_registry = self
            .driver_registry
            .expect("TestKernelBuilder requires a DriverRegistry — call .driver_registry() first");

        let max_concurrency = self.config.max_concurrency;
        Kernel::for_testing(
            self.config,
            Arc::new(ProcessTable::new()),
            Arc::new(Semaphore::new(max_concurrency)),
            driver_registry,
            Arc::new(self.tool_registry),
            Arc::new(NoopMemory),
            Arc::new(NoopNotificationBus),
            Arc::new(crate::security::SecuritySubsystem::noop()),
            Arc::new(self.agent_registry),
            Arc::new(crate::audit::AuditSubsystem::noop()),
            Arc::new(NoopSessionRepository) as SessionRepoRef,
            Arc::new(NoopSessionIndex) as SessionIndexRef,
            Arc::new(NoopSettingsProvider) as SettingsRef,
            Arc::new(StreamHub::new(16)),
            PipeRegistry::new(),
            Arc::new(DeviceRegistry::new()),
        )
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
