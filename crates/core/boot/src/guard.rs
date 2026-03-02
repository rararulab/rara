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

//! Guard construction helpers.

use rara_domain_shared::settings::{SettingsProvider, keys};
use rara_kernel::process::SandboxConfig;

/// Build a [`SandboxConfig`] from the settings provider.
///
/// Reads `fs.allowed_directories`, `fs.read_only_directories`, and
/// `fs.denied_directories` (stored as JSON string arrays).
pub async fn sandbox_config_from_settings(settings: &dyn SettingsProvider) -> SandboxConfig {
    let allowed = parse_json_string_array(settings.get(keys::FS_ALLOWED_DIRECTORIES).await);
    let read_only = parse_json_string_array(settings.get(keys::FS_READ_ONLY_DIRECTORIES).await);
    let denied = parse_json_string_array(settings.get(keys::FS_DENIED_DIRECTORIES).await);
    SandboxConfig {
        allowed_paths:      allowed,
        read_only_paths:    read_only,
        denied_paths:       denied,
        isolated_workspace: false,
    }
}

/// Parse an optional JSON string as a `Vec<String>`.
/// Returns empty vec if `None` or if JSON is invalid.
fn parse_json_string_array(value: Option<String>) -> Vec<String> {
    value
        .and_then(|v| serde_json::from_str::<Vec<String>>(&v).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_json_string_array_valid() {
        let input = Some(r#"["/tmp/test", "/data/shared"]"#.to_string());
        let result = parse_json_string_array(input);
        assert_eq!(result, vec!["/tmp/test", "/data/shared"]);
    }

    #[test]
    fn test_parse_json_string_array_empty() {
        let result = parse_json_string_array(None);
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_json_string_array_invalid_json() {
        let input = Some("not json".to_string());
        let result = parse_json_string_array(input);
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_json_string_array_empty_array() {
        let input = Some("[]".to_string());
        let result = parse_json_string_array(input);
        assert!(result.is_empty());
    }
}
