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

//! `/anchor-mode` command — control automatic context folding aggressiveness
//! through user-facing presets.

use std::sync::Arc;

use async_trait::async_trait;
use rara_domain_shared::settings::SettingsProvider;
use rara_kernel::{
    channel::command::{
        CommandContext, CommandDefinition, CommandHandler, CommandInfo, CommandResult,
    },
    error::KernelError,
    kernel::{
        ContextFoldingConfig, ContextFoldingPreset, context_folding_mode_name,
        context_folding_settings_patch, effective_context_folding_config,
    },
};

use super::session::html_escape;

pub struct AnchorModeCommandHandler {
    settings: Arc<dyn SettingsProvider>,
    base_config: ContextFoldingConfig,
}

impl AnchorModeCommandHandler {
    pub fn new(settings: Arc<dyn SettingsProvider>, base_config: ContextFoldingConfig) -> Self {
        Self {
            settings,
            base_config,
        }
    }

    async fn effective_config(&self) -> ContextFoldingConfig {
        effective_context_folding_config(self.settings.as_ref(), &self.base_config).await
    }

    fn usage_text() -> &'static str {
        "Usage: /anchor-mode [chat|balanced|deep-work]"
    }

    fn render_settings(config: &ContextFoldingConfig) -> String {
        let fold_model = config.fold_model.as_deref().unwrap_or("(session model)");
        format!(
            "<code>enabled={}</code>\n<code>fold_threshold={:.2}</code>\n<code>min_entries_between_folds={}</code>\n<code>fold_model={}</code>",
            config.enabled,
            config.fold_threshold,
            config.min_entries_between_folds,
            html_escape(fold_model),
        )
    }

    fn render_readback(config: &ContextFoldingConfig) -> CommandResult {
        let mode = context_folding_mode_name(config);
        let practical = ContextFoldingPreset::parse(mode)
            .map(ContextFoldingPreset::practical_effect)
            .unwrap_or("Using a custom folding profile outside the built-in presets.");

        CommandResult::Html(format!(
            "<b>Anchor mode</b>: <code>{}</code>\n{}\nApplies from the next agent turn.\n\n<b>Current settings</b>\n{}",
            html_escape(mode),
            html_escape(practical),
            Self::render_settings(config),
        ))
    }
}

#[async_trait]
impl CommandHandler for AnchorModeCommandHandler {
    fn commands(&self) -> Vec<CommandDefinition> {
        vec![CommandDefinition {
            name: "anchor-mode".to_owned(),
            description: "Show or switch automatic anchor frequency presets".to_owned(),
            usage: Some("/anchor-mode [chat|balanced|deep-work]".to_owned()),
        }]
    }

    async fn handle(
        &self,
        command: &CommandInfo,
        _context: &CommandContext,
    ) -> Result<CommandResult, KernelError> {
        let raw_args = command.args.trim();
        if raw_args.is_empty() {
            return Ok(Self::render_readback(&self.effective_config().await));
        }

        let Some(preset) = ContextFoldingPreset::parse(raw_args) else {
            return Ok(CommandResult::Html(format!(
                "{}\nAvailable modes: <code>chat</code>, <code>balanced</code>, <code>deep-work</code>.",
                Self::usage_text()
            )));
        };

        let target_config = preset.config(self.base_config.fold_model.clone());
        self.settings
            .batch_update(context_folding_settings_patch(&target_config))
            .await
            .map_err(|error| KernelError::Other {
                message: format!("failed to update anchor mode: {error}").into(),
            })?;

        Ok(CommandResult::Html(format!(
            "<b>Anchor mode updated</b>: <code>{}</code>\n{}\nApplies from the next agent turn.\n\n<b>Applied settings</b>\n{}",
            preset.slug(),
            html_escape(preset.practical_effect()),
            Self::render_settings(&target_config),
        )))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    struct TestSettings {
        data: tokio::sync::RwLock<HashMap<String, String>>,
        tx: tokio::sync::watch::Sender<()>,
        rx: tokio::sync::watch::Receiver<()>,
    }

    impl TestSettings {
        fn new() -> Self {
            let (tx, rx) = tokio::sync::watch::channel(());
            Self {
                data: tokio::sync::RwLock::new(HashMap::new()),
                tx,
                rx,
            }
        }
    }

    #[async_trait]
    impl SettingsProvider for TestSettings {
        async fn get(&self, key: &str) -> Option<String> {
            self.data.read().await.get(key).cloned()
        }

        async fn set(&self, key: &str, value: &str) -> anyhow::Result<()> {
            self.data
                .write()
                .await
                .insert(key.to_owned(), value.to_owned());
            let _ = self.tx.send(());
            Ok(())
        }

        async fn delete(&self, key: &str) -> anyhow::Result<()> {
            self.data.write().await.remove(key);
            let _ = self.tx.send(());
            Ok(())
        }

        async fn list(&self) -> HashMap<String, String> {
            self.data.read().await.clone()
        }

        async fn batch_update(
            &self,
            patches: HashMap<String, Option<String>>,
        ) -> anyhow::Result<()> {
            let mut data = self.data.write().await;
            for (key, value) in patches {
                match value {
                    Some(value) => {
                        data.insert(key, value);
                    }
                    None => {
                        data.remove(&key);
                    }
                }
            }
            let _ = self.tx.send(());
            Ok(())
        }

        fn subscribe(&self) -> tokio::sync::watch::Receiver<()> {
            self.rx.clone()
        }
    }

    fn test_context(args: &str) -> CommandInfo {
        CommandInfo {
            name: "anchor-mode".to_owned(),
            args: args.to_owned(),
            raw: if args.is_empty() {
                "/anchor-mode".to_owned()
            } else {
                format!("/anchor-mode {args}")
            },
        }
    }

    fn test_command_context() -> CommandContext {
        CommandContext {
            channel_type: rara_kernel::channel::types::ChannelType::Cli,
            session_key: "session".to_owned(),
            user: rara_kernel::channel::types::ChannelUser {
                platform_id: "cli:test".to_owned(),
                display_name: Some("test".to_owned()),
            },
            metadata: HashMap::new(),
        }
    }

    #[tokio::test]
    async fn anchor_mode_readback_reports_effective_mode() {
        let settings = Arc::new(TestSettings::new());
        let handler = AnchorModeCommandHandler::new(
            settings,
            ContextFoldingPreset::Balanced.config(Some("fold-model".to_owned())),
        );

        let result = handler
            .handle(&test_context(""), &test_command_context())
            .await
            .expect("readback succeeds");

        match result {
            CommandResult::Html(html) => {
                assert!(html.contains("<code>balanced</code>"));
                assert!(html.contains("fold_model=fold-model"));
            }
            other => panic!("expected HTML response, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn anchor_mode_invalid_argument_returns_usage() {
        let settings = Arc::new(TestSettings::new());
        let handler =
            AnchorModeCommandHandler::new(settings, ContextFoldingPreset::Balanced.config(None));

        let result = handler
            .handle(&test_context("turbo"), &test_command_context())
            .await
            .expect("invalid argument still returns help");

        match result {
            CommandResult::Html(html) => {
                assert!(html.contains("Usage: /anchor-mode"));
                assert!(html.contains("deep-work"));
            }
            other => panic!("expected HTML response, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn anchor_mode_updates_settings_to_selected_preset() {
        use rara_domain_shared::settings::keys;

        let settings = Arc::new(TestSettings::new());
        let handler = AnchorModeCommandHandler::new(
            settings.clone(),
            ContextFoldingPreset::Balanced.config(Some("fold-model".to_owned())),
        );

        let result = handler
            .handle(&test_context("chat"), &test_command_context())
            .await
            .expect("set succeeds");

        match result {
            CommandResult::Html(html) => {
                assert!(html.contains("<code>chat</code>"));
                assert!(html.contains("working window smaller"));
            }
            other => panic!("expected HTML response, got {other:?}"),
        }

        assert_eq!(
            settings.get(keys::CONTEXT_FOLDING_ENABLED).await.as_deref(),
            Some("true")
        );
        assert_eq!(
            settings
                .get(keys::CONTEXT_FOLDING_FOLD_THRESHOLD)
                .await
                .as_deref(),
            Some("0.45")
        );
        assert_eq!(
            settings
                .get(keys::CONTEXT_FOLDING_MIN_ENTRIES_BETWEEN_FOLDS)
                .await
                .as_deref(),
            Some("8")
        );
    }

    #[tokio::test]
    async fn anchor_mode_help_is_discoverable_via_basic_handler() {
        let settings = Arc::new(TestSettings::new());
        let anchor_mode =
            AnchorModeCommandHandler::new(settings, ContextFoldingPreset::Balanced.config(None));
        let basic = super::super::basic::BasicCommandHandler::new(anchor_mode.commands());

        let result = basic
            .handle(
                &CommandInfo {
                    name: "help".to_owned(),
                    args: String::new(),
                    raw: "/help".to_owned(),
                },
                &test_command_context(),
            )
            .await
            .expect("help succeeds");

        match result {
            CommandResult::Html(html) => {
                assert!(html.contains("/anchor-mode [chat|balanced|deep-work]"));
            }
            other => panic!("expected HTML response, got {other:?}"),
        }
    }
}
