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

use std::{collections::HashMap, env, time::Duration};

use anyhow::{Context, Result, anyhow};
use reqwest::{
    ClientBuilder,
    header::{HeaderMap, HeaderName, HeaderValue},
};
use tokio::time;

/// Await a fallible future with an optional timeout.
///
/// # Arguments
///
/// * `fut`     — The future to execute. Its error type must implement `Display`
///   so it can be wrapped in an `anyhow::Error`.
/// * `timeout` — Maximum wait duration. When `None`, waits indefinitely.
/// * `label`   — Human-readable description included in timeout / failure error
///   messages (e.g. `"MCP handshake"`).
pub(crate) async fn run_with_timeout<F, T, E>(
    fut: F,
    timeout: Option<Duration>,
    label: &str,
) -> Result<T>
where
    F: std::future::Future<Output = Result<T, E>>,
    E: std::fmt::Display,
{
    if let Some(duration) = timeout {
        let result = time::timeout(duration, fut)
            .await
            .with_context(|| anyhow!("timed out awaiting {label} after {duration:?}"))?;
        result.map_err(|err| anyhow!("{label} failed: {err}"))
    } else {
        fut.await.map_err(|err| anyhow!("{label} failed: {err}"))
    }
}

pub(crate) fn create_env_for_mcp_server(
    extra_env: Option<HashMap<String, String>>,
    env_vars: &[String],
) -> HashMap<String, String> {
    DEFAULT_ENV_VARS
        .iter()
        .copied()
        .chain(env_vars.iter().map(String::as_str))
        .filter_map(|var| env::var(var).ok().map(|value| (var.to_string(), value)))
        .chain(extra_env.unwrap_or_default())
        .collect()
}

#[cfg(unix)]
pub(crate) const DEFAULT_ENV_VARS: &[&str] = &[
    "HOME",
    "LOGNAME",
    "PATH",
    "SHELL",
    "USER",
    "__CF_USER_TEXT_ENCODING",
    "LANG",
    "LC_ALL",
    "TERM",
    "TMPDIR",
    "TZ",
];

pub(crate) fn build_default_headers(
    http_headers: Option<HashMap<String, String>>,
    env_http_headers: Option<HashMap<String, String>>,
) -> Result<HeaderMap> {
    let mut headers = HeaderMap::new();

    if let Some(static_headers) = http_headers {
        for (name, value) in static_headers {
            let header_name = match HeaderName::from_bytes(name.as_bytes()) {
                Ok(name) => name,
                Err(err) => {
                    tracing::warn!("invalid HTTP header name `{name}`: {err}");
                    continue;
                }
            };
            let header_value = match HeaderValue::from_str(value.as_str()) {
                Ok(value) => value,
                Err(err) => {
                    tracing::warn!("invalid HTTP header value for `{name}`: {err}");
                    continue;
                }
            };
            headers.insert(header_name, header_value);
        }
    }

    if let Some(env_headers) = env_http_headers {
        for (name, env_var) in env_headers {
            if let Ok(value) = env::var(&env_var) {
                if value.trim().is_empty() {
                    continue;
                }

                let header_name = match HeaderName::from_bytes(name.as_bytes()) {
                    Ok(name) => name,
                    Err(err) => {
                        tracing::warn!("invalid HTTP header name `{name}`: {err}");
                        continue;
                    }
                };

                let header_value = match HeaderValue::from_str(value.as_str()) {
                    Ok(value) => value,
                    Err(err) => {
                        tracing::warn!(
                            "invalid HTTP header value read from {env_var} for `{name}`: {err}"
                        );
                        continue;
                    }
                };
                headers.insert(header_name, header_value);
            }
        }
    }

    Ok(headers)
}

pub(crate) fn apply_default_headers(
    builder: ClientBuilder,
    default_headers: &HeaderMap,
) -> ClientBuilder {
    if default_headers.is_empty() {
        builder
    } else {
        builder.default_headers(default_headers.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── create_env_for_mcp_server ───────────────────────────────────

    #[test]
    fn create_env_includes_default_vars_from_host() {
        // PATH is always set on any Unix system.
        let result = create_env_for_mcp_server(None, &[]);
        assert!(
            result.contains_key("PATH"),
            "expected PATH in result, got keys: {:?}",
            result.keys().collect::<Vec<_>>()
        );
    }

    #[test]
    fn create_env_extra_overrides_defaults() {
        let extra = HashMap::from([("PATH".to_string(), "/custom/bin".to_string())]);
        let result = create_env_for_mcp_server(Some(extra), &[]);
        assert_eq!(result.get("PATH").map(String::as_str), Some("/custom/bin"));
    }

    #[test]
    fn create_env_forwards_explicit_env_vars() {
        // HOME is always set on Unix; use it as the explicit env var to forward.
        let result = create_env_for_mcp_server(None, &["HOME".to_string()]);
        assert!(
            result.contains_key("HOME"),
            "expected HOME in result (it should be forwarded as explicit var too)",
        );
    }

    #[test]
    fn create_env_skips_missing_env_vars() {
        let result = create_env_for_mcp_server(None, &["RARA_NONEXISTENT_VAR_12345".to_string()]);
        assert!(!result.contains_key("RARA_NONEXISTENT_VAR_12345"));
    }

    // ── build_default_headers ───────────────────────────────────────

    #[test]
    fn build_default_headers_with_static_headers() {
        let headers = HashMap::from([("x-api-key".to_string(), "secret".to_string())]);
        let result = build_default_headers(Some(headers), None).unwrap();
        assert_eq!(result.get("x-api-key").unwrap().to_str().unwrap(), "secret");
    }

    #[test]
    fn build_default_headers_with_env_headers() {
        // Use PATH which is always set and non-empty.
        let env_headers = HashMap::from([("x-env-header".to_string(), "PATH".to_string())]);
        let result = build_default_headers(None, Some(env_headers)).unwrap();
        assert!(
            result.get("x-env-header").is_some(),
            "expected x-env-header to be set from PATH env var",
        );
    }

    #[test]
    fn build_default_headers_skips_missing_env_var() {
        let env_headers = HashMap::from([(
            "x-missing".to_string(),
            "RARA_NONEXISTENT_VAR_99999".to_string(),
        )]);
        let result = build_default_headers(None, Some(env_headers)).unwrap();
        assert!(result.get("x-missing").is_none());
    }

    #[test]
    fn build_default_headers_skips_invalid_name() {
        // Header names cannot contain spaces.
        let headers = HashMap::from([("invalid header".to_string(), "value".to_string())]);
        let result = build_default_headers(Some(headers), None).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn build_default_headers_skips_invalid_value() {
        // Header values cannot contain \n.
        let headers = HashMap::from([("x-bad".to_string(), "line1\nline2".to_string())]);
        let result = build_default_headers(Some(headers), None).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn build_default_headers_empty_when_none() {
        let result = build_default_headers(None, None).unwrap();
        assert!(result.is_empty());
    }
}
