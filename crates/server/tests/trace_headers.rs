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

//! Spec scenarios for `specs/issue-1975-trace-id-response-header.spec.md`.
//!
//! Both scenarios drive a router built with the same layer stack as
//! [`rara_server::http::start_rest_server`] (TraceLayer + the
//! `inject_trace_headers` middleware), instrumented with an in-process OTel
//! tracer so `Span::current().context()` resolves to a valid span. The
//! production code path is exercised end-to-end.

use std::{sync::Once, time::Duration};

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
    middleware,
    routing::get,
};
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_sdk::trace::SdkTracerProvider;
use tower::ServiceExt;
use tower_http::trace::TraceLayer;
use tracing::Span;
use tracing_subscriber::{Registry, layer::SubscriberExt, util::SubscriberInitExt};

static INIT: Once = Once::new();

/// Install a global tracing subscriber with a `tracing-opentelemetry`
/// layer backed by an in-process `SdkTracerProvider`. This is what makes
/// `Span::current().context().span().span_context().is_valid()` return
/// `true` inside the test — without an OTel layer, the bridge yields an
/// empty context.
fn init_otel() {
    INIT.call_once(|| {
        let provider = SdkTracerProvider::builder()
            .with_sampler(opentelemetry_sdk::trace::Sampler::AlwaysOn)
            .build();
        let tracer = provider.tracer("rara-server-test");
        let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);
        Registry::default().with(otel_layer).init();
    });
}

/// Replicate the layer order from [`start_rest_server`]: the
/// `inject_trace_headers` middleware sits inside `TraceLayer` so the span
/// is active when the middleware reads `Span::current()` on the response
/// path.
fn build_test_router() -> Router {
    Router::new()
        .route("/health", get(|| async { (StatusCode::OK, "OK") }))
        .layer(middleware::from_fn(rara_server::http::inject_trace_headers))
        .layer(
            TraceLayer::new_for_http().make_span_with(|request: &axum::http::Request<_>| {
                tracing::info_span!(
                    "http_request",
                    method = %request.method(),
                    path = %request.uri().path(),
                )
            }),
        )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn trace_headers_x_request_id_matches_otel_trace_id() {
    init_otel();

    // Capture the trace_id observed inside the request span by entering a
    // matching span around the oneshot call. TraceLayer sets up its own
    // span as a child of whatever is current, so the trace_id propagates.
    let outer_span = tracing::info_span!("test_outer");
    let trace_id_hex = {
        use opentelemetry::trace::TraceContextExt;
        use tracing_opentelemetry::OpenTelemetrySpanExt;
        let cx = outer_span.context();
        let span_ref = cx.span();
        let sc = span_ref.span_context();
        assert!(sc.is_valid(), "outer span must be OTel-instrumented");
        format!("{:032x}", u128::from_be_bytes(sc.trace_id().to_bytes()))
    };

    let app = build_test_router();
    let response = {
        let _enter = outer_span.enter();
        // Drop the guard before awaiting (Send across thread boundary).
        drop(_enter);
        let _enter = outer_span.enter();
        app.oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
    };
    // Force the provider to flush — not strictly required since the
    // assertion is on response headers, not exported spans, but keeps
    // the test deterministic when run alongside the other.
    tokio::time::sleep(Duration::from_millis(10)).await;

    assert_eq!(response.status(), StatusCode::OK);

    let header_value = response
        .headers()
        .get("x-request-id")
        .expect("x-request-id must be present on the response")
        .to_str()
        .unwrap()
        .to_string();

    assert_eq!(
        header_value.len(),
        32,
        "x-request-id must be exactly 32 hex chars, got {header_value:?}"
    );
    assert!(
        header_value
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
        "x-request-id must be lowercase hex, got {header_value:?}"
    );
    assert_eq!(
        header_value, trace_id_hex,
        "x-request-id must equal the OTel trace_id active during the request"
    );

    // Suppress unused-variable hint — the span is held alive for the call.
    drop(Span::current());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn trace_headers_traceparent_w3c_format() {
    init_otel();

    let app = build_test_router();
    let outer_span = tracing::info_span!("test_outer_2");
    let response = {
        let _enter = outer_span.enter();
        app.oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
    };

    assert_eq!(response.status(), StatusCode::OK);

    let traceparent = response
        .headers()
        .get("traceparent")
        .expect("traceparent must be present on the response")
        .to_str()
        .unwrap()
        .to_string();
    let request_id = response
        .headers()
        .get("x-request-id")
        .expect("x-request-id must also be present")
        .to_str()
        .unwrap()
        .to_string();

    // Format check: 00-<32hex>-<16hex>-01.
    let parts: Vec<&str> = traceparent.split('-').collect();
    assert_eq!(
        parts.len(),
        4,
        "traceparent must have 4 dash-separated parts: {traceparent:?}"
    );
    assert_eq!(parts[0], "00", "version must be 00");
    assert_eq!(parts[1].len(), 32, "trace_id segment must be 32 hex chars");
    assert_eq!(parts[2].len(), 16, "span_id segment must be 16 hex chars");
    assert_eq!(parts[3], "01", "sampled flag must be 01");
    assert!(
        parts[1]
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
            && parts[2]
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
        "trace_id and span_id must be lowercase hex"
    );

    assert_eq!(
        parts[1], request_id,
        "trace_id segment of traceparent must equal x-request-id on the same response"
    );
}

/// Spec scenario `api_client_request_id_header_name`: the wire header name
/// emitted by [`rara_server::http::inject_trace_headers`] must equal the
/// literal that `web/src/api/client.ts` reads via
/// `res.headers.get('x-request-id')`. agent-spec's `Test:` selector only
/// dispatches `cargo test`, so this Rust test pins the cross-language
/// contract that the vitest test
/// `web/src/api/__tests__/client.requestId.test.ts` exercises end-to-end:
/// if either end drifts off the literal `x-request-id`, this assertion
/// catches it from the Rust side and the vitest assertion catches it
/// from the TS side.
#[test]
fn api_client_request_id_header_name() {
    assert_eq!(
        rara_server::http::TRACE_HEADER_REQUEST_ID.as_str(),
        "x-request-id",
        "wire header name must match the literal in web/src/api/client.ts (REQUEST_ID_HEADER); \
         changing one without the other breaks ApiError.requestId propagation"
    );
}
