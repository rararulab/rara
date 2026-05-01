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

//! [`RuntimeEmbedder`] — registry-backed [`LlmEmbedder`] adapter.
//!
//! Sibling of [`RuntimeModelLister`](super::runtime_lister) for the
//! embedding path. Routes through [`DriverRegistry`](super::DriverRegistry) so
//! a settings switch of `llm.default_provider` redirects the knowledge layer's
//! next embed call to the new provider — same indirection, same
//! restart-free swap contract.

use async_trait::async_trait;
use snafu::OptionExt;

use super::{
    driver::LlmEmbedder,
    registry::DriverRegistryRef,
    types::{EmbeddingRequest, EmbeddingResponse},
};
use crate::error::{self, Result};

/// Registry-backed [`LlmEmbedder`] that delegates to the current
/// default driver on every call.
pub struct RuntimeEmbedder {
    registry: DriverRegistryRef,
}

impl RuntimeEmbedder {
    /// Create a new adapter that resolves through `registry`.
    pub fn new(registry: DriverRegistryRef) -> Self { Self { registry } }
}

#[async_trait]
impl LlmEmbedder for RuntimeEmbedder {
    async fn embed(&self, request: EmbeddingRequest) -> Result<EmbeddingResponse> {
        let name = self.registry.default_driver();
        let embedder = self
            .registry
            .get_embedder(&name)
            .context(error::ProviderNotConfiguredSnafu)?;
        embedder.embed(request).await
    }
}
