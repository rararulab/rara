spec: task
name: "issue-1975-trace-id-response-header"
inherits: project
tags: ["enhancement", "server", "web", "telemetry"]
---

## Intent

Today every backend HTTP request is wrapped in an `http_request` info_span
inside `crates/server/src/http.rs:199` (`TraceLayer::new_for_http()`), and
the `tracing-opentelemetry` layer attaches an OTel trace_id that flows out
to Langfuse (traces) and Loki (logs, with `trace_id` / `span_id` as
structured-metadata). The trace_id exists, is durable, and is the join key
between the two observability backends — but the browser never sees it.
When a user reports "rara didn't reply to my 3:42pm message", a developer
has nothing to grep for: no header in devtools, no ID in the error toast,
just a wall-clock window across three channels (web, telegram, kernel).

This spec exposes the existing OTel trace_id to the client by writing two
response headers from a single tower middleware layered onto the global
`TraceLayer`:

- `x-request-id: <trace_id_32_hex>` — what the user asked for; copyable
  ID that pastes directly into Langfuse and Loki searches.
- `traceparent: 00-<trace_id>-<span_id>-01` — W3C standard, lets a future
  browser OTel SDK link client spans to server traces without us having
  to invent a second propagation path.

Both headers are added to the CORS `expose_headers` allow-list in
`crates/extensions/backend-admin/src/state.rs::build_cors_layer` so the
browser can read them across the `localhost:5173 → 10.0.0.183:25555`
proxy boundary. Frontend then surfaces `x-request-id` from the existing
`ApiError` path in `web/src/api/client.ts` so error toasts include a
copyable ID.

This work does NOT introduce a new ID concept. The `request_id` UUID in
`crates/channels/src/telegram/adapter.rs` and `crates/channels/src/web.rs`
is unrelated — it correlates guard-prompt/resolution pairs inside the
channel layer, not HTTP requests. The new header carries the existing OTel
trace_id, nothing else. No new tracing infrastructure, no new ID
generator, no new SDK initialization.

Reproducer for the bug we are preventing: 1. user pings rara at 15:42 and
gets no reply; 2. user reports it; 3. dev opens browser devtools — no
identifier on the failed request; 4. dev SSHes to `raratekiAir`, tails
`Library/Logs/rara/job.*`, greps a 30-minute window across web,
telegram, and kernel spans because there is no anchor; 5. dev gives up or
guesses. After this spec ships, step 3 yields a 32-hex `x-request-id`
that pastes into Langfuse and Loki and resolves the trace in seconds.

Goal alignment: advances `goal.md` signal 4 ("Every action is
inspectable") — the trace_id already exists end-to-end in the backend;
this work extends inspectability to the client surface so the user (or
the developer reading a user's bug report) is one paste away from the
trace. Does not cross any "What rara is NOT" line: this is observability
plumbing, not a new product surface or feature-parity item.

Hermes positioning: not applicable. Hermes Agent's observability
boundary is its own; rara's choice to expose OTel trace_id at the HTTP
edge is independent.

## Decisions

### Where the middleware lives

A single tower middleware named `inject_trace_headers` added at the
global router level in `crates/server/src/http.rs`, layered AFTER
`TraceLayer::new_for_http()` so the OTel span is already active when the
middleware reads it. Equivalently this can be expressed as a `.map_response`
adapter or as a `tower::Layer` that wraps the inner service and rewrites
the response on the way out. Either form is acceptable provided the layer
is applied exactly once at the global router (not per-route, not in any
domain router).

The middleware reads the current OTel context via
`tracing::Span::current().context()` (the bridge already used in
`crates/common/telemetry/src/tracing_context.rs`), extracts `trace_id`
and `span_id` from the active `SpanContext`, and writes the two headers
on the outbound `axum::http::Response`.

If the active span is not sampled or has no valid trace_id (e.g. the
request was rejected before reaching `TraceLayer`'s `make_span_with`),
the middleware MUST NOT panic and MUST NOT write empty header values —
it leaves the headers off and lets the response pass through.

### Header names and formats

- `x-request-id`: lowercase 32-hex characters, no separators, exactly
  matching the OTel `TraceId::to_hex` representation already emitted to
  Langfuse and Loki. This is what the user pastes.
- `traceparent`: W3C Trace Context v00, `00-<32-hex trace_id>-<16-hex
  span_id>-01` (sampled flag set to `01`). The crate
  `opentelemetry-http` provides a propagator helper, but the format is
  stable and small enough that constructing it directly in the
  middleware is equally acceptable. Either choice — propagator or
  direct format — is fine; the constraint is the on-the-wire bytes.

The two headers are independent: if for some reason the middleware can
read `trace_id` but not `span_id`, it writes `x-request-id` and skips
`traceparent`. Partial information beats no information.

### CORS expose_headers

`build_cors_layer` in `crates/extensions/backend-admin/src/state.rs`
currently sets `allow_headers([AUTHORIZATION, CONTENT_TYPE])` and does
not call `.expose_headers(...)`. Without `expose_headers`, the browser
fetch API hides every non-CORS-safelisted response header — including
both new ones. The decision is to add a single `.expose_headers([...])`
call listing exactly `x-request-id` and `traceparent`. Do not expose
any other header in the same call.

### Frontend surface — minimal viable

`web/src/api/client.ts` already has a centralized `ApiError` class
constructed at the two `throw new ApiError(...)` sites (lines 230, 235,
274, 278 area). The decision is:

1. Extend `ApiError` with one optional field `requestId?: string`.
2. At each `throw new ApiError(...)` site, read
   `res.headers.get('x-request-id')` and pass it through.
3. The existing error rendering path (whichever component actually
   renders an `ApiError` for the user — pick the most-used one, do NOT
   sweep) appends `requestId` to the displayed message in a copyable
   form, e.g. `(id: <first-8-hex>…)` with the full ID available on
   click or in the title attribute.

If the codebase has no single dominant error renderer, the implementer
falls back to logging the full `requestId` to `console.error` alongside
the existing `ApiError` message — the developer can always pull it from
the browser console, which is strictly better than today.

### What this spec does NOT do

- Does not add any new tracing crate, SDK, propagator, or initializer.
  The `TraceContextPropagator` is already installed at
  `crates/common/telemetry/src/logging.rs:668`.
- Does not touch the `request_id` UUID in `crates/channels/src/`. That
  field stays exactly as-is.
- Does not add the headers per-endpoint. The middleware is global.
- Does not add a frontend OTel SDK. The `traceparent` header is shipped
  for forward compatibility; nobody on the frontend reads it today.
- Does not modify `e2e.yml`, `rust.yml`, or any CI config.
- Does not modify the `cors_allowed_origins` config field. Only
  `expose_headers` changes inside `build_cors_layer`.

## Boundaries

### Allowed Changes

- **/crates/server/src/http.rs
- **/crates/extensions/backend-admin/src/state.rs
- **/web/src/api/client.ts
- **/web/src/api/__tests__/**
- **/web/src/components/**
- **/specs/issue-1975-trace-id-response-header.spec.md
- **/crates/server/Cargo.toml
- **/crates/server/tests/**
- New backend test file under `crates/server/tests/` (or extending an
  existing one) covering the middleware behavior.
- New frontend test under `web/src/api/__tests__/` covering
  `ApiError.requestId` propagation.

### Forbidden

- Do NOT introduce a new ID generator (UUID, ULID, nanoid, etc.). The
  trace_id already exists; reuse it.
- Do NOT add or modify the `request_id` field in
  `crates/channels/src/telegram/adapter.rs` or
  `crates/channels/src/web.rs`. That is a different concept.
- Do NOT add the header at any per-route, per-router, or per-handler
  layer. Exactly one global tower layer in `crates/server/src/http.rs`.
- Do NOT widen `expose_headers` to include any header beyond
  `x-request-id` and `traceparent`. CORS allow-lists are
  default-deny; a sweep here is out of scope.
- Do NOT change `allow_origin`, `allow_methods`, or `allow_headers`
  in `build_cors_layer`. Only `expose_headers` is added.
- Do NOT change the response status, body, or any other header for
  any request. The middleware is additive only.
- Do NOT install a new tracing subscriber, propagator, or tower
  middleware ordering elsewhere in the stack. The change is one layer
  in one file.
- Do NOT introduce a new error type on the frontend. Extend the
  existing `ApiError` class with one optional field.
- Do NOT sweep every error surface in the frontend. Pick one path
  (the existing error renderer) or fall back to `console.error`.
- Do NOT block the response when the active span has no trace_id —
  skip the headers and pass through.
- Do NOT introduce new YAML config for header names, formats, or
  enable/disable flags. The mechanism is always-on.

## Completion Criteria

Scenario: A successful HTTP request carries x-request-id matching the OTel trace_id
  Test:
    Package: rara-server
    Filter: trace_headers_x_request_id_matches_otel_trace_id
  Given the rara HTTP server is running with TraceLayer::new_for_http() applied
  When a client issues GET /health
  Then the response includes an x-request-id header consisting of exactly 32 lowercase hex characters
  And the value equals the OTel trace_id captured from the active span during the request

Scenario: A successful HTTP request carries a W3C-format traceparent header
  Test:
    Package: rara-server
    Filter: trace_headers_traceparent_w3c_format
  Given the rara HTTP server is running with TraceLayer::new_for_http() applied
  When a client issues GET /health
  Then the response includes a traceparent header matching the regex ^00-[0-9a-f]{32}-[0-9a-f]{16}-01$
  And the trace_id segment equals the x-request-id header on the same response

Scenario: CORS expose_headers permits the browser to read both headers
  Test:
    Package: rara-backend-admin
    Filter: cors_exposes_trace_headers
  Given build_cors_layer is constructed with at least one allowed origin
  When the resulting CorsLayer is inspected for its expose_headers configuration
  Then the configuration lists exactly x-request-id and traceparent

Scenario: ApiError carries the request id from the response when set
  Test:
    Package: rara-server
    Filter: api_client_request_id_header_name
  Given the server middleware emits a response header under a fixed wire name
  When the web api client reads that header to populate ApiError.requestId
  Then the wire name is pinned to `x-request-id` so both ends of the
       cross-language contract resolve to the same header.
  Note: agent-spec's `Test:` selector only dispatches `cargo test`, so the
  Rust assertion above pins the wire literal. The end-to-end TS assertion
  ("ApiError instance exposes requestId equal to the header value") lives
  at `web/src/api/__tests__/client.requestId.test.ts` and runs under
  `bun run test`; the implementer attaches its passing output as
  supplementary verification.

## Out of Scope

- A frontend OpenTelemetry SDK or any client-side tracing instrumentation.
- A user-facing settings UI for "send diagnostics" or "include trace id"
  toggles. The headers are always emitted; whether the UI surfaces the
  ID is a separate UX decision and is fixed to "show on error" in this
  spec.
- Backfilling all existing frontend error toasts to include the request
  id. Only the ApiError-rendering path is modified; bespoke error UIs
  in individual components are not touched in this PR.
- Removing or refactoring the `request_id` UUID concept in
  `crates/channels/src/`. That is a different correlation ID and is
  preserved as-is.
- Plumbing the trace_id into WebSocket frames. The vite proxy preserves
  HTTP upgrade headers, but per-frame correlation is a different problem
  and is not in scope.
- Sampling decisions, header redaction in untrusted environments, or
  per-route opt-out. The middleware is unconditional and global.
- Changing how Langfuse or Loki ingest trace data. The trace_id format
  emitted on the wire matches what those backends already see.
