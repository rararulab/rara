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
