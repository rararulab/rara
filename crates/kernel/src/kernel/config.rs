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

//! Kernel configuration types.

use std::sync::Arc;

use crate::queue::ShardedEventQueueConfig;

// ---------------------------------------------------------------------------
// KernelConfig
// ---------------------------------------------------------------------------

/// Kernel configuration.
#[derive(Debug, Clone, smart_default::SmartDefault)]
pub struct KernelConfig {
    /// Maximum number of concurrent agent processes globally.
    #[default = 16]
    pub max_concurrency:        usize,
    /// Default maximum number of children per agent.
    #[default = 8]
    pub default_child_limit:    usize,
    /// Default max LLM iterations for spawned agents.
    #[default = 25]
    pub default_max_iterations: usize,
    /// Maximum number of KV entries per agent (0 = unlimited).
    /// Applies to the agent-scoped namespace only.
    #[default = 1000]
    pub memory_quota_per_agent: usize,
    /// Event queue configuration. Controls whether the kernel uses a single
    /// global queue (`num_shards = 0`) or sharded parallel processing.
    #[default(ShardedEventQueueConfig::single())]
    pub event_queue:            ShardedEventQueueConfig,
}

/// Shared reference to a
/// [`SettingsProvider`](rara_domain_shared::settings::SettingsProvider).
pub type SettingsRef = Arc<dyn rara_domain_shared::settings::SettingsProvider>;

// ---------------------------------------------------------------------------
// NoopSettingsProvider
// ---------------------------------------------------------------------------

mod noop {
    use async_trait::async_trait;

    /// A no-op settings provider for testing — always returns `None`.
    pub struct NoopSettingsProvider;

    #[async_trait]
    impl rara_domain_shared::settings::SettingsProvider for NoopSettingsProvider {
        async fn get(&self, _key: &str) -> Option<String> { None }

        async fn set(&self, _key: &str, _value: &str) -> anyhow::Result<()> { Ok(()) }

        async fn delete(&self, _key: &str) -> anyhow::Result<()> { Ok(()) }

        async fn list(&self) -> std::collections::HashMap<String, String> {
            std::collections::HashMap::new()
        }

        async fn batch_update(
            &self,
            _patches: std::collections::HashMap<String, Option<String>>,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        fn subscribe(&self) -> tokio::sync::watch::Receiver<()> {
            let (_tx, rx) = tokio::sync::watch::channel(());
            rx
        }
    }
}

pub use noop::NoopSettingsProvider;
