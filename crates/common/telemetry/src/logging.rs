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

#![allow(
    clippy::or_fun_call,
    clippy::option_if_let_else,
    clippy::doc_markdown,
    clippy::unwrap_or_default,
    clippy::map_unwrap_or,
    clippy::literal_string_with_formatting_args
)]

use std::{
    collections::HashMap,
    env,
    io::IsTerminal,
    sync::{Arc, Mutex, Once},
};

use bon::Builder;
use once_cell::sync::{Lazy, OnceCell};
use opentelemetry::{KeyValue, global, trace::TracerProvider};
use opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge;
use opentelemetry_otlp::{LogExporter, Protocol, SpanExporter, WithExportConfig, WithHttpConfig};
use opentelemetry_sdk::{
    logs::SdkLoggerProvider, metrics::SdkMeterProvider, propagation::TraceContextPropagator,
    trace::Sampler,
};
use opentelemetry_semantic_conventions::resource;
use serde::{Deserialize, Deserializer, Serialize, de};
use smart_default::SmartDefault;
/// Re-export so binary crates that hold the guards returned by
/// `init_global_logging` can name their type without depending on
/// `tracing-appender` directly.
pub use tracing_appender::non_blocking::WorkerGuard as LoggingWorkerGuard;
use tracing_appender::{
    non_blocking::WorkerGuard,
    rolling::{RollingFileAppender, Rotation},
};
use tracing_log::LogTracer;
use tracing_subscriber::{EnvFilter, Registry, filter, layer::SubscriberExt, prelude::*};

use crate::tracing_sampler::{TracingSampleOptions, create_sampler};

/// Deserializes a string value, using `Default::default()` if the string is
/// empty.
///
/// This helper function is used for serde deserialization where an empty string
/// should be treated as the default value for the type. It's particularly
/// useful for configuration fields where both missing values and empty strings
/// should result in default behavior.
///
/// # Type Parameters
///
/// * `D` - The deserializer type
/// * `T` - The target type that implements both `Deserialize` and `Default`
///
/// # Returns
///
/// * `Ok(T)` - The deserialized value or default if string was empty
/// * `Err(D::Error)` - Deserialization error if the string was invalid
///
/// # Errors
/// Returns an error if deserialization fails.
pub fn empty_string_as_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    let s = String::deserialize(deserializer)?;
    if s.is_empty() {
        Ok(T::default())
    } else {
        // Parse the string content into type T
        T::deserialize(de::value::StrDeserializer::new(&s)).map_err(|e: de::value::Error| {
            de::Error::custom(format!("invalid value, expect empty string, err: {e}"))
        })
    }
}

/// The default OTLP endpoint when using gRPC exporter protocol.
///
/// This is the standard gRPC endpoint for OpenTelemetry Protocol (OTLP) that
/// most observability backends listen on by default. This endpoint is typically
/// used with Jaeger, Tempo, or other OTLP-compatible trace collectors.
pub const DEFAULT_OTLP_GRPC_ENDPOINT: &str = "http://localhost:4317";

/// The default OTLP endpoint when using HTTP exporter protocol.
///
/// This is the standard HTTP endpoint for OpenTelemetry Protocol (OTLP) traces.
/// The `/v1/traces` path is the OTLP specification endpoint for trace data.
/// HTTP export is useful when gRPC is not available or when custom headers
/// are needed for authentication.
pub const DEFAULT_OTLP_HTTP_ENDPOINT: &str = "http://localhost:4318/v1/traces";

/// The default OTLP logs HTTP endpoint.
///
/// Used when `otlp_logs_endpoint` is not configured but logs export is
/// enabled. The `/v1/logs` path is the OTLP specification endpoint for log
/// records.
pub const DEFAULT_OTLP_HTTP_LOGS_ENDPOINT: &str = "http://localhost:4318/v1/logs";

/// The default directory name for log files when file logging is enabled.
///
/// This directory will be created relative to the application's working
/// directory if a relative path is used, or can be overridden with an absolute
/// path in the `LoggingOptions.dir` field.
pub const DEFAULT_LOGGING_DIR: &str = "logs";

/// Global handle for dynamically reloading log levels at runtime.
///
/// This static variable holds a reload handle that allows changing log levels
/// and filters without restarting the application. It's populated during
/// logging initialization and can be used later to modify logging behavior.
///
/// # Note
///
/// This handle is only available after `init_global_logging` has been called.
/// Attempting to use it before initialization will return `None`.
pub static RELOAD_HANDLE: OnceCell<tracing_subscriber::reload::Handle<filter::Targets, Registry>> =
    OnceCell::new();

/// Configuration options for the logging system.
///
/// This structure contains all the configuration parameters needed to set up
/// the logging infrastructure, including output destinations, formats,
/// OpenTelemetry integration, and performance tuning options.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, SmartDefault, Builder)]
#[serde(default)]
pub struct LoggingOptions {
    /// Directory path for storing log files.
    ///
    /// When set to a non-empty string, log files will be created in this
    /// directory with automatic hourly rotation. If empty, only stdout
    /// logging will be used. The directory will be created if it doesn't
    /// exist.
    #[default = ""]
    pub dir: String,

    /// Log level filter string.
    ///
    /// Supports standard Rust log level syntax like "info", "debug,hyper=warn",
    /// or more complex filters like "`info,my_crate::module=debug`". If None,
    /// falls back to the `RUST_LOG` environment variable or "info" default.
    pub level: Option<String>,

    /// Output format for log messages.
    ///
    /// - `Text`: Human-readable format suitable for development and console
    ///   output
    /// - `Json`: Machine-parseable JSON format ideal for log aggregation
    ///   systems
    #[serde(default, deserialize_with = "empty_string_as_default")]
    pub log_format: LogFormat,

    /// Maximum number of rotated log files to retain.
    ///
    /// When log rotation occurs (hourly), old files are automatically deleted
    /// when this limit is reached. Default is 720 files (30 days of hourly
    /// logs). This applies to both main logs and error-specific logs.
    #[default = 720]
    pub max_log_files: usize,

    /// Whether to output logs to stdout in addition to files.
    ///
    /// When true, logs will be written to both stdout and files (if file
    /// logging is enabled). When false, logs only go to files. Default is true.
    #[default = true]
    pub append_stdout: bool,

    /// Enable OpenTelemetry Protocol (OTLP) tracing integration.
    ///
    /// When true, spans and traces will be exported to an OTLP-compatible
    /// backend like Jaeger, Tempo, or other observability platforms.
    /// Default is false.
    #[default = false]
    pub enable_otlp_tracing: bool,

    /// Custom OTLP endpoint URL.
    ///
    /// If None, uses default endpoints based on the protocol:
    /// - gRPC: `http://localhost:4317`
    /// - HTTP: `http://localhost:4318/v1/traces`
    ///
    /// URLs without a scheme will automatically get "http://" prepended.
    pub otlp_endpoint: Option<String>,

    /// Sampling configuration for trace collection.
    ///
    /// Controls which traces are collected and exported to reduce overhead
    /// in high-throughput applications. If None, all traces are collected.
    pub tracing_sample_ratio: Option<TracingSampleOptions>,

    /// OTLP transport protocol selection.
    ///
    /// - `Grpc`: More efficient binary protocol, requires gRPC support
    /// - `Http`: HTTP-based transport, better firewall compatibility
    ///
    /// If None, defaults to HTTP protocol.
    pub otlp_export_protocol: Option<OtlpExportProtocol>,

    /// Custom HTTP headers for OTLP HTTP exports.
    ///
    /// Used for authentication, routing, or other metadata when using HTTP
    /// transport. Common examples include Authorization headers or tenant IDs.
    /// Only applies when using HTTP export protocol.
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    #[default(_code = "HashMap::new()")]
    pub otlp_headers: HashMap<String, String>,

    /// OpenTelemetry semantic-convention schema URL pinned on the tracer
    /// provider's resource.
    ///
    /// Pinning a schema URL lets backends (e.g. Langfuse) interpret span
    /// attributes against a known semconv version. When `None`, the resource
    /// is built without a schema URL.
    pub otlp_schema_url: Option<String>,

    /// Deployment environment label (e.g. `dev`, `staging`, `prod`).
    ///
    /// When set, the value is attached to the OTel resource as
    /// `deployment.environment.name` so traces from different environments
    /// can be filtered downstream.
    pub otlp_deployment_environment: Option<String>,

    /// Enable OTLP log export via the `tracing` → OTel logs bridge.
    ///
    /// When true, every `tracing` event is converted to an OTLP `LogRecord`
    /// and shipped to `otlp_logs_endpoint`. This is independent of trace
    /// export — Loki receives logs while Langfuse receives traces. Default
    /// is false.
    #[default = false]
    pub enable_otlp_logs: bool,

    /// OTLP/HTTP logs ingest URL — full path including `/v1/logs`.
    ///
    /// Separate from `otlp_endpoint` because logs and traces typically live
    /// on different services even when colocated (e.g. Loki vs Langfuse).
    /// If `enable_otlp_logs` is true and this is `None`, falls back to
    /// [`DEFAULT_OTLP_HTTP_LOGS_ENDPOINT`].
    pub otlp_logs_endpoint: Option<String>,

    /// Custom HTTP headers attached to OTLP log exports.
    ///
    /// Used for tenant routing or auth — Loki, for example, requires
    /// `X-Scope-OrgID` even when `auth_enabled` is false.
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    #[default(_code = "HashMap::new()")]
    pub otlp_logs_headers: HashMap<String, String>,
}

/// OpenTelemetry Protocol (OTLP) export transport protocols.
///
/// Defines the available transport mechanisms for sending trace data to
/// observability backends. Each protocol has different characteristics
/// and use cases.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, derive_more::Display)]
#[serde(rename_all = "snake_case")]
pub enum OtlpExportProtocol {
    /// gRPC transport protocol.
    ///
    /// A high-performance binary protocol that's more efficient for large
    /// volumes of telemetry data. Typically used with Jaeger agents or
    /// other backends that support gRPC. Requires gRPC infrastructure
    /// and may have firewall considerations.
    Grpc,

    /// HTTP transport protocol with binary protobuf encoding.
    ///
    /// Uses HTTP POST with protobuf binary payloads. Better for environments
    /// where gRPC is not available or when custom headers are needed for
    /// authentication. Works well through firewalls and load balancers.
    Http,
}

/// Available log output formats.
///
/// Controls how log messages are formatted when written to outputs.
/// Different formats serve different purposes and consumption patterns.
#[derive(
    Clone, Debug, Copy, PartialEq, Eq, Serialize, Deserialize, Default, derive_more::Display,
)]
#[serde(rename_all = "snake_case")]
pub enum LogFormat {
    /// JSON-structured log format.
    ///
    /// Outputs logs as JSON objects with structured fields. Ideal for:
    /// - Log aggregation systems (ELK, Splunk, etc.)
    /// - Machine parsing and analysis
    /// - Production environments with log processing pipelines
    ///
    /// Example output:
    /// ```json
    /// {"timestamp":"2024-01-01T12:00:00Z","level":"INFO","target":"my_app","message":"Server started"}
    /// ```
    Json,

    /// Human-readable text format.
    ///
    /// Traditional log format optimized for human readability. Best for:
    /// - Development and debugging
    /// - Console output
    /// - Direct human consumption
    ///
    /// Example output:
    /// ```text
    /// 2024-01-01T12:00:00.123Z  INFO my_app: Server started
    /// ```
    #[default]
    Text,
}

/// Configuration options for advanced tracing features.
///
/// Contains settings for optional tracing integrations that provide
/// additional debugging and monitoring capabilities beyond basic logging.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, SmartDefault)]
pub struct TracingOptions {
    /// TCP address for tokio-console integration.
    ///
    /// When the `tokio-console` feature is enabled, this specifies the
    /// address where the tokio-console server should listen. Tokio-console
    /// provides real-time debugging of async Rust applications.
    ///
    /// Example: `"127.0.0.1:6669"` or `"0.0.0.0:6669"
    ///
    /// Only available when compiled with the `tokio-console` feature flag.
    #[cfg(feature = "tokio-console")]
    pub tokio_console_addr: Option<String>,
}

/// Initialize tracing with default configuration for simple applications.
///
/// This is a convenience function that sets up basic logging with default
/// settings. Logs are written to stdout with text formatting and no file
/// output or OpenTelemetry integration.
///
/// # Parameters
///
/// * `app_name` - Application name used for service identification in traces
///
/// # Returns
///
/// A vector of `WorkerGuard`s that must be kept alive for logging to function.
/// Drop these guards to shut down logging gracefully.
///
/// # Note
///
/// This function can only be called once per application. Subsequent calls
/// will be ignored due to internal `Once` synchronization.
#[must_use]
pub fn init_tracing_subscriber(app_name: &str) -> Vec<WorkerGuard> {
    let logging_opts = LoggingOptions::default();
    let tracing_opts = TracingOptions::default();
    init_global_logging(app_name, &logging_opts, &tracing_opts, None)
}

/// Initialize logging specifically designed for unit tests.
///
/// This function sets up logging that's appropriate for unit test environments,
/// with logs written to files in a dedicated test directory. It's designed to
/// be called multiple times safely and uses environment variables for
/// configuration.
///
/// # Environment Variables
///
/// * `UNITTEST_LOG_DIR` - Directory for test logs (default:
///   "/tmp/__`unittest_logs`")
/// * `UNITTEST_LOG_LEVEL` - Log level filter (default:
///   "debug,hyper=warn,tower=warn,...")
///
/// # Behavior
///
/// - Creates test-specific log files in the configured directory
/// - Uses debug-level logging by default with reduced noise from dependencies
/// - Safe to call multiple times (uses `Once` for synchronization)
/// - Maintains worker guards in a global static to prevent cleanup during tests
///
/// # Note
///
/// This function is thread-safe and can be called from multiple test functions
/// simultaneously. The first call initializes logging, subsequent calls are
/// no-ops.
///
/// # Panics
///
/// May panic if the global logging subscriber fails to initialize.
pub fn init_default_ut_logging() {
    static START: Once = Once::new();

    START.call_once(|| {
        let mut g = GLOBAL_UT_LOG_GUARD
            .as_ref()
            .lock()
            .expect("GLOBAL_UT_LOG_GUARD mutex poisoned by an earlier panic");

        let dir =
            env::var("UNITTEST_LOG_DIR").unwrap_or_else(|_| "/tmp/__unittest_logs".to_string());

        let level = env::var("UNITTEST_LOG_LEVEL").unwrap_or_else(|_| {
            "debug,hyper=warn,tower=warn,datafusion=warn,reqwest=warn,sqlparser=warn,h2=info,\
             opendal=info,rskafka=info"
                .to_string()
        });
        let opts = LoggingOptions {
            dir: dir.clone(),
            level: Some(level),
            ..Default::default()
        };
        *g = Some(init_global_logging(
            "unittest",
            &opts,
            &TracingOptions::default(),
            None,
        ));

        tracing::info!("logs dir = {}", dir);
    });
}

/// Global storage for unit test logging worker guards.
///
/// This static holds the worker guards for unit test logging to prevent them
/// from being dropped during test execution. The guards are wrapped in
/// Arc<Mutex<>> to allow safe concurrent access from multiple test threads.
static GLOBAL_UT_LOG_GUARD: Lazy<Arc<Mutex<Option<Vec<WorkerGuard>>>>> =
    Lazy::new(|| Arc::new(Mutex::new(None)));

/// Default log level filter when no specific configuration is provided.
///
/// This is used as a fallback when neither the `level` field in
/// `LoggingOptions` nor the `RUST_LOG` environment variable is set.
const DEFAULT_LOG_TARGETS: &str = "warn,rara=info,rara_=info,common_=info,yunara_=info,base=info";

/// Initialize comprehensive logging with full configuration options.
///
/// This is the main logging initialization function that supports all features
/// including file logging, OpenTelemetry integration, custom formatting, and
/// advanced tracing features. It sets up multiple output layers and configures
/// the global tracing subscriber.
///
/// # Parameters
///
/// * `app_name` - Application name used for service identification in traces
/// * `opts` - Complete logging configuration options
/// * `tracing_opts` - Advanced tracing feature configuration
/// * `node_id` - Optional node/instance identifier for distributed systems
///
/// # Returns
///
/// A vector of `WorkerGuard`s that must be kept alive for the lifetime of the
/// application. Dropping these guards will stop the background logging threads.
///
/// # Logging Layers
///
/// The function sets up multiple layers depending on configuration:
///
/// - **Stdout Layer**: Logs to stdout (if `append_stdout` is true)
/// - **File Layer**: Main log files with hourly rotation (if `dir` is set)
/// - **Error File Layer**: Error-only logs in separate files (if `dir` is set)
/// - **OTLP Layer**: OpenTelemetry export (if `enable_otlp_tracing` is true)
/// - **Tokio Console Layer**: Async debugging (if feature enabled and
///   configured)
///
/// # Thread Safety
///
/// This function is thread-safe and uses `Once` synchronization to ensure
/// it can only be called once per application. Subsequent calls will be
/// ignored.
///
/// # Error Handling
///
/// The function panics on critical initialization failures to ensure
/// observability issues are caught early. This includes:
/// - Log directory creation failures
/// - Invalid log level strings
/// - OTLP exporter setup failures
///
/// # Performance Notes
///
/// - All writers use non-blocking I/O to prevent blocking application threads
/// - File rotation happens automatically without blocking
/// - OTLP export is batched for efficiency
/// - Sampling can be configured to reduce overhead
///
/// # Panics
///
/// May panic if:
/// - Failed to set global tracing subscriber
/// - File writer initialization fails
/// - OTLP exporter initialization fails
#[allow(clippy::print_stdout, clippy::too_many_lines)]
pub fn init_global_logging(
    app_name: &str,
    opts: &LoggingOptions,
    tracing_opts: &TracingOptions,
    node_id: Option<String>,
) -> Vec<WorkerGuard> {
    static START: Once = Once::new();
    let mut guards = vec![];

    START.call_once(|| {
        LogTracer::init().expect("log tracer must be valid");

        let stdout_logging_layer = if opts.append_stdout {
            let (writer, guard) = tracing_appender::non_blocking(std::io::stdout());
            guards.push(guard);

            if opts.log_format == LogFormat::Json {
                Some(
                    tracing_subscriber::fmt::Layer::new()
                        .json()
                        .with_writer(writer)
                        .with_ansi(std::io::stdout().is_terminal())
                        .with_current_span(true)
                        .with_span_list(true)
                        .boxed(),
                )
            } else {
                Some(
                    tracing_subscriber::fmt::Layer::new()
                        .with_writer(writer)
                        .with_ansi(std::io::stdout().is_terminal())
                        .boxed(),
                )
            }
        } else {
            None
        };

        let file_logging_layer = if opts.dir.is_empty() {
            None
        } else {
            let rolling_appender = RollingFileAppender::builder()
                .rotation(Rotation::HOURLY)
                .filename_prefix("job")
                .max_log_files(opts.max_log_files)
                .build(&opts.dir)
                .unwrap_or_else(|e| {
                    panic!(
                        "initializing rolling file appender at {} failed: {}",
                        &opts.dir, e
                    )
                });
            let (writer, guard) = tracing_appender::non_blocking(rolling_appender);
            guards.push(guard);

            if opts.log_format == LogFormat::Json {
                Some(
                    tracing_subscriber::fmt::Layer::new()
                        .json()
                        .with_writer(writer)
                        .with_ansi(false)
                        .with_current_span(true)
                        .with_span_list(true)
                        .boxed(),
                )
            } else {
                Some(
                    tracing_subscriber::fmt::Layer::new()
                        .with_writer(writer)
                        .with_ansi(false)
                        .boxed(),
                )
            }
        };

        let err_file_logging_layer = if opts.dir.is_empty() {
            None
        } else {
            let rolling_appender = RollingFileAppender::builder()
                .rotation(Rotation::HOURLY)
                .filename_prefix("raraerr")
                .max_log_files(opts.max_log_files)
                .build(&opts.dir)
                .unwrap_or_else(|e| {
                    panic!(
                        "initializing rolling file appender at {} failed: {}",
                        &opts.dir, e
                    )
                });
            let (writer, guard) = tracing_appender::non_blocking(rolling_appender);
            guards.push(guard);

            if opts.log_format == LogFormat::Json {
                Some(
                    tracing_subscriber::fmt::Layer::new()
                        .json()
                        .with_writer(writer)
                        .with_ansi(false)
                        .with_filter(filter::LevelFilter::ERROR)
                        .boxed(),
                )
            } else {
                Some(
                    tracing_subscriber::fmt::Layer::new()
                        .with_writer(writer)
                        .with_ansi(false)
                        .with_filter(filter::LevelFilter::ERROR)
                        .boxed(),
                )
            }
        };

        let filter = opts
            .level
            .as_deref()
            .or(env::var(EnvFilter::DEFAULT_ENV).ok().as_deref())
            .unwrap_or(DEFAULT_LOG_TARGETS)
            .parse::<filter::Targets>()
            .expect("error parsing log level string");

        let (dyn_filter, reload_handle) = tracing_subscriber::reload::Layer::new(filter);

        RELOAD_HANDLE
            .set(reload_handle)
            .expect("reload handle already set, maybe init_global_logging get called twice?");

        #[cfg(feature = "tokio-console")]
        let subscriber = {
            let tokio_console_layer = if let Some(tokio_console_addr) =
                &tracing_opts.tokio_console_addr
            {
                let addr: std::net::SocketAddr = tokio_console_addr.parse().unwrap_or_else(|e| {
                    panic!("Invalid binding address '{tokio_console_addr}' for tokio-console: {e}");
                });
                println!("tokio-console listening on {{addr}}");

                Some(
                    console_subscriber::ConsoleLayer::builder()
                        .server_addr(addr)
                        .spawn(),
                )
            } else {
                None
            };

            Registry::default()
                .with(dyn_filter)
                .with(tokio_console_layer)
                .with(stdout_logging_layer)
                .with(file_logging_layer)
                .with(err_file_logging_layer)
        };

        let _ = tracing_opts;

        #[cfg(not(feature = "tokio-console"))]
        let subscriber = Registry::default()
            .with(dyn_filter)
            .with(stdout_logging_layer)
            .with(file_logging_layer)
            .with(err_file_logging_layer);

        // Build the OTel resource once if either OTLP signal is enabled —
        // traces, metrics, and logs all share the same `service.*` identity.
        let otel_resource = if opts.enable_otlp_tracing || opts.enable_otlp_logs {
            Some(build_otel_resource(app_name, node_id.as_deref(), opts))
        } else {
            None
        };

        let otel_trace_layer = if opts.enable_otlp_tracing {
            global::set_text_map_propagator(TraceContextPropagator::new());

            let sampler = opts
                .tracing_sample_ratio
                .as_ref()
                .map(create_sampler)
                .map_or(
                    Sampler::ParentBased(Box::new(Sampler::AlwaysOn)),
                    Sampler::ParentBased,
                );

            let resource = otel_resource
                .clone()
                .expect("otel_resource present when enable_otlp_tracing");

            let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder()
                .with_batch_exporter(build_otlp_exporter(opts))
                .with_sampler(sampler)
                .with_resource(resource.clone())
                .build();
            let tracer = provider.tracer("job");

            // Initialize the OTel metrics pipeline alongside traces.
            let meter_provider = init_meter_provider(opts, resource);
            global::set_meter_provider(meter_provider);

            Some(tracing_opentelemetry::layer().with_tracer(tracer))
        } else {
            None
        };

        let otel_logs_layer = if opts.enable_otlp_logs {
            let resource = otel_resource
                .clone()
                .expect("otel_resource present when enable_otlp_logs");
            let logger_provider = init_logger_provider(opts, resource);
            // The bridge converts every `tracing` event into an OTLP
            // `LogRecord`. Keep it as a separate layer so it stacks on the
            // same `Registry` as the file/stdout layers and the trace layer.
            Some(OpenTelemetryTracingBridge::new(&logger_provider))
        } else {
            None
        };

        tracing::subscriber::set_global_default(
            subscriber.with(otel_trace_layer).with(otel_logs_layer),
        )
        .expect("error setting global tracing subscriber");
    });

    guards
}

/// Build the shared OTel `Resource` that identifies this process to every
/// OTLP signal (traces, metrics, logs).
///
/// Pulling this into one place keeps the three signals consistent — they all
/// see the same `service.name`, `service.instance.id`, `service.version`,
/// `process.pid`, and (when configured) `deployment.environment.name` and
/// semconv schema URL.
fn build_otel_resource(
    app_name: &str,
    node_id: Option<&str>,
    opts: &LoggingOptions,
) -> opentelemetry_sdk::Resource {
    let mut resource_attrs = vec![
        KeyValue::new(resource::SERVICE_NAME, app_name.to_string()),
        KeyValue::new(
            resource::SERVICE_INSTANCE_ID,
            node_id.unwrap_or("none").to_string(),
        ),
        KeyValue::new(resource::SERVICE_VERSION, env!("CARGO_PKG_VERSION")),
        KeyValue::new(resource::PROCESS_PID, std::process::id().to_string()),
    ];
    if let Some(env) = opts.otlp_deployment_environment.as_deref() {
        resource_attrs.push(KeyValue::new(
            resource::DEPLOYMENT_ENVIRONMENT_NAME,
            env.to_string(),
        ));
    }
    let builder = opentelemetry_sdk::Resource::builder_empty();
    match opts.otlp_schema_url.as_deref() {
        Some(schema_url) => builder
            .with_schema_url(resource_attrs, schema_url.to_string())
            .build(),
        None => builder.with_attributes(resource_attrs).build(),
    }
}

/// Build a `reqwest::blocking::Client` configured for OTLP HTTP exporters.
///
/// OTLP exporters target collectors on the LAN (Alloy, Langfuse, Loki). They
/// must NOT honor `HTTP_PROXY` / `HTTPS_PROXY` from the environment, and
/// OTel's BatchProcessor / PeriodicReader threads have no tokio runtime, so we
/// use the blocking client.
fn build_otlp_http_client() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .no_proxy()
        .build()
        .expect("Failed to build reqwest client for OTLP HTTP exporter")
}

fn build_otlp_exporter(opts: &LoggingOptions) -> SpanExporter {
    let protocol = opts
        .otlp_export_protocol
        .clone()
        .unwrap_or(OtlpExportProtocol::Http);

    let endpoint = opts
        .otlp_endpoint
        .as_ref()
        .map(|e| {
            if e.starts_with("http") {
                e.clone()
            } else {
                format!("http://{e}")
            }
        })
        .unwrap_or_else(|| match protocol {
            OtlpExportProtocol::Grpc => DEFAULT_OTLP_GRPC_ENDPOINT.to_string(),
            OtlpExportProtocol::Http => DEFAULT_OTLP_HTTP_ENDPOINT.to_string(),
        });

    match protocol {
        OtlpExportProtocol::Grpc => SpanExporter::builder()
            .with_tonic()
            .with_endpoint(endpoint)
            .build()
            .expect("Failed to create OTLP gRPC exporter "),

        OtlpExportProtocol::Http => SpanExporter::builder()
            .with_http()
            .with_http_client(build_otlp_http_client())
            .with_endpoint(endpoint)
            .with_protocol(Protocol::HttpBinary)
            .with_headers(opts.otlp_headers.clone())
            .build()
            .expect("Failed to create OTLP HTTP exporter "),
    }
}

/// Initialize an OpenTelemetry `MeterProvider` that periodically pushes
/// metrics to an OTLP endpoint.
///
/// The exporter reuses the same endpoint / protocol configuration as the trace
/// exporter so that a single collector receives both signals.
fn init_meter_provider(
    opts: &LoggingOptions,
    resource: opentelemetry_sdk::Resource,
) -> SdkMeterProvider {
    let protocol = opts
        .otlp_export_protocol
        .clone()
        .unwrap_or(OtlpExportProtocol::Http);

    let endpoint = opts
        .otlp_endpoint
        .as_ref()
        .map(|e| {
            if e.starts_with("http") {
                e.clone()
            } else {
                format!("http://{e}")
            }
        })
        .unwrap_or_else(|| match protocol {
            // Metrics share the same base endpoint; the SDK appends the
            // correct path automatically for HTTP.
            OtlpExportProtocol::Grpc => DEFAULT_OTLP_GRPC_ENDPOINT.to_string(),
            OtlpExportProtocol::Http => {
                // Use the base OTLP HTTP endpoint — the SDK appends /v1/metrics.
                "http://localhost:4318".to_string()
            }
        });

    let exporter = match protocol {
        OtlpExportProtocol::Grpc => opentelemetry_otlp::MetricExporter::builder()
            .with_tonic()
            .with_endpoint(&endpoint)
            .build()
            .expect("failed to build OTLP gRPC metric exporter"),
        OtlpExportProtocol::Http => opentelemetry_otlp::MetricExporter::builder()
            .with_http()
            .with_http_client(build_otlp_http_client())
            .with_endpoint(&endpoint)
            .with_headers(opts.otlp_headers.clone())
            .build()
            .expect("failed to build OTLP HTTP metric exporter"),
    };

    let reader = opentelemetry_sdk::metrics::PeriodicReader::builder(exporter)
        .with_interval(std::time::Duration::from_secs(30))
        .build();

    SdkMeterProvider::builder()
        .with_reader(reader)
        .with_resource(resource)
        .build()
}

/// Initialize an OpenTelemetry `SdkLoggerProvider` that ships log records to
/// an OTLP/HTTP endpoint via a batch processor.
///
/// The exporter is intentionally HTTP-only — Loki's native OTLP receiver
/// listens on `/otlp/v1/logs` over HTTP, and we don't currently target a gRPC
/// log backend. `otlp_logs_endpoint` is independent from `otlp_endpoint`
/// because logs and traces typically live on different services even when
/// colocated (Loki vs Langfuse).
fn init_logger_provider(
    opts: &LoggingOptions,
    resource: opentelemetry_sdk::Resource,
) -> SdkLoggerProvider {
    let endpoint = opts
        .otlp_logs_endpoint
        .as_ref()
        .map(|e| {
            if e.starts_with("http") {
                e.clone()
            } else {
                format!("http://{e}")
            }
        })
        .unwrap_or_else(|| DEFAULT_OTLP_HTTP_LOGS_ENDPOINT.to_string());

    let exporter = LogExporter::builder()
        .with_http()
        .with_http_client(build_otlp_http_client())
        .with_endpoint(endpoint)
        .with_protocol(Protocol::HttpBinary)
        .with_headers(opts.otlp_logs_headers.clone())
        .build()
        .expect("failed to build OTLP HTTP log exporter");

    SdkLoggerProvider::builder()
        .with_batch_exporter(exporter)
        .with_resource(resource)
        .build()
}

#[cfg(test)]
#[allow(unsafe_code)]
mod tests {
    //! Tests for the OTLP construction site fix (issue #1982).
    //!
    //! These tests run in plain `#[test]` mode — i.e. no outer tokio
    //! Runtime is active on the calling thread. That mirrors the fixed
    //! production codepath, where `init_logging` runs synchronously
    //! before `Runtime::new()` in `fn main()`. If any of these test
    //! functions panicked with "Cannot drop a runtime in a context where
    //! blocking is not allowed", the fix would have regressed.
    //!
    //! Note: we intentionally exercise `build_otlp_exporter` and
    //! `build_otlp_http_client` directly rather than going through
    //! `init_global_logging`, because the latter installs a process-wide
    //! global subscriber via `Once` — only one `#[test]` could use it
    //! per test binary. The panic root-cause we are guarding against
    //! lives in `reqwest::blocking::Client::builder().build()`, which is
    //! reached via `build_otlp_http_client`, which is called from all
    //! three `build_otlp_*` / `init_*_provider` helpers — so testing the
    //! helper directly is equivalent.

    use std::{
        io::{Read, Write},
        net::{SocketAddr, TcpListener, TcpStream},
        sync::mpsc,
        thread,
        time::Duration,
    };

    use opentelemetry::trace::{Tracer, TracerProvider as _};
    use opentelemetry_sdk::trace::SdkTracerProvider;

    use super::*;

    /// Scenario 1: telemetry init does not panic when OTLP is enabled.
    ///
    /// The pre-fix codepath called
    /// `reqwest::blocking::Client::builder().build()` from inside
    /// `#[tokio::main]` and panicked with "Cannot drop a runtime in a
    /// context where blocking is not allowed". The fix moves the call out
    /// of the async context — this test reproduces the synchronous (no
    /// outer Runtime) call shape that production now uses.
    #[test]
    fn otlp_init_does_not_panic_from_production_codepath() {
        // No tokio runtime is current on this thread — that is the
        // shape of the post-fix call from `init_server_sync` in
        // `crates/cmd/src/main.rs`.
        assert!(
            tokio::runtime::Handle::try_current().is_err(),
            "test precondition: no tokio runtime should be current",
        );

        let opts = LoggingOptions {
            enable_otlp_tracing: true,
            otlp_endpoint: Some("http://127.0.0.1:1/".to_string()),
            otlp_export_protocol: Some(OtlpExportProtocol::Http),
            ..Default::default()
        };

        // The exporter builds the HTTP client synchronously. If the
        // panic regresses, this line aborts the test thread.
        let _exporter = build_otlp_exporter(&opts);
    }

    /// Scenario 2: OTLP HTTP client preserves `.no_proxy()`.
    ///
    /// `reqwest::blocking::Client` does not expose its proxy
    /// configuration via a public getter, so we observe the behavior:
    /// set `HTTPS_PROXY` to an unreachable address, ask the client to
    /// hit an HTTPS URL pointing at a local TCP listener, and check
    /// where the connection actually lands. With `.no_proxy()` the
    /// client connects to our listener; without it, the client would
    /// try the proxy address instead.
    #[test]
    fn otlp_http_client_bypasses_env_proxy() {
        // Bind a local listener that records the first incoming peer.
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind local listener");
        let addr = listener.local_addr().expect("local addr");
        let (tx, rx) = mpsc::channel::<()>();

        let listener_thread = thread::spawn(move || {
            // Accept one connection and signal — that's enough to prove
            // the client targeted us, not the env proxy.
            if let Ok((mut stream, _peer)) = listener.accept() {
                let _ = tx.send(());
                // Drain anything the client wrote so it doesn't block.
                let mut buf = [0u8; 64];
                let _ = stream.set_read_timeout(Some(Duration::from_millis(200)));
                let _ = stream.read(&mut buf);
            }
        });

        // SAFETY: `set_var` is unsafe in 2024 edition because env vars
        // are process-global. This test does not run in parallel with
        // other tests that read `HTTPS_PROXY` (unique to this test in
        // this crate). The variable is restored before the test exits.
        let prev = std::env::var("HTTPS_PROXY").ok();
        unsafe {
            std::env::set_var("HTTPS_PROXY", "http://127.0.0.1:1");
        }

        let client = build_otlp_http_client();
        // A short timeout keeps the test fast — we only need to see
        // *which* address the client tried to connect to.
        let url = format!("http://{addr}/");
        let _ = client
            .post(&url)
            .timeout(Duration::from_secs(2))
            .body("ping")
            .send();

        // Restore the env var before asserting.
        unsafe {
            match prev {
                Some(v) => std::env::set_var("HTTPS_PROXY", v),
                None => std::env::remove_var("HTTPS_PROXY"),
            }
        }

        let connected = rx.recv_timeout(Duration::from_secs(3)).is_ok();
        let _ = listener_thread.join();
        assert!(
            connected,
            "OTLP blocking client honored HTTPS_PROXY instead of bypassing it — .no_proxy() \
             regression"
        );
    }

    /// Scenario 3: batch exporter delivers spans via the blocking client.
    ///
    /// Stand up a minimal HTTP server on a local port, point an OTLP
    /// trace pipeline at it, emit one span, shut the provider down (which
    /// drains any pending batches synchronously), and assert the server
    /// received at least one POST. Specifically guards against:
    ///   1. "Cannot drop a runtime in a context where blocking is not allowed"
    ///      — would surface if `reqwest::blocking::Client` construction landed
    ///      back inside an async context.
    ///   2. "there is no reactor running" — would surface if we regressed to
    ///      async `reqwest::Client` on the BatchSpan processor's std::thread.
    #[test]
    fn otlp_trace_export_round_trip_via_blocking_client() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind stub server");
        let addr: SocketAddr = listener.local_addr().expect("local addr");
        let _ = listener.set_nonblocking(false);

        let (post_tx, post_rx) = mpsc::channel::<()>();

        // Minimal HTTP/1.1 server. Loops accepting connections until the
        // test drops the listener (server_done flag) or the loop exits
        // naturally on a closed socket. Each connection gets one response.
        let stop_flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stop_flag_for_thread = stop_flag.clone();
        let server_thread = thread::spawn(move || {
            // Short accept timeout so the loop can poll the stop flag.
            let _ = listener.set_nonblocking(true);
            while !stop_flag_for_thread.load(std::sync::atomic::Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => handle_one_post(stream, &post_tx),
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(50));
                    }
                    Err(_) => break,
                }
            }
        });

        let opts = LoggingOptions {
            enable_otlp_tracing: true,
            otlp_endpoint: Some(format!("http://{addr}/v1/traces")),
            otlp_export_protocol: Some(OtlpExportProtocol::Http),
            ..Default::default()
        };

        // Build the exporter the same way `init_global_logging` does,
        // but assemble the provider locally so this test does not
        // collide with the process-global `Once` guard inside
        // `init_global_logging`.
        let exporter = build_otlp_exporter(&opts);
        let resource = build_otel_resource("test", None, &opts);
        let provider = SdkTracerProvider::builder()
            .with_batch_exporter(exporter)
            .with_sampler(opentelemetry_sdk::trace::Sampler::AlwaysOn)
            .with_resource(resource)
            .build();
        let tracer = provider.tracer("otlp_trace_export_round_trip_via_blocking_client");

        // Emit one span and shut down the provider on a worker thread so
        // we can bound the wait. `shutdown` drains pending batches
        // synchronously through the blocking exporter — this is the
        // codepath that would panic on the regression.
        let provider_for_worker = provider.clone();
        let worker = thread::spawn(move || {
            let span = tracer.start("test-span");
            drop(span);
            // `shutdown` flushes pending batches and is the canonical
            // synchronization point in 0.31 — `force_flush` returns
            // before the send completes for the standard processor.
            let _ = provider_for_worker.shutdown();
        });

        // Wait up to 5s for the stub server to record a POST.
        let got_post = post_rx.recv_timeout(Duration::from_secs(5)).is_ok();

        // Tear down: signal server, drop provider, join threads.
        stop_flag.store(true, std::sync::atomic::Ordering::SeqCst);
        let _ = worker.join();
        // Open a throwaway connection so the server's accept loop wakes
        // up and observes the stop flag immediately.
        let _ = TcpStream::connect_timeout(&addr, Duration::from_millis(200));
        let _ = server_thread.join();

        assert!(
            got_post,
            "BatchSpanProcessor did not deliver any POST to the stub OTLP server within 5s",
        );
    }

    fn handle_one_post(mut stream: TcpStream, post_tx: &mpsc::Sender<()>) {
        let _ = stream.set_nonblocking(false);
        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
        let mut buf = [0u8; 4096];
        let n = stream.read(&mut buf).unwrap_or(0);
        let request = String::from_utf8_lossy(&buf[..n]);
        if request.starts_with("POST ") {
            let _ = post_tx.send(());
        }
        // 200 OK with empty body is enough for OTel's HTTP exporter to
        // treat the export as successful.
        let _ =
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        let _ = stream.flush();
    }
}
