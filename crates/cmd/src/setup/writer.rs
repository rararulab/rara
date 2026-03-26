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

//! Config assembly, masked preview, and file writing for the setup wizard.

use std::path::Path;

use snafu::{FromString, ResultExt, Whatever};

use super::prompt;

/// Assemble all step results into a [`serde_yaml::Value`].
///
/// When `base_yaml` is provided (Modify/FillMissing mode), step results are
/// merged into the existing config tree. Otherwise a fresh mapping is built.
pub fn assemble_config(
    base_yaml: Option<serde_yaml::Value>,
    db_url: Option<&str>,
    llm: Option<&super::llm::LlmResult>,
    telegram: Option<&super::telegram::TelegramResult>,
    users: Option<&[super::user::UserResult]>,
    stt: Option<&super::stt::SttResult>,
) -> Result<serde_yaml::Value, Whatever> {
    let mut root = base_yaml.unwrap_or(serde_yaml::Value::Mapping(serde_yaml::Mapping::new()));

    let map = root
        .as_mapping_mut()
        .ok_or(snafu::Whatever::without_source(
            "config root is not a mapping".into(),
        ))?;

    // Database
    if let Some(url) = db_url {
        let mut db = serde_yaml::Mapping::new();
        db.insert(y_str("database_url"), y_str_val(url));
        map.insert(y_str("database"), serde_yaml::Value::Mapping(db));
    }

    // LLM
    if let Some(llm) = llm {
        let mut provider = serde_yaml::Mapping::new();
        provider.insert(y_str("base_url"), y_str_val(&llm.base_url));
        provider.insert(y_str("api_key"), y_str_val(&llm.api_key));
        provider.insert(y_str("default_model"), y_str_val(&llm.default_model));

        let mut providers = serde_yaml::Mapping::new();
        providers.insert(
            y_str_val(&llm.provider_name),
            serde_yaml::Value::Mapping(provider),
        );

        let mut llm_section = serde_yaml::Mapping::new();
        llm_section.insert(y_str("default_provider"), y_str_val(&llm.provider_name));
        llm_section.insert(y_str("providers"), serde_yaml::Value::Mapping(providers));

        map.insert(y_str("llm"), serde_yaml::Value::Mapping(llm_section));
    }

    // Telegram
    if let Some(tg) = telegram {
        let mut tg_section = serde_yaml::Mapping::new();
        tg_section.insert(y_str("bot_token"), y_str_val(&tg.bot_token));
        tg_section.insert(y_str("chat_id"), y_str_val(&tg.chat_id));
        map.insert(y_str("telegram"), serde_yaml::Value::Mapping(tg_section));
    }

    // Users
    if let Some(users) = users {
        let user_list: Vec<serde_yaml::Value> = users
            .iter()
            .map(|u| {
                let mut user_map = serde_yaml::Mapping::new();
                user_map.insert(y_str("name"), y_str_val(&u.name));
                user_map.insert(y_str("role"), y_str_val(&u.role));

                if let Some(ref tg_id) = u.telegram_user_id {
                    let mut platform = serde_yaml::Mapping::new();
                    platform.insert(y_str("type"), y_str_val("telegram"));
                    platform.insert(y_str("user_id"), y_str_val(tg_id));
                    user_map.insert(
                        y_str("platforms"),
                        serde_yaml::Value::Sequence(vec![serde_yaml::Value::Mapping(platform)]),
                    );
                }

                serde_yaml::Value::Mapping(user_map)
            })
            .collect();
        map.insert(y_str("users"), serde_yaml::Value::Sequence(user_list));
    }

    // STT
    if let Some(stt) = stt {
        let mut stt_section = serde_yaml::Mapping::new();
        stt_section.insert(y_str("base_url"), y_str_val(&stt.base_url));
        stt_section.insert(y_str("model"), y_str_val(&stt.model));
        if let Some(ref lang) = stt.language {
            stt_section.insert(y_str("language"), y_str_val(lang));
        }
        map.insert(y_str("stt"), serde_yaml::Value::Mapping(stt_section));
    }

    Ok(root)
}

/// Serialize a config value to a YAML string.
pub fn to_yaml(config: &serde_yaml::Value) -> Result<String, Whatever> {
    serde_yaml::to_string(config).whatever_context("failed to serialize config to YAML")
}

/// Create a masked version of YAML for preview (hides API keys and tokens).
pub fn mask_secrets(yaml: &str) -> String {
    let mut output = String::new();
    for line in yaml.lines() {
        if let Some((key_part, val_part)) = line.split_once(": ") {
            let key_lower = key_part.trim().to_lowercase();
            if key_lower.contains("api_key")
                || key_lower.contains("bot_token")
                || key_lower.contains("owner_token")
                || key_lower.contains("secret")
            {
                let val = val_part.trim().trim_matches('"').trim_matches('\'');
                output.push_str(key_part);
                output.push_str(": ");
                output.push_str(&prompt::mask_secret(val));
                output.push('\n');
                continue;
            }
        }
        output.push_str(line);
        output.push('\n');
    }
    output
}

/// Backup existing config file (if any) and write the new YAML content.
pub fn write_config(config_path: &Path, yaml: &str) -> Result<(), Whatever> {
    if config_path.is_file() {
        let backup = config_path.with_extension("yaml.bak");
        std::fs::copy(config_path, &backup).whatever_context("failed to create backup")?;
        prompt::print_ok(&format!("backup saved to {}", backup.display()));
    }

    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent).whatever_context("failed to create config directory")?;
    }

    std::fs::write(config_path, yaml).whatever_context("failed to write config")?;

    prompt::print_ok(&format!("config written to {}", config_path.display()));
    Ok(())
}

/// Helper: create a YAML string key.
fn y_str(s: &str) -> serde_yaml::Value { serde_yaml::Value::String(s.to_owned()) }

/// Helper: create a YAML string value.
fn y_str_val(s: &str) -> serde_yaml::Value { serde_yaml::Value::String(s.to_owned()) }
