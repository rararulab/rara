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

//! [`RuntimeModelLister`] — registry-backed [`LlmModelLister`] adapter.
//!
//! Boot used to clone a single `OpenAiDriver` into the chat-path
//! `LlmModelListerRef`, freezing the model list to the boot-time
//! provider for the lifetime of the process. This adapter routes
//! through [`DriverRegistry`](super::DriverRegistry) on every call instead, so
//! a `set_default_driver` on the registry — for example, after a
//! `PATCH /api/v1/settings { "llm.default_provider": "openrouter" }` —
//! takes effect on the next `list_models` invocation. See
//! `specs/issue-2014-chat-model-lister-runtime-provider-switch.spec.md`.

use async_trait::async_trait;
use snafu::OptionExt;

use super::{driver::LlmModelLister, registry::DriverRegistryRef, types::ModelInfo};
use crate::error::{self, Result};

/// Registry-backed [`LlmModelLister`] that delegates to the current
/// default driver on every call.
pub struct RuntimeModelLister {
    registry: DriverRegistryRef,
}

impl RuntimeModelLister {
    /// Create a new adapter that resolves through `registry`.
    pub fn new(registry: DriverRegistryRef) -> Self { Self { registry } }
}

#[async_trait]
impl LlmModelLister for RuntimeModelLister {
    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        let name = self.registry.default_driver();
        let lister = self
            .registry
            .get_lister(&name)
            .context(error::ProviderNotConfiguredSnafu)?;
        lister.list_models().await
    }
}
