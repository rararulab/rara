use std::sync::Arc;

use rara_composio::{ComposioAuthProvider, ComposioClient};

/// Shared state for all Composio tool variants.
#[derive(Clone)]
pub(super) struct ComposioShared {
    pub client: ComposioClient,
}

impl ComposioShared {
    pub fn from_auth_provider(auth_provider: Arc<dyn ComposioAuthProvider>) -> Self {
        Self {
            client: ComposioClient::with_auth_provider(auth_provider),
        }
    }

    /// Resolve entity_id from params or fall back to config default.
    pub async fn resolve_entity_id_async(&self, params: &serde_json::Value) -> String {
        if let Some(id) = params.get("entity_id").and_then(|v| v.as_str()) {
            return id.to_owned();
        }
        self.client
            .default_entity_id()
            .await
            .unwrap_or_else(|_| "default".to_owned())
    }
}
