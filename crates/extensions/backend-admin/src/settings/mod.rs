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

pub mod router;
pub mod service;

pub use router::{SettingsRouterState, routes};
pub use service::SettingsSvc;

#[cfg(test)]
mod tests {
    //! Integration tests for the settings PATCH side-effects (#2014).
    //!
    //! These exercise `apply_default_provider_side_effects` end-to-end
    //! via the public router state — verifying that a PATCH touching
    //! `llm.default_provider` flips the registry default AND drops the
    //! chat-model cache before the next read.

    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use async_trait::async_trait;
    use rara_domain_shared::settings::keys;
    use rara_kernel::llm::{
        DriverRegistry, LlmModelLister, LlmModelListerRef, ModelInfo, OpenRouterCatalog,
        RuntimeModelLister,
    };

    use super::router::SettingsRouterState;
    use crate::chat::model_catalog::ModelCatalog;

    struct CountingLister {
        models: Vec<&'static str>,
        calls:  AtomicUsize,
    }

    #[async_trait]
    impl LlmModelLister for CountingLister {
        async fn list_models(&self) -> rara_kernel::error::Result<Vec<ModelInfo>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(self
                .models
                .iter()
                .map(|id| ModelInfo {
                    id:       (*id).to_owned(),
                    owned_by: String::new(),
                    created:  None,
                })
                .collect())
        }
    }

    /// Spec scenario `patch_default_provider_invalidates_chat_model_cache`:
    /// after a PATCH that switches `llm.default_provider` from
    /// `provider_a` to `provider_b`, the registry's `default_driver()`
    /// reports `provider_b` and the chat-model cache reports empty so
    /// the very next `list_models` call performs a fresh fetch.
    ///
    /// We exercise the same code path the HTTP handler runs by calling
    /// the side-effect helper directly through its public surface — the
    /// handler is a thin wrapper around it. A full HTTP e2e (axum
    /// `oneshot`) would need to mount the auth middleware fixtures and
    /// adds no behavioral coverage beyond what this asserts.
    #[tokio::test]
    async fn patch_default_provider_invalidates_chat_model_cache() {
        let registry = Arc::new(DriverRegistry::new(
            "provider_a",
            Arc::new(OpenRouterCatalog::new()),
        ));
        let lister_a: Arc<CountingLister> = Arc::new(CountingLister {
            models: vec!["model-a-1"],
            calls:  AtomicUsize::new(0),
        });
        let lister_b: Arc<CountingLister> = Arc::new(CountingLister {
            models: vec!["model-b-1"],
            calls:  AtomicUsize::new(0),
        });
        registry.register_lister("provider_a", lister_a.clone() as LlmModelListerRef);
        registry.register_lister("provider_b", lister_b.clone() as LlmModelListerRef);

        let runtime_lister: LlmModelListerRef = Arc::new(RuntimeModelLister::new(registry.clone()));
        let catalog = ModelCatalog::new(runtime_lister);

        // Prime the cache with provider_a entries so the test can later
        // observe the invalidation.
        let primed = catalog.list_models(&[]).await;
        assert!(primed.iter().any(|m| m.id == "model-a-1"));
        assert!(catalog.has_cached_entry().await, "cache must be populated");

        // Build the router state we'd hand to axum — same shape as the
        // production wiring in `BackendState::routes`.
        let state = SettingsRouterState {
            // The test does not invoke the HTTP layer, so the provider
            // is unused; pass a stub.
            provider:        Arc::new(StubProvider),
            driver_registry: registry.clone(),
            model_catalog:   catalog.clone(),
        };

        // Simulate the PATCH side effect.
        super::router::apply_default_provider_side_effects(
            &state,
            keys::LLM_DEFAULT_PROVIDER,
            Some("provider_b"),
        )
        .await;

        assert_eq!(
            registry.default_driver(),
            "provider_b",
            "registry default must reflect the new provider"
        );
        assert!(
            !catalog.has_cached_entry().await,
            "cache must be empty after invalidation so the next read fetches fresh"
        );

        // The next read goes to provider_b's catalog.
        let after = catalog.list_models(&[]).await;
        let ids: Vec<String> = after.into_iter().map(|m| m.id).collect();
        assert!(
            ids.contains(&"model-b-1".to_string()),
            "post-patch fetch must reach provider_b, got: {ids:?}"
        );
        assert!(
            !ids.contains(&"model-a-1".to_string()),
            "post-patch fetch must NOT serve stale provider_a entries"
        );
    }

    /// Minimal `SettingsProvider` stub — the side-effect helper only
    /// reads the registry + catalog, so the provider methods are never
    /// hit. Returning empty results keeps the impl trivially compatible
    /// with the trait.
    struct StubProvider;

    #[async_trait]
    impl rara_domain_shared::settings::SettingsProvider for StubProvider {
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
