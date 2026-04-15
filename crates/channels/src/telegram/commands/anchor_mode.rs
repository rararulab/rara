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

//! `/anchor_mode` command — show or set auto-anchor aggressiveness presets.

use std::{collections::HashMap, fmt::Write, sync::Arc};

use async_trait::async_trait;
use rara_domain_shared::settings::{SettingsProvider, keys};
use rara_kernel::{
    channel::command::{
        CommandContext, CommandDefinition, CommandHandler, CommandInfo, CommandResult,
    },
    error::KernelError,
    kernel::ContextFoldingConfig,
};

use super::session::html_escape;

pub struct AnchorModeCommandHandler {
    settings:       Arc<dyn SettingsProvider>,
    default_config: ContextFoldingConfig,
}

impl AnchorModeCommandHandler {
    pub fn new(settings: Arc<dyn SettingsProvider>, default_config: ContextFoldingConfig) -> Self {
        Self {
            settings,
            default_config,
        }
    }

    async fn current_config(&self) -> ContextFoldingConfig {
        ContextFoldingConfig::resolve_from_settings(self.settings.as_ref(), &self.default_config)
            .await
    }

    async fn persist_preset(
        &self,
        preset: AnchorModePreset,
        current: &ContextFoldingConfig,
    ) -> Result<(), KernelError> {
        let mapped = preset.config();
        let mut patches = HashMap::new();
        patches.insert(
            keys::CONTEXT_FOLDING_ENABLED.to_owned(),
            Some(mapped.enabled.to_string()),
        );
        patches.insert(
            keys::CONTEXT_FOLDING_FOLD_THRESHOLD.to_owned(),
            Some(mapped.fold_threshold.to_string()),
        );
        patches.insert(
            keys::CONTEXT_FOLDING_MIN_ENTRIES_BETWEEN_FOLDS.to_owned(),
            Some(mapped.min_entries_between_folds.to_string()),
        );
        if let Some(ref fold_model) = current.fold_model {
            patches.insert(
                keys::CONTEXT_FOLDING_FOLD_MODEL.to_owned(),
                Some(fold_model.clone()),
            );
        }
        self.settings
            .batch_update(patches)
            .await
            .map_err(|error| KernelError::Other {
                message: format!("failed to update anchor mode: {error}").into(),
            })
    }
}

#[async_trait]
impl CommandHandler for AnchorModeCommandHandler {
    fn commands(&self) -> Vec<CommandDefinition> {
        vec![CommandDefinition {
            name:        "anchor_mode".to_owned(),
            description: "Show or set auto-anchor frequency: chat, balanced, deep-work".to_owned(),
            usage:       Some("/anchor_mode [chat|balanced|deep-work]".to_owned()),
        }]
    }

    async fn handle(
        &self,
        command: &CommandInfo,
        _context: &CommandContext,
    ) -> Result<CommandResult, KernelError> {
        let args = command.args.trim();
        if args.is_empty() || matches!(args, "show" | "status" | "current") {
            let current = self.current_config().await;
            return Ok(CommandResult::Html(render_anchor_mode_report(
                &current, None,
            )));
        }

        let Some(preset) = AnchorModePreset::parse(args) else {
            return Ok(CommandResult::Text(
                "Usage: /anchor_mode [chat|balanced|deep-work]".to_owned(),
            ));
        };

        let current = self.current_config().await;
        self.persist_preset(preset, &current).await?;
        let updated = self.current_config().await;

        Ok(CommandResult::Html(render_anchor_mode_report(
            &updated,
            Some(format!(
                "Updated anchor mode to <b>{}</b>. The new folding behavior applies on the next \
                 turn.",
                preset.label()
            )),
        )))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AnchorModePreset {
    Chat,
    Balanced,
    DeepWork,
}

impl AnchorModePreset {
    fn parse(raw: &str) -> Option<Self> {
        let normalized = raw.trim().to_ascii_lowercase().replace('_', "-");
        match normalized.as_str() {
            "chat" => Some(Self::Chat),
            "balanced" => Some(Self::Balanced),
            "deep-work" | "deepwork" => Some(Self::DeepWork),
            _ => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Chat => "chat",
            Self::Balanced => "balanced",
            Self::DeepWork => "deep-work",
        }
    }

    fn config(self) -> ContextFoldingConfig {
        match self {
            Self::Chat => ContextFoldingConfig {
                enabled:                   true,
                fold_threshold:            0.45,
                min_entries_between_folds: 6,
                fold_model:                None,
            },
            Self::Balanced => ContextFoldingConfig {
                enabled:                   true,
                fold_threshold:            0.60,
                min_entries_between_folds: 15,
                fold_model:                None,
            },
            Self::DeepWork => ContextFoldingConfig {
                enabled:                   true,
                fold_threshold:            0.68,
                min_entries_between_folds: 32,
                fold_model:                None,
            },
        }
    }

    fn behavior(self) -> &'static str {
        match self {
            Self::Chat => "Fold sooner so casual chat keeps a smaller working set.",
            Self::Balanced => "Keep the default balance between continuity and context cleanup.",
            Self::DeepWork => "Fold later so longer local reasoning stays in immediate context.",
        }
    }

    fn from_config(config: &ContextFoldingConfig) -> Option<Self> {
        for preset in [Self::Chat, Self::Balanced, Self::DeepWork] {
            let mapped = preset.config();
            if config.enabled == mapped.enabled
                && approx_eq(config.fold_threshold, mapped.fold_threshold)
                && config.min_entries_between_folds == mapped.min_entries_between_folds
            {
                return Some(preset);
            }
        }
        None
    }
}

fn render_anchor_mode_report(
    config: &ContextFoldingConfig,
    update_notice: Option<String>,
) -> String {
    let preset = AnchorModePreset::from_config(config);
    let mode_label = preset.map_or_else(
        || {
            if config.enabled {
                "custom".to_owned()
            } else {
                "custom (auto-fold off)".to_owned()
            }
        },
        |preset| preset.label().to_owned(),
    );
    let behavior = preset.map_or_else(
        || {
            if config.enabled {
                "Using custom context folding values."
            } else {
                "Automatic context folding is currently disabled."
            }
        },
        AnchorModePreset::behavior,
    );
    let fold_model = config
        .fold_model
        .as_deref()
        .map(html_escape)
        .unwrap_or_else(|| "session default".to_owned());

    let mut text = String::new();
    if let Some(update_notice) = update_notice {
        let _ = writeln!(text, "{update_notice}\n");
    }
    let _ = writeln!(text, "<b>Anchor mode</b>: {}", html_escape(&mode_label));
    let _ = writeln!(text, "<b>Behavior</b>: {}", html_escape(behavior));
    let _ = writeln!(text, "<b>Auto-fold enabled</b>: {}", config.enabled);
    let _ = writeln!(text, "<b>Fold threshold</b>: {:.2}", config.fold_threshold);
    let _ = writeln!(
        text,
        "<b>Min entries between folds</b>: {}",
        config.min_entries_between_folds
    );
    let _ = writeln!(text, "<b>Fold model</b>: {fold_model}");
    let _ = write!(
        text,
        "\nSet with <code>/anchor_mode chat</code>, <code>/anchor_mode balanced</code>, or \
         <code>/anchor_mode deep-work</code>."
    );
    text
}

fn approx_eq(left: f64, right: f64) -> bool { (left - right).abs() < 1e-9 }

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Arc};

    use async_trait::async_trait;
    use rara_domain_shared::settings::SettingsProvider;
    use rara_kernel::{
        channel::{
            command::{CommandContext, CommandHandler, CommandInfo, CommandResult},
            types::{ChannelType, ChannelUser},
        },
        kernel::ContextFoldingConfig,
    };

    use super::{AnchorModeCommandHandler, AnchorModePreset, approx_eq};

    struct StubSettings {
        data: tokio::sync::RwLock<HashMap<String, String>>,
        tx:   tokio::sync::watch::Sender<()>,
        rx:   tokio::sync::watch::Receiver<()>,
    }

    impl StubSettings {
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
    impl SettingsProvider for StubSettings {
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

        async fn list(&self) -> HashMap<String, String> { self.data.read().await.clone() }

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

        fn subscribe(&self) -> tokio::sync::watch::Receiver<()> { self.rx.clone() }
    }

    fn test_context() -> CommandContext {
        CommandContext {
            channel_type: ChannelType::Cli,
            session_key:  "cli-session".to_owned(),
            user:         ChannelUser {
                platform_id:  "cli:test".to_owned(),
                display_name: Some("test".to_owned()),
            },
            metadata:     HashMap::new(),
        }
    }

    fn balanced_config() -> ContextFoldingConfig { AnchorModePreset::Balanced.config() }

    #[tokio::test]
    async fn show_reports_current_mode() {
        let settings = Arc::new(StubSettings::new());
        let handler = AnchorModeCommandHandler::new(settings, balanced_config());

        let result = handler
            .handle(
                &CommandInfo {
                    name: "anchor_mode".to_owned(),
                    args: String::new(),
                    raw:  "/anchor_mode".to_owned(),
                },
                &test_context(),
            )
            .await
            .unwrap();

        match result {
            CommandResult::Html(text) => {
                assert!(text.contains("Anchor mode"));
                assert!(text.contains("balanced"));
            }
            other => panic!("unexpected command result: {other:?}"),
        }
    }

    #[tokio::test]
    async fn setting_chat_mode_updates_runtime_settings() {
        let settings = Arc::new(StubSettings::new());
        let handler = AnchorModeCommandHandler::new(settings.clone(), balanced_config());

        let result = handler
            .handle(
                &CommandInfo {
                    name: "anchor_mode".to_owned(),
                    args: "chat".to_owned(),
                    raw:  "/anchor_mode chat".to_owned(),
                },
                &test_context(),
            )
            .await
            .unwrap();

        match result {
            CommandResult::Html(text) => assert!(text.contains("chat")),
            other => panic!("unexpected command result: {other:?}"),
        }

        let effective =
            ContextFoldingConfig::resolve_from_settings(settings.as_ref(), &balanced_config())
                .await;
        assert!(effective.enabled);
        assert!(approx_eq(
            effective.fold_threshold,
            AnchorModePreset::Chat.config().fold_threshold,
        ));
        assert_eq!(
            effective.min_entries_between_folds,
            AnchorModePreset::Chat.config().min_entries_between_folds,
        );
    }

    #[tokio::test]
    async fn invalid_mode_returns_usage() {
        let settings = Arc::new(StubSettings::new());
        let handler = AnchorModeCommandHandler::new(settings, balanced_config());

        let result = handler
            .handle(
                &CommandInfo {
                    name: "anchor_mode".to_owned(),
                    args: "turbo".to_owned(),
                    raw:  "/anchor_mode turbo".to_owned(),
                },
                &test_context(),
            )
            .await
            .unwrap();

        match result {
            CommandResult::Text(text) => {
                assert!(text.contains("/anchor_mode"));
                assert!(text.contains("deep-work"));
            }
            other => panic!("unexpected command result: {other:?}"),
        }
    }
}
