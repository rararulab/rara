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

//! # Distributed Tracing Context
//!
//! Provides context propagation for distributed tracing across service
//! boundaries. Supports W3C Trace Context standard and OpenTelemetry
//! integration.

use std::collections::HashMap;

use opentelemetry::propagation::TextMapPropagator;
use opentelemetry_sdk::propagation::TraceContextPropagator;
use smart_default::SmartDefault;
use tracing_opentelemetry::OpenTelemetrySpanExt;

/// Extension trait for instrumenting futures with tracing spans.
pub trait FutureExt: std::future::Future + Sized {
    /// Attach a span to this future for tracing.
    fn trace(self, span: tracing::span::Span) -> tracing::instrument::Instrumented<Self>;
}

impl<T: std::future::Future> FutureExt for T {
    #[inline]
    fn trace(self, span: tracing::span::Span) -> tracing::instrument::Instrumented<Self> {
        tracing::instrument::Instrument::instrument(self, span)
    }
}

/// Distributed tracing context for propagating trace information across
/// services.
///
/// Enables trace correlation in distributed systems by carrying trace context
/// between services. Uses W3C Trace Context format for interoperability.
#[derive(Debug, Clone, SmartDefault, derive_more::Into, derive_more::From)]
pub struct TracingContext(
    #[default(_code = "opentelemetry::Context::new()")] opentelemetry::Context,
);

/// W3C Trace Context format as key-value pairs.
pub type W3cTrace = HashMap<String, String>;

type Propagator = TraceContextPropagator;

impl TracingContext {
    /// Create context from a specific span.
    #[must_use]
    pub fn from_span(span: &tracing::Span) -> Self { Self(span.context()) }

    /// Create context from the current active span.
    #[must_use]
    pub fn from_current_span() -> Self { Self::from_span(&tracing::Span::current()) }

    /// Create an empty context.
    #[must_use]
    pub fn new() -> Self { Self(opentelemetry::Context::new()) }

    /// Attach a span as a child of this context.
    #[must_use]
    pub fn attach(&self, span: tracing::Span) -> tracing::Span {
        let _ = span.set_parent(self.0.clone());
        span
    }

    /// Convert to W3C trace context format.
    #[must_use]
    pub fn to_w3c(&self) -> W3cTrace {
        let mut fields = HashMap::new();
        Propagator::new().inject_context(&self.0, &mut fields);
        fields
    }

    /// Create from W3C trace context format.
    #[must_use]
    pub fn from_w3c(fields: &W3cTrace) -> Self {
        let context = Propagator::new().extract(fields);
        Self(context)
    }

    /// Serialize to JSON string.
    ///
    /// # Panics
    ///
    /// Panics if JSON serialization fails (should not happen with valid W3C
    /// context).
    #[must_use]
    pub fn to_json(&self) -> String { serde_json::to_string(&self.to_w3c()).unwrap() }

    /// Deserialize from JSON string. Returns empty context on invalid JSON.
    #[must_use]
    pub fn from_json(json: &str) -> Self {
        let fields: W3cTrace = serde_json::from_str(json).unwrap_or_default();
        Self::from_w3c(&fields)
    }
}
