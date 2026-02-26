pub mod model;

/// Trait for crates that need to mutate runtime settings without depending on
/// the concrete [`SettingsSvc`](crate::settings) implementation.
#[async_trait::async_trait]
pub trait SettingsUpdater: Send + Sync {
    async fn update_settings(&self, patch: model::UpdateRequest)
    -> anyhow::Result<model::Settings>;
}
