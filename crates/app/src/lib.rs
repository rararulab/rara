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

mod boot;
pub mod config_sync;
pub mod flatten;
pub mod gateway;
// Re-export `rara_kernel::tool` so the `ToolDef` proc macro can resolve
// `crate::tool::AgentTool` in derived impls.
pub(crate) use rara_kernel::tool;
mod feed_store;
pub mod sandbox;
pub mod tools;
mod web_server;

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use rara_kernel::channel::{
    adapter::ChannelAdapter,
    types::{ChannelType, GroupPolicy},
};
use rara_server::{
    grpc::{GrpcServerConfig, hello::HelloService, start_grpc_server},
    http::{RestServerConfig, health_routes, start_rest_server},
};
use serde::{Deserialize, Serialize};
use snafu::{ResultExt, Whatever};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};
use yunara_store::{config::DatabaseConfig, db::DBStore};

// ---------------------------------------------------------------------------
// Static config types (immutable after startup)
// ---------------------------------------------------------------------------

/// Static application configuration — immutable after startup.
///
/// Loaded from a YAML config file (see [`rara_paths::config_file()`]).
/// All required fields must be present; missing keys cause startup
/// failure with a clear error.
///
/// For runtime-changeable values (OpenRouter key, Telegram token, …) see
/// `rara_backend_admin::settings::SettingsSvc`.
#[derive(Debug, Clone, bon::Builder, Serialize, Deserialize)]
#[builder(on(String, into))]
pub struct AppConfig {
    /// Database connection pool (optional — defaults to max_connections=5).
    #[serde(default = "default_database_config")]
    pub database:               DatabaseConfig,
    /// HTTP server bind / limits.
    ///
    /// `RestServerConfig` is `SmartDefault` (binds `127.0.0.1:25555`), so an
    /// existing config that omits the section keeps booting on the standard
    /// port instead of hard-failing — see issue #1913 / `crates/app/AGENT.md`.
    #[serde(default)]
    pub http:                   RestServerConfig,
    /// gRPC server bind / limits.
    ///
    /// `GrpcServerConfig` is `SmartDefault` (binds `127.0.0.1:50051`).
    #[serde(default)]
    pub grpc:                   GrpcServerConfig,
    /// General OTLP telemetry (Alloy/Tempo).
    #[serde(default)]
    pub telemetry:              TelemetryConfig,
    // REQUIRED: defaulting an auth secret would silently authorize every
    // request — the operator must supply a real long random string.
    /// Static bearer token for owner authentication (Web UI + admin API).
    ///
    /// Required. The same token is accepted via `Authorization: Bearer`
    /// on admin HTTP endpoints and via `?token=` on the legacy WebSocket
    /// upgrade. Missing/empty at startup is a fatal config error.
    pub owner_token:            String,
    // REQUIRED: identifies which `users[]` entry owns the admin surface;
    // no safe default — picking arbitrarily would grant ownership to a
    // wrong principal.
    /// Kernel username resolved for authenticated owner requests.
    ///
    /// Must reference an entry in [`AppConfig::users`] whose role is
    /// `root` or `admin`. Validated at startup; boot fails if the user
    /// is missing or lacks admin privileges.
    pub owner_user_id:          String,
    /// LLM provider configuration (seeded to settings store at startup).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub llm:                    Option<flatten::LlmConfig>,
    /// Telegram bot configuration (seeded to settings store at startup).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub telegram:               Option<flatten::TelegramConfig>,
    /// WeChat iLink Bot configuration (seeded to settings store at startup).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wechat:                 Option<flatten::WechatConfig>,
    // REQUIRED: an empty users list would leave the kernel with no
    // resolvable identities; `owner_user_id` references this list, so a
    // missing/empty value is a hard boot-time error rather than a silent
    // default.
    /// Configured users with platform identity mappings (required).
    pub users:                  Vec<crate::boot::UserConfig>,
    /// Maximum ingress messages per user per minute (rate limiting).
    #[serde(default = "default_max_ingress_per_minute")]
    pub max_ingress_per_minute: u32,
    // REQUIRED: `heartbeat_interval` has no defensible default — a fast
    // tick burns LLM budget, a slow tick disables proactive behavior.
    // Operator must pick.
    /// Mita proactive agent configuration (required).
    pub mita:                   MitaConfig,
    /// Knowledge layer configuration (seeded to settings store at startup).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub knowledge:              Option<flatten::KnowledgeConfig>,
    /// Per-agent `{driver, model}` bindings (unified registry; #1636).
    ///
    /// ```yaml
    /// agents:
    ///   knowledge_extractor:
    ///     driver: "openrouter"
    ///     model: "gpt-4o-mini"
    /// ```
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agents:                 Option<flatten::AgentsConfig>,
    /// Speech-to-Text configuration (optional).
    /// When present, `base_url` is required — startup fails if missing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stt:                    Option<rara_stt::SttConfig>,
    /// Text-to-Speech configuration (optional).
    /// When present, voice replies are enabled for channels that support it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tts:                    Option<rara_tts::TtsConfig>,
    /// Gateway supervisor configuration (optional — used by `rara gateway`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gateway:                Option<GatewayConfig>,
    /// Context folding (auto-anchor) configuration for the kernel.
    #[serde(default)]
    pub context_folding:        rara_kernel::kernel::ContextFoldingConfig,
    /// Lightpanda browser subsystem (optional).
    ///
    /// When present, rara starts a Lightpanda CDP server and registers all
    /// browser tools (`browser-navigate`, `browser-click`, etc.). When absent
    /// or when the binary is not installed, browser tools are not available and
    /// rara falls back to `http-fetch` for web access.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub browser:                Option<rara_browser::BrowserConfig>,
    /// Sandboxed code execution (optional).
    ///
    /// When present, the `run_code` tool is registered and uses the configured
    /// rootfs image to spin up a per-session boxlite microVM. When absent,
    /// `run_code` is still registered but every invocation returns a clear
    /// "sandbox not configured" error so the LLM can react and the user can
    /// fix their YAML — no hardcoded image fallback (per
    /// `docs/guides/anti-patterns.md`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox:                Option<SandboxToolConfig>,
}

/// Configuration for the `run_code` sandbox tool.
///
/// The default rootfs image MUST live in YAML — there is no Rust fallback.
/// See `crates/app/src/tools/run_code.rs` and `crates/rara-sandbox/AGENT.md`
/// for the rationale.
#[derive(Debug, Clone, bon::Builder, Serialize, Deserialize)]
pub struct SandboxToolConfig {
    /// OCI image reference passed to boxlite (e.g. `"alpine:latest"`,
    /// `"python:3.12-slim"`). The image must already be resolvable by the
    /// host's boxlite image store.
    pub default_rootfs_image: String,
    /// Per-tool sandbox tuning for `bash`. `None` (the YAML default when
    /// the `bash:` block is absent) means `bash` runs with network
    /// `Disabled` and an empty allow-list — the safe ground state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bash:                 Option<BashSandboxConfig>,
}

/// Network and runtime policy for the sandboxed `bash` tool.
///
/// Per `docs/guides/rust-style.md`, this struct does NOT derive `Default` —
/// the absent state is represented by `Option<BashSandboxConfig>` on
/// [`SandboxToolConfig::bash`], not by a Rust-side default value.
#[derive(Debug, Clone, bon::Builder, Serialize, Deserialize)]
pub struct BashSandboxConfig {
    /// Hosts the sandboxed `bash` may reach over network. An empty list
    /// means no network access. A non-empty list is forwarded to boxlite as
    /// [`rara_sandbox::NetworkPolicy::Enabled`].
    ///
    /// Note: when `run_code` is also configured, the per-session VM uses
    /// the **fused** policy across all callers — see
    /// `crates/app/src/sandbox.rs` for the union rule.
    #[serde(default)]
    #[builder(default)]
    pub allow_net: Vec<String>,
}

/// Configuration for the Mita background proactive agent.
#[derive(Debug, Clone, bon::Builder, Serialize, Deserialize)]
pub struct MitaConfig {
    /// Heartbeat interval (e.g. "30m", "1800s").
    #[serde(
        deserialize_with = "humantime_serde::deserialize",
        serialize_with = "humantime_serde::serialize"
    )]
    pub heartbeat_interval: Duration,
}

/// Configuration for the gateway supervisor.
#[derive(Debug, Clone, bon::Builder, Serialize, Deserialize)]
pub struct GatewayConfig {
    /// Upstream check interval (e.g. "5m", "300s").
    #[serde(
        default = "gateway_defaults::check_interval",
        deserialize_with = "humantime_serde::deserialize",
        serialize_with = "humantime_serde::serialize"
    )]
    pub check_interval:       Duration,
    /// Total health confirmation timeout in seconds.
    #[serde(default = "gateway_defaults::health_timeout")]
    pub health_timeout:       u64,
    /// HTTP health poll interval (e.g. "2s").
    #[serde(
        default = "gateway_defaults::health_poll_interval",
        deserialize_with = "humantime_serde::deserialize",
        serialize_with = "humantime_serde::serialize"
    )]
    pub health_poll_interval: Duration,
    /// Max consecutive restart failures before giving up.
    #[serde(default = "gateway_defaults::max_restart_attempts")]
    pub max_restart_attempts: u32,
    /// Whether to auto-apply upstream updates.
    #[serde(default = "gateway_defaults::auto_update")]
    pub auto_update:          bool,
    /// Bind address for the gateway admin HTTP API.
    #[serde(default = "gateway_defaults::bind_address")]
    pub bind_address:         String,
    /// Repository URL for commit links in notifications (e.g. "<https://github.com/rararulab/rara>").
    pub repo_url:             String,
    /// Telegram bot token for the gateway management bot (separate from rara's
    /// bot).
    pub bot_token:            String,
    /// Telegram chat ID for the gateway bot (typically the admin's private
    /// chat).
    pub chat_id:              i64,
}

mod gateway_defaults {
    use std::time::Duration;
    pub fn check_interval() -> Duration { Duration::from_mins(5) }
    pub fn health_timeout() -> u64 { 30 }
    pub fn health_poll_interval() -> Duration { Duration::from_secs(2) }
    pub fn max_restart_attempts() -> u32 { 3 }
    pub fn auto_update() -> bool { true }
    pub fn bind_address() -> String { "127.0.0.1:25556".to_owned() }
}

/// General OTLP telemetry configuration.
#[derive(Debug, Clone, Default, bon::Builder, Serialize, Deserialize)]
pub struct TelemetryConfig {
    /// OTLP endpoint URL (e.g. `http://alloy:4318/v1/traces`).
    #[serde(default)]
    pub otlp_endpoint:    Option<String>,
    /// Export protocol: `"http"` or `"grpc"`.
    #[serde(default)]
    pub otlp_protocol:    Option<String>,
    /// Self-hosted Langfuse / OTLP HTTP traces exporter (opt-in).
    ///
    /// When `enabled`, the application configures an OTLP/HTTP traces
    /// exporter pointing at `traces_endpoint` with the provided `headers`
    /// (typically `authorization: "Basic <base64(public:secret)>"` for
    /// Langfuse). Disabled by default.
    #[serde(default)]
    pub otlp:             Option<OtlpConfig>,
    /// Continuous CPU profiling via Pyroscope. Section omitted = off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pyroscope:        Option<common_telemetry::profiling::PyroscopeConfig>,
    /// Deployment environment label (e.g. `"prod"`, `"dev"`). Used as
    /// a low-cardinality process-level tag on profiling samples.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env:              Option<String>,
    /// Layer B payload sampling. When absent, no Layer B attributes are
    /// emitted — Layer A keeps working as the always-on contract.
    #[serde(default)]
    pub payload_sampling: Option<common_telemetry::payload_sampler::PayloadSamplingConfig>,
}

/// OTLP/HTTP traces exporter config (Langfuse-compatible).
#[derive(Debug, Clone, bon::Builder, Serialize, Deserialize)]
pub struct OtlpConfig {
    /// Whether the exporter is active. `false` skips construction entirely.
    pub enabled:                Option<bool>,
    /// OTLP/HTTP traces ingest URL — full path including
    /// `/v1/traces` (or Langfuse's `/api/public/otel/v1/traces`).
    pub traces_endpoint:        Option<String>,
    /// HTTP headers attached to every export request.
    #[serde(default)]
    pub headers:                std::collections::HashMap<String, String>,
    /// Deployment environment label (e.g. `dev`, `staging`, `prod`)
    /// emitted as `deployment.environment.name` resource attribute.
    pub deployment_environment: Option<String>,
    /// Enable OTLP log export (separate from traces). Targets a
    /// log-specific backend like Loki's native OTLP receiver.
    #[serde(default)]
    pub logs_enabled:           Option<bool>,
    /// OTLP/HTTP logs ingest URL — full path including `/v1/logs`
    /// (e.g. Loki: `http://10.0.0.168:31100/otlp/v1/logs`).
    #[serde(default)]
    pub logs_endpoint:          Option<String>,
    /// HTTP headers attached to log exports. Loki requires
    /// `X-Scope-OrgID` even when `auth_enabled` is false.
    #[serde(default)]
    pub logs_headers:           std::collections::HashMap<String, String>,
}

fn default_database_config() -> DatabaseConfig { DatabaseConfig::builder().build() }
fn default_max_ingress_per_minute() -> u32 { 30 }

// ---------------------------------------------------------------------------
// Friendly "missing required field" error (#1913)
// ---------------------------------------------------------------------------
//
// `serde` reports missing fields as `missing field \`X\``. Operators reading
// that on startup have no way to recover without grepping the codebase, so we
// intercept the message and append a pointer at `config.example.yaml` where
// every top-level key first appears.

/// `config.example.yaml` shipped with the source tree, embedded at compile
/// time so we can quote line numbers from it without a runtime file read.
const EXAMPLE_YAML: &str = include_str!("../../../config.example.yaml");

/// Lazy index: top-level YAML key → 1-based line number in `EXAMPLE_YAML`.
///
/// Built once on first miss; subsequent misses are O(1).
fn example_line_index() -> &'static std::collections::HashMap<&'static str, usize> {
    use std::sync::OnceLock;
    static INDEX: OnceLock<std::collections::HashMap<&'static str, usize>> = OnceLock::new();
    INDEX.get_or_init(|| {
        let mut map = std::collections::HashMap::new();
        for (i, line) in EXAMPLE_YAML.lines().enumerate() {
            // Match top-level keys only — `^[a-z_]+:` with no leading
            // whitespace and a colon directly after the identifier. Skip
            // comment lines (`#`).
            let trimmed = line.trim_end();
            if trimmed.starts_with('#') || trimmed.starts_with(char::is_whitespace) {
                continue;
            }
            let Some(colon) = trimmed.find(':') else {
                continue;
            };
            let key = &trimmed[..colon];
            if !key
                .chars()
                .all(|c| c.is_ascii_lowercase() || c == '_' || c.is_ascii_digit())
                || key.is_empty()
            {
                continue;
            }
            // Only insert the first occurrence — operators want the example
            // stanza, not the last reference inside a comment.
            map.entry(Box::leak(key.to_owned().into_boxed_str()) as &'static str)
                .or_insert(i + 1);
        }
        map
    })
}

/// Extract the field name from a serde "missing field" message.
///
/// Handles both shapes seen in practice:
/// - serde direct: `... missing field \`X\` ...`
/// - `config` crate wrapped: `missing configuration field "X"`
fn parse_missing_field(msg: &str) -> Option<&str> {
    for (needle, terminator) in [
        ("missing field `", '`'),
        ("missing configuration field \"", '"'),
    ] {
        if let Some(idx) = msg.find(needle) {
            let rest = &msg[idx + needle.len()..];
            if let Some(end) = rest.find(terminator) {
                return Some(&rest[..end]);
            }
        }
    }
    None
}

/// Format the friendly missing-field error described in issue #1913.
///
/// Falls back to `None` if the underlying message does not match the
/// `missing field` shape — caller should keep its existing error path.
fn format_missing_field_error(raw: &str, config_path: &Path) -> Option<String> {
    let field = parse_missing_field(raw)?;
    let line_hint = example_line_index()
        .get(field)
        .map(|line| format!("See config.example.yaml line {line} for an example stanza.\n"))
        .unwrap_or_default();
    Some(format!(
        "Failed to load config: missing required field `{field}`.\n{line_hint}Config file: {}",
        config_path.display()
    ))
}

// ---------------------------------------------------------------------------
// StartOptions
// ---------------------------------------------------------------------------

/// Options for starting the application with custom adapters.
///
/// Used by `start_with_options` to inject pre-created adapters
/// (e.g. a [`TerminalAdapter`](rara_channels::terminal::TerminalAdapter)
/// for the CLI chat command).
#[derive(Default)]
pub struct StartOptions {
    /// CLI terminal adapter (if running in interactive CLI mode).
    pub cli_adapter: Option<Arc<rara_channels::terminal::TerminalAdapter>>,
}

/// Resolve the active `config.yaml` path using the same precedence as
/// [`AppConfig::new`]: prefer `$CWD/config.yaml` when it exists, else fall
/// back to [`rara_paths::config_file()`].
///
/// Both `AppConfig::new` (load step) and `start_with_options`
/// (`ConfigFileSync` watch target) call this so the two stay in lock-step.
/// Earlier, `start_with_options` hard-coded `$CWD/config.yaml` and
/// panicked at startup whenever the only config lived at the XDG path —
/// the exact arrangement `e2e.yml` set up via `XDG_CONFIG_HOME` after
/// PR #1948. Re-lands the inline fix originally authored on the
/// abandoned `issue-1850-live-e2e-in-ci` branch (commit `4f1e7f8b`) as a
/// shared helper so the two call sites cannot drift again.
fn resolve_config_path() -> PathBuf {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    resolve_config_path_from(&cwd, rara_paths::config_file())
}

/// Pure variant of [`resolve_config_path`] that takes the CWD and XDG
/// path explicitly, so unit tests can drive both without touching process
/// state.
fn resolve_config_path_from(cwd: &Path, xdg: &Path) -> PathBuf {
    let local = cwd.join("config.yaml");
    if local.is_file() {
        local
    } else {
        xdg.to_path_buf()
    }
}

/// Format the wrap-error message for a `ConfigFileSync::new` failure.
/// Includes the resolved path so an operator reading logs can identify
/// which file is missing without re-deriving the precedence rules.
fn config_file_sync_failure_message(path: &Path) -> String {
    format!(
        "Failed to initialize config file sync (resolved path: {})",
        path.display()
    )
}

impl AppConfig {
    /// Load config from YAML files.
    ///
    /// Sources (later sources override earlier ones):
    /// - global: [`rara_paths::config_file()`]
    /// - local override: `./config.yaml`
    ///
    /// All required fields must be present after merging; missing
    /// keys cause a deserialization error at startup.
    pub fn new() -> Result<Self, config::ConfigError> {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Self::load_from_paths(
            rara_paths::config_file().as_path(),
            &cwd.join("config.yaml"),
        )
    }

    fn load_from_paths(global_path: &Path, local_path: &Path) -> Result<Self, config::ConfigError> {
        if !(global_path.is_file() || local_path.is_file()) {
            return Err(config::ConfigError::Message(format!(
                "No config.yaml found. Looked for {} and {}",
                local_path.display(),
                global_path.display()
            )));
        }

        let cfg = config::Config::builder()
            .add_source(
                config::File::from(global_path)
                    .format(config::FileFormat::Yaml)
                    .required(false),
            )
            .add_source(
                config::File::from(local_path)
                    .format(config::FileFormat::Yaml)
                    .required(false),
            )
            .build()?;
        tracing::info!(?cfg, "Raw configuration");
        cfg.try_deserialize().map_err(|err| {
            // Pick whichever path actually contributed config (local wins
            // when both exist) so the operator sees the file they would
            // edit. If neither exists we already returned early above.
            let displayed_path = if local_path.is_file() {
                local_path
            } else {
                global_path
            };
            match format_missing_field_error(&err.to_string(), displayed_path) {
                Some(friendly) => config::ConfigError::Message(friendly),
                None => err,
            }
        })
    }
}

/// Initialize infrastructure, wire services, start servers & workers,
/// and block until shutdown.
pub async fn run(config: AppConfig) -> Result<(), Whatever> {
    let handle = start(config).await?;
    handle.wait_for_shutdown().await;
    Ok(())
}

/// Initialize infrastructure, wire services, start servers & workers,
/// and return a handle for controlling the running application.
pub async fn start(config: AppConfig) -> Result<AppHandle, Whatever> {
    start_with_options(config, StartOptions::default()).await
}

/// Initialize infrastructure, wire services, start servers & workers,
/// and return a handle for controlling the running application.
///
/// Accepts [`StartOptions`] for injecting pre-created adapters.
pub async fn start_with_options(
    config: AppConfig,
    options: StartOptions,
) -> Result<AppHandle, Whatever> {
    info!("Initializing job application");

    // Validate owner auth config before any subsystem starts. The backend
    // admin middleware resolves every authenticated request against this
    // identity, so a missing or under-privileged owner is a fatal config
    // error — not a runtime 500 per request.
    validate_owner_auth(&config)?;

    // Validate STT config: if section is present, base_url must be non-empty.
    if let Some(ref stt) = config.stt {
        snafu::ensure_whatever!(
            !stt.base_url.trim().is_empty(),
            "stt.base_url is required when stt section is configured"
        );
        info!(base_url = %stt.base_url, "STT service configured");
    }

    // If managed mode, spawn and wait for whisper-server before building STT
    // client.
    let whisper_process = if let Some(ref stt) = config.stt {
        if let Some(mut wp) = rara_stt::WhisperProcess::from_config(stt) {
            wp.start().await.whatever_context(
                "failed to start managed whisper-server (check stt.server_bin and stt.model_path)",
            )?;
            info!("managed whisper-server started");
            Some(wp)
        } else {
            None
        }
    } else {
        None
    };

    let stt_service = config.stt.as_ref().map(rara_stt::SttService::from_config);

    // Build TTS service when configured — symmetric to STT.
    if let Some(ref tts) = config.tts {
        snafu::ensure_whatever!(
            !tts.base_url.trim().is_empty(),
            "tts.base_url is required when tts section is configured"
        );
        info!(base_url = %tts.base_url, model = %tts.model, "TTS service configured");
    }
    let tts_service = config.tts.as_ref().map(rara_tts::TtsService::from_config);

    let db_store = init_infra(&config)
        .await
        .whatever_context("Failed to initialize infrastructure services")?;
    let diesel_pools = db_store.pools().clone();

    let settings_svc =
        rara_backend_admin::settings::SettingsSvc::load(db_store.kv_store(), diesel_pools.clone())
            .await
            .whatever_context("Failed to initialize runtime settings")?;

    let settings_provider: Arc<dyn rara_domain_shared::settings::SettingsProvider> =
        Arc::new(settings_svc.clone());
    info!("Runtime settings service loaded");

    // Resolve via the shared helper so this matches AppConfig::new's
    // precedence (local CWD override beats the XDG global file). Hard-coding
    // `$CWD/config.yaml` here was the bug behind issue #1981 / #1850 — when
    // CI ran in a directory without a local config, ConfigFileSync would
    // panic on a missing file even though AppConfig::new had succeeded
    // against the XDG path.
    let config_path = resolve_config_path();
    let config_file_sync = config_sync::ConfigFileSync::new(
        settings_provider.clone(),
        config.clone(),
        config_path.clone(),
    )
    .await
    .with_whatever_context::<_, _, snafu::Whatever>(|_| {
        config_file_sync_failure_message(&config_path)
    })?;

    // -- browser subsystem (optional) -------------------------------------
    // Start Lightpanda if a `browser:` section exists in config. Failure to
    // start is non-fatal — browser tools are simply not registered.
    let browser_manager: Option<rara_browser::BrowserManagerRef> =
        if let Some(browser_cfg) = config.browser.clone() {
            match rara_browser::BrowserManager::start(browser_cfg).await {
                Ok(manager) => {
                    info!("browser subsystem initialized with Lightpanda");
                    Some(std::sync::Arc::new(manager))
                }
                Err(e) => {
                    warn!(error = %e, "browser subsystem disabled — Lightpanda failed to start");
                    None
                }
            }
        } else {
            None
        };

    // Shared per-session sandbox map — the `run_code` tool inserts entries,
    // and the `SandboxCleanupHook` registered below removes them when the
    // owning session ends.
    let sandbox_map: crate::tools::SandboxMap = std::sync::Arc::new(dashmap::DashMap::new());

    let rara = crate::boot::boot(
        diesel_pools.clone(),
        settings_provider.clone(),
        &config.users,
        &config.owner_user_id,
        browser_manager,
        config.sandbox.clone(),
        sandbox_map.clone(),
    )
    .await
    .whatever_context("Failed to boot kernel dependencies")?;

    // -- Data feed subsystem --------------------------------------------------
    // Create the event channel and registry. The registry holds in-memory
    // feed configs + cancellation tokens; the channel carries FeedEvents
    // from all transports to the dispatch task.
    let (feed_event_tx, mut feed_event_rx) =
        tokio::sync::mpsc::channel::<rara_kernel::data_feed::FeedEvent>(256);
    let feed_registry = Arc::new(rara_kernel::data_feed::DataFeedRegistry::new(feed_event_tx));
    let feed_store: rara_kernel::data_feed::FeedStoreRef = Arc::new(
        crate::feed_store::SqliteFeedStore::new(diesel_pools.clone()),
    );
    let feed_svc = rara_backend_admin::data_feeds::DataFeedSvc::new(diesel_pools.clone());

    // Install the status reporter so runtime transitions (running / idle /
    // error + last_error) persist back to the `data_feeds` table.
    feed_registry.set_reporter(Arc::new(
        rara_backend_admin::data_feeds::SvcStatusReporter::new(feed_svc.clone()),
    ));

    // Install the status reporter so runtime transitions (running / idle /
    // error + last_error) persist back to the `data_feeds` table.
    feed_registry.set_reporter(Arc::new(
        rara_backend_admin::data_feeds::SvcStatusReporter::new(feed_svc.clone()),
    ));

    // Install the status reporter so runtime transitions (running / idle /
    // error + last_error) persist back to the `data_feeds` table.
    feed_registry.set_reporter(Arc::new(
        rara_backend_admin::data_feeds::SvcStatusReporter::new(feed_svc.clone()),
    ));

    // Install the status reporter so runtime transitions (running / idle /
    // error + last_error) persist back to the `data_feeds` table.
    feed_registry.set_reporter(Arc::new(
        rara_backend_admin::data_feeds::SvcStatusReporter::new(feed_svc.clone()),
    ));

    // Restore feed configs from database into registry.
    match feed_svc.list_feeds().await {
        Ok(configs) => {
            let count = configs.len();
            feed_registry.restore(configs);
            info!(count, "restored data feed configs from database");
        }
        Err(e) => {
            warn!(error = %e, "failed to restore data feed configs, starting with empty registry");
        }
    }

    let feed_router_state = rara_backend_admin::data_feeds::DataFeedRouterState {
        svc:      feed_svc,
        registry: feed_registry.clone(),
    };

    // TraceService is shared between the kernel (which writes traces at
    // turn end) and the backend session service (which reads them for
    // the web "📊 详情" button). Create it once here so both sides see
    // the same underlying pool.
    let trace_service = rara_kernel::trace::TraceService::new(diesel_pools.clone());

    let backend = rara_backend_admin::state::BackendState::init(
        rara.session_index.clone(),
        rara.tape_service.clone(),
        trace_service.clone(),
        settings_provider.clone(),
        settings_svc.clone(),
        rara.model_lister.clone(),
        feed_router_state,
        rara.driver_registry.clone(),
    )
    .await
    .whatever_context("Failed to initialize BackendState")?;

    // Created here (rather than below near `kernel.start`) so the web
    // reply-buffer sweeper task started a few lines down can listen on
    // the same shutdown signal as every other long-running task.
    let cancellation_token = CancellationToken::new();

    // The web reply buffer is always wired in production — the mechanism
    // is unconditional and its caps are `const` (see #1831 / #1907). The
    // sweeper runs until `cancellation_token` fires.
    let reply_buffer = rara_channels::web_reply_buffer::ReplyBuffer::new();
    Arc::clone(&reply_buffer).spawn_sweeper(cancellation_token.clone());

    let web_adapter = Arc::new(
        rara_channels::web::WebAdapter::new(
            config.owner_token.clone(),
            config.owner_user_id.clone(),
        )
        .with_stt_service(stt_service.clone())
        .with_reply_buffer(reply_buffer),
    );
    let web_router = web_adapter.router();

    let telegram_adapter = match try_build_telegram(
        &backend.settings_svc,
        rara.user_question_manager.clone(),
        stt_service,
        tts_service,
    )
    .await
    {
        Ok(Some(adapter)) => {
            info!("Telegram adapter built");
            Some(adapter)
        }
        Ok(None) => {
            info!("Telegram not configured (bot_token unset in settings), skipping");
            None
        }
        Err(e) => {
            warn!(error = %e, "Failed to build Telegram adapter, skipping");
            None
        }
    };

    let wechat_adapter = match try_build_wechat(&backend.settings_svc).await {
        Ok(Some(adapter)) => {
            info!("WeChat adapter built");
            Some(adapter)
        }
        Ok(None) => {
            info!("WeChat not configured (account_id unset in settings), skipping");
            None
        }
        Err(e) => {
            warn!(error = %e, "Failed to build WeChat adapter, skipping");
            None
        }
    };

    // Build IOSubsystem with all adapters before passing to Kernel.
    let notification_channel_id = settings_provider
        .get(rara_domain_shared::settings::keys::TELEGRAM_NOTIFICATION_CHANNEL_ID)
        .await
        .and_then(|s| s.parse::<i64>().ok());
    let mut io = rara_kernel::io::IOSubsystem::new(
        rara.identity_resolver.clone(),
        rara.session_index.clone(),
        notification_channel_id,
        config.max_ingress_per_minute,
    );
    if let Some(ref tg) = telegram_adapter {
        io.register_adapter(ChannelType::Telegram, tg.clone() as Arc<dyn ChannelAdapter>);
    }
    if let Some(ref wc) = wechat_adapter {
        io.register_adapter(ChannelType::Wechat, wc.clone() as Arc<dyn ChannelAdapter>);
    }
    io.register_adapter(
        ChannelType::Web,
        web_adapter.clone() as Arc<dyn ChannelAdapter>,
    );
    if let Some(ref cli) = options.cli_adapter {
        io.register_adapter(ChannelType::Cli, cli.clone() as Arc<dyn ChannelAdapter>);
    }

    let mcp_tool_provider: Option<rara_kernel::tool::DynamicToolProviderRef> = Some(Arc::new(
        boot::McpDynamicToolProvider::new(rara.mcp_manager.clone()),
    ));

    // Build the Layer B payload sampler. Default-on so the Langfuse UI
    // renders trace Input / Output without per-deployment YAML — operators
    // who want to dial back `on_success` set the field explicitly.
    let payload_sampler = Arc::new(
        common_telemetry::payload_sampler::PayloadSampler::from_optional_config(
            config.telemetry.payload_sampling.clone(),
        ),
    );

    // Reuse the existing `telemetry.otlp.deployment_environment` YAML key as
    // the source for `langfuse.environment`. Adding a new YAML key would
    // collide with `anti-patterns.md` ("would a deploy operator have a real
    // reason to pick a different value?" — they already pick this one for
    // `deployment.environment.name`). Falls back to `telemetry.env`.
    let langfuse_environment = config
        .telemetry
        .otlp
        .as_ref()
        .and_then(|o| o.deployment_environment.clone())
        .or_else(|| config.telemetry.env.clone());

    let kernel_config = rara_kernel::kernel::KernelConfig {
        mita_heartbeat_interval: Some(config.mita.heartbeat_interval),
        context_folding: config.context_folding.clone(),
        payload_sampler: Some(payload_sampler),
        langfuse_environment,
        ..Default::default()
    };

    // Build a closure that captures the skill registry and generates the
    // skills prompt block on each agent turn.
    let skill_prompt_provider: rara_kernel::handle::SkillPromptProvider = {
        let registry = rara.skill_registry.clone();
        Arc::new(move || {
            let skills = registry.list_all();
            rara_skills::prompt_gen::generate_skills_prompt(&skills)
        })
    };

    let mut kernel = rara_kernel::kernel::Kernel::builder()
        .config(kernel_config)
        .driver_registry(rara.driver_registry.clone())
        .tool_registry(rara.tool_registry.clone())
        .agent_registry(rara.agent_registry.clone())
        .session_index(rara.session_index.clone())
        .tape_service(rara.tape_service.clone())
        .settings(settings_provider.clone())
        .security(Arc::new(rara_kernel::security::SecuritySubsystem::new(
            rara.user_store.clone(),
            Arc::new(rara_kernel::security::ApprovalManager::new(
                rara_kernel::security::ApprovalPolicy::default(),
            )),
        )))
        .io(io)
        .knowledge(rara.knowledge_service.clone())
        .maybe_dynamic_tool_provider(mcp_tool_provider)
        .trace_service(trace_service)
        .skill_prompt_provider(skill_prompt_provider)
        .scheduler_dir(rara_paths::workspace_dir().join("scheduler"))
        .build();

    // Supervisor restarts whisper-server on crash, stops on app shutdown.
    let _whisper_supervisor =
        whisper_process.map(|wp| wp.spawn_supervisor(cancellation_token.clone()));

    // Start bidirectional config <-> settings sync
    {
        let cancel = cancellation_token.clone();
        tokio::spawn(async move {
            config_file_sync.start(cancel).await;
        });
    }

    // Register lifecycle hooks for the closed learning loop and per-session
    // resource cleanup.
    kernel.set_lifecycle_hooks(rara_kernel::lifecycle::LifecycleHookRegistry::with_hooks(
        vec![
            std::sync::Arc::new(rara_kernel::lifecycle::SkillNudgeHook),
            std::sync::Arc::new(rara_kernel::lifecycle::MemoryNudgeHook::new(10)),
            std::sync::Arc::new(crate::tools::SandboxCleanupHook::new(sandbox_map.clone())),
        ],
    ));

    // Wire data feed subsystem into the kernel before start().
    kernel.set_feed_subsystem(feed_registry.clone(), feed_store.clone());

    let (_kernel_arc, kernel_handle) = kernel.start(cancellation_token.clone());

    // Inject the kernel handle into the session service so endpoints that
    // need to drive the kernel directly (e.g. POST .../regenerate-title)
    // can reach it. `BackendState::init` had to run before kernel boot so
    // the handle is wired in here.
    backend
        .session_service
        .set_kernel_handle(kernel_handle.clone());

    // Spawn the feed dispatch task — consumes events from all transports,
    // persists them to the data_feed_events table, and routes matching events
    // to subscribing sessions via SubscriptionRegistry.
    {
        let store = feed_store.clone();
        let handle = kernel_handle.clone();
        tokio::spawn(async move {
            while let Some(event) = feed_event_rx.recv().await {
                if let Err(e) = store.append(&event).await {
                    warn!(
                        source = %event.source_name,
                        error = %e,
                        "failed to persist feed event"
                    );
                }

                // Route to subscribers whose tags overlap with the event.
                let matched = handle
                    .subscription_registry()
                    .match_tags_any_owner(&event.tags)
                    .await;
                if matched.is_empty() {
                    continue;
                }

                let event_json = serde_json::to_value(&event).unwrap_or_default();
                let payload_pretty =
                    serde_json::to_string_pretty(&event.payload).unwrap_or_default();

                use rara_kernel::notification::NotifyAction;

                let futs: Vec<_> = matched
                    .into_iter()
                    .map(|sub| {
                        let handle = handle.clone();
                        let event_json = event_json.clone();
                        let payload_pretty = payload_pretty.clone();
                        let source_name = event.source_name.clone();
                        let event_type = event.event_type.clone();
                        let tags = event.tags.clone();
                        async move {
                            match sub.on_receive {
                                NotifyAction::ProactiveTurn => {
                                    if !handle.process_table().contains(&sub.subscriber) {
                                        warn!(
                                            subscriber = %sub.subscriber,
                                            "feed ProactiveTurn downgraded to SilentAppend: \
                                             subscriber session not in process table"
                                        );
                                        let sub_tape = sub.subscriber.to_string();
                                        let _ = handle
                                            .tape()
                                            .store()
                                            .append(
                                                &sub_tape,
                                                rara_kernel::memory::TapEntryKind::FeedEvent,
                                                event_json,
                                                None,
                                            )
                                            .await;
                                        return;
                                    }
                                    let directive = format!(
                                        "[FeedEvent] source={source_name} type={event_type} \
                                         tags={tags:?}\n{payload_pretty}",
                                    );
                                    let msg = rara_kernel::io::InboundMessage::synthetic(
                                        directive,
                                        sub.owner.clone(),
                                        sub.subscriber,
                                    );
                                    handle.deliver_internal(msg).await;
                                }
                                NotifyAction::SilentAppend => {
                                    let sub_tape = sub.subscriber.to_string();
                                    let _ = handle
                                        .tape()
                                        .store()
                                        .append(
                                            &sub_tape,
                                            rara_kernel::memory::TapEntryKind::FeedEvent,
                                            event_json,
                                            None,
                                        )
                                        .await;
                                }
                            }
                        }
                    })
                    .collect();
                futures::future::join_all(futs).await;

                info!(
                    source = %event.source_name,
                    event_type = %event.event_type,
                    "feed event dispatched to subscribers"
                );
            }
            info!("feed dispatch task stopped (channel closed)");
        });
    }

    // Start polling feed tasks for all enabled polling-type feeds that were
    // restored from the database.
    {
        let configs = feed_registry.list();
        for config in &configs {
            if config.enabled {
                rara_backend_admin::data_feeds::start_feed_task(config, &feed_registry);
            }
        }
    }

    // Wire DispatchRaraTool and ListSessionsTool with the now-available
    // KernelHandle.
    {
        let mut lock = rara.dispatch_rara_handle.write().await;
        *lock = Some(kernel_handle.clone());
    }
    {
        let mut lock = rara.list_sessions_handle.write().await;
        *lock = Some(kernel_handle.clone());
    }

    // MCP heartbeat: reconnect dead servers periodically
    {
        let mcp_mgr = rara.mcp_manager.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(30));
            ticker.tick().await; // skip immediate first tick
            loop {
                ticker.tick().await;
                mcp_mgr.reconnect_dead().await;
            }
        });
    }

    let auth_state = rara_backend_admin::auth::AuthState::new(
        config.owner_token.clone(),
        config.owner_user_id.clone(),
        &kernel_handle,
    );
    let (domain_routes, _openapi) = backend.routes(
        &kernel_handle,
        &rara.skill_registry,
        &rara.mcp_manager,
        auth_state,
        &config.http.cors_allowed_origins,
    );

    // Build webhook routes for passive data feed ingestion.
    let webhook_state = Arc::new(rara_kernel::data_feed::webhook::WebhookState::new(
        feed_registry.clone(),
        feed_registry.event_tx(),
    ));
    let webhook_router =
        (rara_kernel::data_feed::webhook::webhook_routes(webhook_state))(axum::Router::new());

    // CORS wraps the outermost composed router so every public surface
    // (health, webhook, kernel chat, admin) shares one allow-list.
    // See `rara_backend_admin::state::build_cors_layer` for the rationale.
    let cors_origins = config.http.cors_allowed_origins.clone();
    let routes_fn: Box<dyn Fn(axum::Router) -> axum::Router + Send + Sync> =
        Box::new(move |router| {
            health_routes(router)
                .merge(domain_routes.clone())
                .merge(webhook_router.clone())
                .nest("/api/v1/kernel/chat", web_router.clone())
                .layer(rara_backend_admin::state::build_cors_layer(&cors_origins))
        });

    info!("Application initialized successfully");

    let running = Arc::new(AtomicBool::new(true));
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

    let mut grpc_handle = start_grpc_server(&config.grpc, &[Arc::new(HelloService)])
        .whatever_context("Failed to start gRPC server")?;
    info!("starting rest server ...");
    let mut http_handle = start_rest_server(config.http.clone(), vec![routes_fn])
        .await
        .whatever_context("Failed to start REST server")?;

    grpc_handle
        .wait_for_start()
        .await
        .whatever_context("gRPC server failed to report started")?;
    http_handle
        .wait_for_start()
        .await
        .whatever_context("REST server failed to report started")?;

    // Signal readiness to the gateway supervisor (if present).
    // The gateway watches our stdout for this marker.
    // tracing goes to stderr, so this does not interfere.
    println!("READY");

    // Build a shared service client used by both command and callback handlers.
    let bot_client: std::sync::Arc<dyn rara_channels::telegram::commands::BotServiceClient> = {
        use rara_channels::telegram::commands::KernelBotServiceClient;
        std::sync::Arc::new(KernelBotServiceClient::new(
            rara.session_index.clone(),
            rara.tape_service.clone(),
            kernel_handle.clone(),
            rara.mcp_manager.clone(),
            rara.model_lister.clone(),
        ))
    };

    // Build command handlers shared across all channels.
    let command_handlers: Vec<std::sync::Arc<dyn rara_kernel::channel::command::CommandHandler>> = {
        use rara_channels::telegram::commands::{
            BasicCommandHandler, DebugCommandHandler, McpCommandHandler, RenameCommandHandler,
            SessionCommandHandler, StatusCommandHandler, StopCommandHandler, TapeCommandHandler,
        };
        let tg_bot = telegram_adapter.as_ref().map(|a| a.bot());
        let session_handler =
            std::sync::Arc::new(SessionCommandHandler::new(bot_client.clone(), tg_bot));
        let stop_handler = std::sync::Arc::new(StopCommandHandler::new(
            bot_client.clone(),
            kernel_handle.clone(),
        ));
        let status_handler = std::sync::Arc::new(StatusCommandHandler::new(
            bot_client.clone(),
            kernel_handle.clone(),
        ));
        let rename_handler = std::sync::Arc::new(RenameCommandHandler::new(bot_client.clone()));
        let tape_handler = std::sync::Arc::new(TapeCommandHandler::new(bot_client.clone()));
        let debug_handler =
            std::sync::Arc::new(DebugCommandHandler::new(rara.tape_service.clone()));
        // Collect all command definitions so /help can list them.
        use rara_kernel::channel::command::CommandHandler as _;
        let all_commands: Vec<rara_kernel::channel::command::CommandDefinition> = [
            session_handler.commands(),
            stop_handler.commands(),
            status_handler.commands(),
            rename_handler.commands(),
            tape_handler.commands(),
            debug_handler.commands(),
        ]
        .into_iter()
        .flatten()
        .collect();
        let basic_handler = std::sync::Arc::new(BasicCommandHandler::new(all_commands));
        let mcp_handler = std::sync::Arc::new(McpCommandHandler::new(bot_client.clone()));
        vec![
            basic_handler,
            session_handler,
            stop_handler,
            status_handler,
            rename_handler,
            tape_handler,
            debug_handler,
            mcp_handler,
        ]
    };

    if let Some(ref tg_adapter) = telegram_adapter {
        tg_adapter.set_command_handlers(command_handlers.clone());

        // Register callback handlers for inline keyboard interactions.
        {
            use rara_channels::telegram::commands::{
                ModelSwitchCallbackHandler, SessionDeleteCallbackHandler,
                SessionDeleteCancelHandler, SessionDeleteConfirmHandler,
                SessionDetailCallbackHandler, SessionSwitchCallbackHandler,
                StatusJobsCallbackHandler,
            };
            let callback_handlers: Vec<
                std::sync::Arc<dyn rara_kernel::channel::command::CallbackHandler>,
            > = vec![
                std::sync::Arc::new(SessionSwitchCallbackHandler::new(bot_client.clone())),
                std::sync::Arc::new(SessionDetailCallbackHandler::new(bot_client.clone())),
                std::sync::Arc::new(SessionDeleteCallbackHandler::new(bot_client.clone())),
                std::sync::Arc::new(SessionDeleteConfirmHandler::new(bot_client.clone())),
                std::sync::Arc::new(SessionDeleteCancelHandler::new()),
                std::sync::Arc::new(ModelSwitchCallbackHandler::new(bot_client.clone())),
                std::sync::Arc::new(StatusJobsCallbackHandler::new(kernel_handle.clone())),
            ];
            tg_adapter.set_callback_handlers(callback_handlers);
        }

        use rara_kernel::channel::adapter::ChannelAdapter as _;
        match tg_adapter.start(kernel_handle.clone()).await {
            Ok(()) => info!("Telegram adapter started"),
            Err(e) => warn!(error = %e, "Failed to start Telegram adapter"),
        }
    }
    {
        use rara_kernel::channel::adapter::ChannelAdapter as _;
        match web_adapter.start(kernel_handle.clone()).await {
            Ok(()) => info!("WebAdapter started"),
            Err(e) => warn!(error = %e, "Failed to start WebAdapter"),
        }
    }
    if let Some(ref wc) = wechat_adapter {
        use rara_kernel::channel::adapter::ChannelAdapter as _;
        match wc.start(kernel_handle.clone()).await {
            Ok(()) => info!("WeChat adapter started"),
            Err(e) => warn!(error = %e, "Failed to start WeChat adapter"),
        }
    }
    info!("Kernel I/O subsystem running");

    // Start web frontend dev server (bun run dev) if web/ exists.
    if let Some(web_port) = config.http.web_port {
        let web_dir = PathBuf::from("web");
        let web_cancel = cancellation_token.clone();
        tokio::spawn(async move {
            web_server::start_web_server(web_dir, web_port, web_cancel).await;
        });
    }

    info!("Application started successfully");

    let app_handle = AppHandle {
        shutdown_tx: Some(shutdown_tx),
        running: Arc::clone(&running),
        cancellation_token: cancellation_token.clone(),
        kernel_handle: Some(kernel_handle),
        command_handlers,
        user_question_manager: Some(rara.user_question_manager.clone()),
    };

    let running_clone = Arc::clone(&running);
    let ct_clone = cancellation_token.clone();

    tokio::spawn(async move {
        shutdown_signal(shutdown_rx).await;
        running_clone.store(false, Ordering::SeqCst);
        ct_clone.cancel();

        if let Some(adapter) = telegram_adapter {
            use rara_kernel::channel::adapter::ChannelAdapter as _;
            info!("Shutting down Telegram adapter");
            let _ = adapter.stop().await;
        }
        {
            use rara_kernel::channel::adapter::ChannelAdapter as _;
            info!("Shutting down WebAdapter");
            let _ = web_adapter.stop().await;
        }

        info!("Shutting down servers");
        grpc_handle.shutdown();
        http_handle.shutdown();
        info!("Application shutdown complete");
    });

    Ok(app_handle)
}

async fn try_build_telegram(
    settings_svc: &rara_backend_admin::settings::SettingsSvc,
    user_question_manager: rara_kernel::user_question::UserQuestionManagerRef,
    stt_service: Option<rara_stt::SttService>,
    tts_service: Option<rara_tts::TtsService>,
) -> Result<Option<Arc<rara_channels::telegram::TelegramAdapter>>, Whatever> {
    use rara_domain_shared::settings::{SettingsProvider, keys};

    fn parse_group_policy(raw: Option<String>) -> GroupPolicy {
        raw.and_then(|s| {
            s.trim()
                .parse::<GroupPolicy>()
                .map_err(|e| warn!(error = %e, "invalid telegram.group_policy, using default"))
                .ok()
        })
        .unwrap_or_default()
    }

    let settings: Arc<dyn SettingsProvider> = Arc::new(settings_svc.clone());
    let token = match settings.get(keys::TELEGRAM_BOT_TOKEN).await {
        Some(t) if !t.is_empty() => t,
        _ => return Ok(None),
    };
    let proxy = std::env::var("HTTPS_PROXY")
        .or_else(|_| std::env::var("HTTP_PROXY"))
        .or_else(|_| std::env::var("ALL_PROXY"))
        .ok()
        .filter(|v| !v.is_empty());
    if let Some(ref p) = proxy {
        info!(proxy = %p, "telegram adapter: using proxy");
    }

    let chat_id: Option<i64> = settings
        .get(keys::TELEGRAM_CHAT_ID)
        .await
        .and_then(|v| v.parse().ok());
    let group_id: Option<i64> = settings
        .get(keys::TELEGRAM_ALLOWED_GROUP_CHAT_ID)
        .await
        .and_then(|v| v.parse().ok());
    let group_policy = parse_group_policy(settings.get(keys::TELEGRAM_GROUP_POLICY).await);

    let mut tg_config = rara_channels::telegram::TelegramConfig::default();
    tg_config.primary_chat_id = chat_id;
    tg_config.allowed_group_chat_id = group_id;
    tg_config.group_policy = group_policy;

    let adapter = Arc::new(
        rara_channels::telegram::TelegramAdapter::with_proxy(
            &token,
            vec![],
            proxy.as_deref(),
            Arc::clone(&settings),
        )
        .whatever_context("failed to build telegram adapter")?
        .with_config(tg_config)
        .with_user_question_manager(user_question_manager)
        .with_stt_service(stt_service)
        .with_tts_service(tts_service),
    );

    let config_handle = adapter.config_handle();
    let mut settings_rx = settings.subscribe();
    tokio::spawn(async move {
        while settings_rx.changed().await.is_ok() {
            let new_chat_id: Option<i64> = settings
                .get(keys::TELEGRAM_CHAT_ID)
                .await
                .and_then(|v| v.parse().ok());
            let new_group_id: Option<i64> = settings
                .get(keys::TELEGRAM_ALLOWED_GROUP_CHAT_ID)
                .await
                .and_then(|v| v.parse().ok());
            let new_group_policy =
                parse_group_policy(settings.get(keys::TELEGRAM_GROUP_POLICY).await);
            let mut cfg = config_handle.write().unwrap_or_else(|e| e.into_inner());
            cfg.primary_chat_id = new_chat_id;
            cfg.allowed_group_chat_id = new_group_id;
            cfg.group_policy = new_group_policy;
        }
    });

    Ok(Some(adapter))
}

async fn try_build_wechat(
    settings_svc: &rara_backend_admin::settings::SettingsSvc,
) -> Result<Option<Arc<rara_channels::wechat::WechatAdapter>>, Whatever> {
    use rara_channels::wechat::storage;
    use rara_domain_shared::settings::{SettingsProvider, keys};

    let settings: Arc<dyn SettingsProvider> = Arc::new(settings_svc.clone());

    // Prefer filesystem credentials (written by login) over settings store.
    let account_id = match storage::get_account_ids() {
        Ok(ids) if !ids.is_empty() => {
            info!(
                account_id = %ids[0],
                "wechat account_id resolved from saved credentials"
            );
            ids.into_iter().next().expect("non-empty")
        }
        _ => match settings.get(keys::WECHAT_ACCOUNT_ID).await {
            Some(id) if !id.is_empty() => id,
            _ => return Ok(None),
        },
    };

    // Read base_url from the persisted AccountData first (login writes it there),
    // then fall back to settings, then to the default.
    let fs_base_url = storage::get_account_data(&account_id)
        .ok()
        .map(|d| d.base_url)
        .filter(|u| !u.is_empty());
    let base_url = match fs_base_url {
        Some(url) => url,
        None => settings
            .get(keys::WECHAT_BASE_URL)
            .await
            .unwrap_or_else(|| storage::DEFAULT_BASE_URL.to_string()),
    };

    let adapter = Arc::new(
        rara_channels::wechat::WechatAdapter::new(account_id, base_url)
            .whatever_context("failed to build wechat adapter")?,
    );

    Ok(Some(adapter))
}

/// Diesel-embedded SQLite migrations, shipped inside the binary at build time.
const EMBEDDED_MIGRATIONS: diesel_migrations::EmbeddedMigrations =
    diesel_migrations::embed_migrations!("../rara-model/migrations");

/// Validate that [`AppConfig::owner_token`] is non-empty and
/// [`AppConfig::owner_user_id`] references a configured user whose role is
/// `root` or `admin`.
///
/// Exposed to tests via `pub(crate)` so we can assert failure shape without
/// booting the whole kernel.
pub(crate) fn validate_owner_auth(config: &AppConfig) -> Result<(), Whatever> {
    snafu::ensure_whatever!(
        !config.owner_token.trim().is_empty(),
        "owner_token must not be empty"
    );
    snafu::ensure_whatever!(
        !config.owner_user_id.trim().is_empty(),
        "owner_user_id must not be empty"
    );
    let Some(user) = config.users.iter().find(|u| u.name == config.owner_user_id) else {
        snafu::whatever!(
            "owner_user_id '{}' does not match any entry in users[]",
            config.owner_user_id
        );
    };
    let role = user.role.to_lowercase();
    snafu::ensure_whatever!(
        matches!(role.as_str(), "root" | "admin"),
        "owner_user_id '{}' must have role root or admin (got '{}')",
        config.owner_user_id,
        user.role
    );
    info!(
        owner = %config.owner_user_id,
        role = %user.role,
        "owner auth validated"
    );
    Ok(())
}

async fn init_infra(config: &AppConfig) -> Result<DBStore, Whatever> {
    let db_dir = rara_paths::database_dir();
    std::fs::create_dir_all(db_dir).whatever_context("Failed to create database directory")?;
    let database_url = format!("{}/rara.db", db_dir.display());

    // Run migrations on a synchronous SqliteConnection before any async
    // pool sees the DB — diesel_migrations' `MigrationHarness` trait is
    // implemented on the blocking driver, so we open a one-shot connection
    // on a blocking task.
    let migrate_url = database_url.clone();
    tokio::task::spawn_blocking(move || {
        use diesel::Connection;
        use diesel_migrations::MigrationHarness;
        let mut conn = diesel::SqliteConnection::establish(&migrate_url)
            .map_err(|e| anyhow::anyhow!("open sqlite for migrations: {e}"))?;
        conn.run_pending_migrations(EMBEDDED_MIGRATIONS)
            .map_err(|e| anyhow::anyhow!("run pending migrations: {e}"))?;
        Ok::<_, anyhow::Error>(())
    })
    .await
    .whatever_context("migration task join failed")?
    .whatever_context("Failed to run database migrations")?;

    let db_store = config
        .database
        .open(&database_url)
        .await
        .whatever_context("Failed to initialize database")?;

    info!("Database initialized");
    Ok(db_store)
}

// ---------------------------------------------------------------------------
// AppHandle
// ---------------------------------------------------------------------------

/// Handle for controlling a running application.
#[allow(dead_code)]
pub struct AppHandle {
    shutdown_tx:               Option<oneshot::Sender<()>>,
    running:                   Arc<AtomicBool>,
    cancellation_token:        CancellationToken,
    /// Kernel handle (for injecting inbound messages, accessing stream hub,
    /// endpoint registry, etc.).
    pub kernel_handle:         Option<rara_kernel::handle::KernelHandle>,
    /// Command handlers shared across all channels (Telegram, CLI, etc.).
    pub command_handlers: Vec<std::sync::Arc<dyn rara_kernel::channel::command::CommandHandler>>,
    /// User question manager for the ask-user tool (CLI needs it to subscribe
    /// and resolve agent questions).
    pub user_question_manager: Option<rara_kernel::user_question::UserQuestionManagerRef>,
}

#[allow(dead_code)]
impl AppHandle {
    /// Gracefully shutdown the application.
    pub fn shutdown(&mut self) {
        info!("Initiating graceful shutdown");
        self.running.store(false, Ordering::SeqCst);
        self.cancellation_token.cancel();

        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }

    /// Check if the application is still running.
    #[must_use]
    pub fn is_running(&self) -> bool { self.running.load(Ordering::SeqCst) }

    /// Wait for the application to shutdown.
    pub async fn wait_for_shutdown(&self) { self.cancellation_token.cancelled().await; }
}

async fn shutdown_signal(shutdown_rx: oneshot::Receiver<()>) {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => { info!("Received Ctrl+C signal"); },
        () = terminate => { info!("Received terminate signal"); },
        _ = shutdown_rx => { info!("Received shutdown signal"); },
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::AppConfig;

    const BASE_YAML: &str = r#"
http:
  bind_address: "127.0.0.1:25555"
grpc:
  bind_address: "127.0.0.1:50051"
  server_address: "127.0.0.1:50051"
owner_token: "test-owner-token"
owner_user_id: "test"
users:
  - name: test
    role: root
    platforms: []
mita:
  heartbeat_interval: "30m"
"#;

    #[test]
    fn app_config_loads_from_global_fallback_when_local_is_missing() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let global = tmp.path().join("global-config.yaml");
        let local = tmp.path().join("config.yaml");
        fs::write(&global, BASE_YAML).expect("write global config");

        let config = AppConfig::load_from_paths(&global, &local).expect("load config");
        assert_eq!(config.http.bind_address, "127.0.0.1:25555");
    }

    #[test]
    fn validate_owner_auth_accepts_admin_user() {
        let cfg: AppConfig = serde_yaml::from_str(BASE_YAML).expect("base yaml");
        super::validate_owner_auth(&cfg).expect("valid");
    }

    #[test]
    fn validate_owner_auth_rejects_missing_user() {
        let yaml = BASE_YAML.replace(r#"owner_user_id: "test""#, r#"owner_user_id: "ghost""#);
        let cfg: AppConfig = serde_yaml::from_str(&yaml).expect("yaml");
        let err = super::validate_owner_auth(&cfg).expect_err("ghost user");
        assert!(
            err.to_string().contains("does not match any entry"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn validate_owner_auth_rejects_non_admin_role() {
        let yaml = BASE_YAML.replace("role: root", "role: user");
        let cfg: AppConfig = serde_yaml::from_str(&yaml).expect("yaml");
        let err = super::validate_owner_auth(&cfg).expect_err("user role");
        assert!(
            err.to_string().contains("must have role root or admin"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn validate_owner_auth_rejects_empty_token() {
        let yaml = BASE_YAML.replace(r#"owner_token: "test-owner-token""#, r#"owner_token: """#);
        let cfg: AppConfig = serde_yaml::from_str(&yaml).expect("yaml");
        let err = super::validate_owner_auth(&cfg).expect_err("empty token");
        assert!(
            err.to_string().contains("owner_token must not be empty"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn telemetry_pyroscope_section_absent_yields_none() {
        // Default: no `telemetry:` section at all → pyroscope disabled, no
        // agent constructed, zero overhead at startup.
        let cfg: AppConfig = serde_yaml::from_str(BASE_YAML).expect("base yaml");
        assert!(cfg.telemetry.pyroscope.is_none());
        assert!(cfg.telemetry.env.is_none());
    }

    #[test]
    fn telemetry_pyroscope_section_parses_required_fields() {
        let yaml = format!(
            "{BASE_YAML}\n\
telemetry:\n  \
  env: \"prod\"\n  \
  pyroscope:\n    \
    enabled: true\n    \
    endpoint: \"http://10.0.0.183:4040\"\n    \
    application_name: \"rara\"\n    \
    sample_rate: 100\n"
        );
        let cfg: AppConfig = serde_yaml::from_str(&yaml).expect("yaml");
        let pyro = cfg.telemetry.pyroscope.expect("pyroscope section parsed");
        assert!(pyro.enabled);
        assert_eq!(pyro.endpoint, "http://10.0.0.183:4040");
        assert_eq!(pyro.application_name, "rara");
        assert_eq!(pyro.sample_rate, 100);
        assert_eq!(cfg.telemetry.env.as_deref(), Some("prod"));
    }

    #[test]
    fn telemetry_pyroscope_disabled_still_parses() {
        // `enabled: false` is the documented opt-in-by-default-off shape.
        let yaml = format!(
            "{BASE_YAML}\n\
telemetry:\n  \
  pyroscope:\n    \
    enabled: false\n    \
    endpoint: \"http://localhost:4040\"\n    \
    application_name: \"rara\"\n    \
    sample_rate: 100\n"
        );
        let cfg: AppConfig = serde_yaml::from_str(&yaml).expect("yaml");
        let pyro = cfg.telemetry.pyroscope.expect("section present");
        assert!(!pyro.enabled);
    }

    #[test]
    fn resolve_config_path_prefers_local() {
        let cwd_dir = tempfile::tempdir().expect("cwd tempdir");
        let xdg_dir = tempfile::tempdir().expect("xdg tempdir");
        let local = cwd_dir.path().join("config.yaml");
        let xdg = xdg_dir.path().join("config.yaml");
        fs::write(&local, "stub: true\n").expect("write local");
        fs::write(&xdg, "stub: true\n").expect("write xdg");

        let resolved = super::resolve_config_path_from(cwd_dir.path(), &xdg);
        assert_eq!(resolved, local);
    }

    #[test]
    fn config_file_sync_failure_message_names_resolved_path() {
        // When ConfigFileSync::new fails, start_with_options wraps the
        // error with this message so operators reading the panic / log
        // can identify which file is actually missing rather than guessing
        // between $CWD/config.yaml and the XDG path.
        let path = std::path::PathBuf::from("/tmp/nope/config.yaml");
        let msg = super::config_file_sync_failure_message(&path);
        assert!(
            msg.contains("/tmp/nope/config.yaml"),
            "message must name the resolved path, got: {msg}"
        );
        assert!(
            msg.contains("config file sync"),
            "message must identify the failing subsystem, got: {msg}"
        );
    }

    #[test]
    fn resolve_config_path_falls_back_to_xdg() {
        let cwd_dir = tempfile::tempdir().expect("cwd tempdir");
        let xdg_dir = tempfile::tempdir().expect("xdg tempdir");
        let xdg = xdg_dir.path().join("config.yaml");
        fs::write(&xdg, "stub: true\n").expect("write xdg");
        // Note: no local config.yaml under cwd_dir.

        let resolved = super::resolve_config_path_from(cwd_dir.path(), &xdg);
        assert_eq!(resolved, xdg);
    }

    #[test]
    fn app_config_prefers_local_override_over_global() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let global = tmp.path().join("global-config.yaml");
        let local = tmp.path().join("config.yaml");
        fs::write(&global, BASE_YAML).expect("write global config");
        fs::write(
            &local,
            BASE_YAML.replace("127.0.0.1:25555", "127.0.0.1:35555"),
        )
        .expect("write local config");

        let config = AppConfig::load_from_paths(&global, &local).expect("load config");
        assert_eq!(config.http.bind_address, "127.0.0.1:35555");
    }

    #[test]
    fn telemetry_otlp_defaults_to_disabled() {
        let cfg: AppConfig = serde_yaml::from_str(BASE_YAML).expect("base yaml");
        assert!(
            cfg.telemetry.otlp.is_none(),
            "no `telemetry.otlp` block should leave it unset"
        );
        assert!(cfg.telemetry.otlp_endpoint.is_none());
    }

    #[test]
    fn telemetry_otlp_parses_full_block() {
        let yaml = format!(
            r#"{BASE_YAML}
telemetry:
  otlp:
    enabled: true
    traces_endpoint: "http://10.0.0.183:3000/api/public/otel/v1/traces"
    deployment_environment: "dev"
    headers:
      authorization: "Basic ZGVtbw=="
"#
        );
        let cfg: AppConfig = serde_yaml::from_str(&yaml).expect("yaml");
        let otlp = cfg.telemetry.otlp.expect("otlp block");
        assert_eq!(otlp.enabled, Some(true));
        assert_eq!(
            otlp.traces_endpoint.as_deref(),
            Some("http://10.0.0.183:3000/api/public/otel/v1/traces")
        );
        assert_eq!(otlp.deployment_environment.as_deref(), Some("dev"));
        assert_eq!(
            otlp.headers.get("authorization").map(String::as_str),
            Some("Basic ZGVtbw==")
        );
    }

    #[test]
    fn telemetry_otlp_disabled_block_parses() {
        let yaml = format!(
            r#"{BASE_YAML}
telemetry:
  otlp:
    enabled: false
    traces_endpoint: "http://example.invalid/v1/traces"
"#
        );
        let cfg: AppConfig = serde_yaml::from_str(&yaml).expect("yaml");
        let otlp = cfg.telemetry.otlp.expect("otlp block");
        assert_eq!(otlp.enabled, Some(false));
        assert!(otlp.headers.is_empty());
    }

    #[test]
    fn minimal_required_yaml_parses_with_all_defaults() {
        // Issue #1913: every truly-optional top-level field must default
        // cleanly. `BASE_YAML` only contains the required fields plus
        // `http`/`grpc` (which themselves now have `#[serde(default)]`).
        // Drop them to verify the defaults actually fire.
        const MINIMAL: &str = r#"
owner_token: "x"
owner_user_id: "test"
users:
  - name: test
    role: root
    platforms: []
mita:
  heartbeat_interval: "30m"
"#;
        let cfg: AppConfig = serde_yaml::from_str(MINIMAL).expect("minimal yaml");
        // SmartDefault on RestServerConfig / GrpcServerConfig
        assert_eq!(cfg.http.bind_address, "127.0.0.1:25555");
        assert_eq!(cfg.grpc.bind_address, "127.0.0.1:50051");
        assert_eq!(cfg.max_ingress_per_minute, 30);
        assert!(cfg.llm.is_none());
        assert!(cfg.sandbox.is_none());
    }

    #[test]
    fn missing_required_field_error_points_at_example_line() {
        // Drop the required `mita` block to trigger a `missing field` error.
        let yaml = r#"
http:
  bind_address: "127.0.0.1:25555"
grpc:
  bind_address: "127.0.0.1:50051"
  server_address: "127.0.0.1:50051"
owner_token: "x"
owner_user_id: "test"
users:
  - name: test
    role: root
    platforms: []
"#;
        let dir = tempfile::tempdir().expect("tempdir");
        let local = dir.path().join("config.yaml");
        let global = dir.path().join("global.yaml");
        fs::write(&local, yaml).expect("write local");

        let err =
            AppConfig::load_from_paths(&global, &local).expect_err("missing mita block must fail");
        let msg = err.to_string();
        assert!(
            msg.contains("missing required field `mita`"),
            "expected friendly error, got: {msg}"
        );
        assert!(
            msg.contains("config.example.yaml line"),
            "expected example pointer, got: {msg}"
        );
        assert!(
            msg.contains(local.to_string_lossy().as_ref()),
            "expected config path in error, got: {msg}"
        );
    }

    #[test]
    fn remote_config_shape_parses_cleanly() {
        // Smoke test guarding against #1907-style regressions: the shape
        // currently deployed on the rara host (no `web`, no `sandbox`)
        // MUST keep parsing after schema additions.
        const REMOTE_SHAPE: &str = r#"
database:
  max_connections: 5
http:
  bind_address: "127.0.0.1:25555"
  cors_allowed_origins: ["http://localhost:5173"]
grpc:
  bind_address: "127.0.0.1:50051"
  server_address: "127.0.0.1:50051"
telemetry:
  otlp_endpoint: "http://alloy:4318/v1/traces"
owner_token: "redacted"
owner_user_id: "rara"
llm:
  default_provider: "openrouter"
  providers:
    openrouter:
      base_url: "https://openrouter.ai/api/v1"
      api_key: "sk-or-..."
      default_model: "anthropic/claude-3.5-sonnet"
telegram:
  bot_token: "x"
  chat_id: "1"
  group_policy: "mention_or_small_group"
wechat:
  account_id: "x"
  base_url: "https://api.example/v1"
users:
  - name: "rara"
    role: root
    platforms: []
max_ingress_per_minute: 60
mita:
  heartbeat_interval: "30m"
knowledge:
  embedding_model: "text-embedding-3-small"
  embedding_dimensions: 1536
  search_top_k: 10
  similarity_threshold: 0.85
stt:
  base_url: "http://localhost:9000"
gateway:
  repo_url: "https://github.com/rararulab/rara"
  bot_token: "x"
  chat_id: 1
"#;
        let cfg: AppConfig = serde_yaml::from_str(REMOTE_SHAPE).expect("remote shape parses");
        assert_eq!(cfg.owner_user_id, "rara");
        assert!(cfg.sandbox.is_none());
        assert!(cfg.gateway.is_some());
    }

    #[test]
    fn telemetry_otlp_missing_endpoint_parses_but_runtime_rejects() {
        // Endpoint is `Option<String>` so deserialization succeeds; the
        // bootstrap path in `rara-cli` is responsible for rejecting an
        // enabled-without-endpoint config at startup.
        let yaml = format!(
            r#"{BASE_YAML}
telemetry:
  otlp:
    enabled: true
"#
        );
        let cfg: AppConfig = serde_yaml::from_str(&yaml).expect("yaml");
        let otlp = cfg.telemetry.otlp.expect("otlp block");
        assert_eq!(otlp.enabled, Some(true));
        assert!(otlp.traces_endpoint.is_none());
    }
}
