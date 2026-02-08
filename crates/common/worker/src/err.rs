// Copyright 2025 Crrow
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

use std::fmt;

use snafu::Snafu;

// ============================================================================
// Work Error Types
// ============================================================================

/// Result type for worker operations.
pub type WorkResult<T = ()> = std::result::Result<T, WorkError>;

/// Error severity level for worker operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorSeverity {
    /// Transient error - worker will continue and retry on next trigger.
    ///
    /// Use this for recoverable errors like:
    /// - Network timeouts
    /// - Temporary resource unavailability
    /// - Rate limiting
    Transient,

    /// Fatal error - worker should stop immediately.
    ///
    /// Use this for unrecoverable errors like:
    /// - Configuration errors
    /// - Missing required resources
    /// - Data corruption
    Fatal,
}

/// Errors that can occur during worker execution.
///
/// Workers can return these errors from `work()`, `on_start()`, and
/// `on_shutdown()` methods. The error severity determines whether the worker
/// continues or stops.
///
/// # Example
///
/// ```rust
/// use rsketch_common_worker::{WorkError, WorkResult};
///
/// async fn do_work() -> WorkResult {
///     // Transient error - worker continues
///     if network_unavailable() {
///         return Err(WorkError::transient("Network temporarily unavailable"));
///     }
///
///     // Fatal error - worker stops
///     if config_invalid() {
///         return Err(WorkError::fatal("Invalid configuration"));
///     }
///
///     Ok(())
/// }
/// # fn network_unavailable() -> bool { false }
/// # fn config_invalid() -> bool { false }
/// ```
#[derive(Debug)]
pub struct WorkError {
    severity: ErrorSeverity,
    message:  String,
    source:   Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl WorkError {
    /// Creates a new transient error.
    ///
    /// Transient errors allow the worker to continue - it will be retried on
    /// the next trigger.
    pub fn transient(message: impl Into<String>) -> Self {
        Self {
            severity: ErrorSeverity::Transient,
            message:  message.into(),
            source:   None,
        }
    }

    /// Creates a new fatal error.
    ///
    /// Fatal errors cause the worker to stop immediately after calling
    /// `on_shutdown()`.
    pub fn fatal(message: impl Into<String>) -> Self {
        Self {
            severity: ErrorSeverity::Fatal,
            message:  message.into(),
            source:   None,
        }
    }

    /// Creates a transient error with a source error.
    pub fn transient_with_source<E>(message: impl Into<String>, source: E) -> Self
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        Self {
            severity: ErrorSeverity::Transient,
            message:  message.into(),
            source:   Some(Box::new(source)),
        }
    }

    /// Creates a fatal error with a source error.
    pub fn fatal_with_source<E>(message: impl Into<String>, source: E) -> Self
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        Self {
            severity: ErrorSeverity::Fatal,
            message:  message.into(),
            source:   Some(Box::new(source)),
        }
    }

    /// Returns the error severity.
    #[must_use]
    pub const fn severity(&self) -> ErrorSeverity { self.severity }

    /// Returns `true` if this is a fatal error.
    #[must_use]
    pub fn is_fatal(&self) -> bool { self.severity == ErrorSeverity::Fatal }

    /// Returns `true` if this is a transient error.
    #[must_use]
    pub fn is_transient(&self) -> bool { self.severity == ErrorSeverity::Transient }

    /// Returns the error message.
    #[must_use]
    pub fn message(&self) -> &str { &self.message }
}

impl fmt::Display for WorkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let severity = match self.severity {
            ErrorSeverity::Transient => "transient",
            ErrorSeverity::Fatal => "fatal",
        };
        write!(f, "[{}] {}", severity, self.message)
    }
}

impl std::error::Error for WorkError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_ref()
            .map(|e| e.as_ref() as &(dyn std::error::Error + 'static))
    }
}

// ============================================================================
// Cron Parse Error
// ============================================================================

/// Errors that can occur when parsing cron expressions.
///
/// # Example
///
/// ```rust
/// use std::str::FromStr;
///
/// use rsketch_common_worker::CronParseError;
///
/// let result = croner::Cron::from_str("invalid cron");
/// match result {
///     Ok(cron) => println!("Valid cron"),
///     Err(e) => println!("Invalid cron: {}", e),
/// }
/// ```
#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum CronParseError {
    /// The cron expression could not be parsed.
    ///
    /// This typically occurs when:
    /// - Invalid syntax (e.g., "60 * * * *" for minute)
    /// - Wrong number of fields (expects 5: minute hour day month weekday)
    /// - Invalid range values
    #[snafu(display("Failed to parse cron expression: {source}"))]
    InvalidExpression {
        source: croner::errors::CronError,
        #[snafu(implicit)]
        loc:    snafu::Location,
    },
}
