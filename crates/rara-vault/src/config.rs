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

use std::{path::PathBuf, time::Duration};

use serde::{Deserialize, Serialize};

/// Top-level Vault configuration.
///
/// ```yaml
/// vault:
///   address: "http://10.0.0.5:30820"
///   mount_path: "secret/rara"
///   auth:
///     method: approle
///     role_id_file: /etc/rara/vault-role-id
///     secret_id_file: /etc/rara/vault-secret-id
///   watch_interval: 30s
///   timeout: 5s
///   fallback_to_local: true
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultConfig {
    /// Vault server address, e.g. `"http://10.0.0.5:30820"`.
    pub address: String,

    /// KV v2 mount path, e.g. `"secret/rara"`.
    #[serde(default = "default_mount_path")]
    pub mount_path: String,

    /// Authentication configuration.
    pub auth: VaultAuthConfig,

    /// How often to poll Vault for changes.
    #[serde(
        default = "default_watch_interval",
        deserialize_with = "humantime_serde::deserialize",
        serialize_with = "humantime_serde::serialize"
    )]
    pub watch_interval: Duration,

    /// HTTP request timeout.
    #[serde(
        default = "default_timeout",
        deserialize_with = "humantime_serde::deserialize",
        serialize_with = "humantime_serde::serialize"
    )]
    pub timeout: Duration,

    /// Whether to fall back to local config when Vault is unreachable.
    #[serde(default = "default_fallback")]
    pub fallback_to_local: bool,
}

/// Authentication method configuration for Vault AppRole.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultAuthConfig {
    /// Auth method name (currently only `"approle"` is supported).
    #[serde(default = "default_auth_method")]
    pub method: String,

    /// Path to a file containing the AppRole `role_id`.
    pub role_id_file: PathBuf,

    /// Path to a file containing the AppRole `secret_id`.
    pub secret_id_file: PathBuf,
}

fn default_mount_path() -> String { "secret/rara".into() }

fn default_watch_interval() -> Duration { Duration::from_secs(30) }

fn default_timeout() -> Duration { Duration::from_secs(5) }

fn default_fallback() -> bool { true }

fn default_auth_method() -> String { "approle".into() }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_roundtrip() {
        let config = VaultConfig {
            address:           "http://10.0.0.5:30820".into(),
            mount_path:        "secret/rara".into(),
            auth:              VaultAuthConfig {
                method:         "approle".into(),
                role_id_file:   "/etc/rara/vault-role-id".into(),
                secret_id_file: "/etc/rara/vault-secret-id".into(),
            },
            watch_interval:    Duration::from_secs(30),
            timeout:           Duration::from_secs(5),
            fallback_to_local: true,
        };

        let yaml = serde_yaml::to_string(&config).expect("serialize");
        let restored: VaultConfig = serde_yaml::from_str(&yaml).expect("deserialize");

        assert_eq!(restored.address, config.address);
        assert_eq!(restored.mount_path, config.mount_path);
        assert_eq!(restored.auth.method, config.auth.method);
        assert_eq!(restored.auth.role_id_file, config.auth.role_id_file);
        assert_eq!(restored.auth.secret_id_file, config.auth.secret_id_file);
        assert_eq!(restored.watch_interval, config.watch_interval);
        assert_eq!(restored.timeout, config.timeout);
        assert_eq!(restored.fallback_to_local, config.fallback_to_local);
    }

    #[test]
    fn deserialize_with_defaults() {
        let yaml = r#"
address: "http://localhost:8200"
auth:
  role_id_file: /tmp/role-id
  secret_id_file: /tmp/secret-id
"#;
        let config: VaultConfig = serde_yaml::from_str(yaml).expect("deserialize");
        assert_eq!(config.mount_path, "secret/rara");
        assert_eq!(config.watch_interval, Duration::from_secs(30));
        assert_eq!(config.timeout, Duration::from_secs(5));
        assert!(config.fallback_to_local);
        assert_eq!(config.auth.method, "approle");
    }
}
