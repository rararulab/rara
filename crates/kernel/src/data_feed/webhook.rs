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

//! Webhook data feed — receives HTTP POST events from external services.
//!
//! Unlike WebSocket or polling feeds, a webhook feed is *passive*: it does not
//! implement [`DataFeed::run`](super::DataFeed::run). Instead, it exposes an
//! axum handler that external services POST to. The handler validates HMAC
//! signatures, deduplicates retried deliveries, constructs a [`FeedEvent`],
//! and forwards it through the kernel's event channel.
//!
//! # Route
//!
//! ```text
//! POST /api/v1/webhooks/{feed_name}
//! ```
//!
//! # Signature verification
//!
//! Two header conventions are supported:
//!
//! - **GitHub**: `X-Hub-Signature-256: sha256=<hex>`
//! - **Generic**: `X-Webhook-Signature: <hex>` (raw HMAC-SHA256 hex digest)
//!
//! If a feed has a `secret` configured but the request carries no recognised
//! signature header, the request is rejected with `401 Unauthorized`.

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use axum::{
    Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::post,
};
use hmac::{Hmac, Mac};
use jiff::Timestamp;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use subtle::ConstantTimeEq;
use tokio::sync::mpsc;
use tracing::{info, warn};

use super::{DataFeedRegistry, FeedEvent, FeedEventId, FeedType};

// ---------------------------------------------------------------------------
// WebhookConfig — per-feed webhook configuration
// ---------------------------------------------------------------------------

/// Webhook-specific configuration deserialised from a
/// [DataFeedConfig](super::DataFeedConfig)'s `config` field.
///
/// Stored as JSON inside the generic `serde_json::Value` config field of a
/// feed registration with [`FeedType::Webhook`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookConfig {
    /// HMAC-SHA256 shared secret for signature verification.
    ///
    /// When present, every incoming request **must** carry a valid signature
    /// header. Requests without a recognised signature header are rejected.
    pub secret: Option<String>,

    /// Optional list of event types to accept (e.g. `["push",
    /// "pull_request"]`).
    ///
    /// When non-empty, the handler checks the `X-GitHub-Event` header (or the
    /// `event_type` field in the JSON body) and rejects events not in this
    /// list. An empty vec means all event types are accepted.
    #[serde(default)]
    pub events: Vec<String>,
}

// ---------------------------------------------------------------------------
// WebhookState — shared state for the axum handler
// ---------------------------------------------------------------------------

/// Shared state injected into the webhook axum handler via [`State`].
pub struct WebhookState {
    /// Feed registry — used to look up per-feed configs at request time.
    registry: Arc<DataFeedRegistry>,
    /// Channel sender for forwarding validated events to the kernel.
    event_tx: mpsc::Sender<FeedEvent>,
    /// In-memory cache of recently-seen delivery IDs for idempotency.
    seen:     Mutex<HashMap<String, Instant>>,
}

/// TTL for idempotency cache entries.
const IDEMPOTENCY_TTL: Duration = Duration::from_secs(3600);

impl WebhookState {
    /// Create a new webhook state.
    pub fn new(registry: Arc<DataFeedRegistry>, event_tx: mpsc::Sender<FeedEvent>) -> Self {
        Self {
            registry,
            event_tx,
            seen: Mutex::new(HashMap::new()),
        }
    }

    /// Check whether `delivery_id` has been seen within the TTL window.
    ///
    /// If not seen, records it and returns `false`. If already present,
    /// returns `true` (duplicate). Also prunes expired entries.
    fn check_idempotency(&self, delivery_id: &str) -> bool {
        let now = Instant::now();
        let mut seen = self.seen.lock();

        // Prune expired entries.
        seen.retain(|_, ts| now.duration_since(*ts) < IDEMPOTENCY_TTL);

        if seen.contains_key(delivery_id) {
            return true; // duplicate
        }
        seen.insert(delivery_id.to_owned(), now);
        false
    }
}

// ---------------------------------------------------------------------------
// Axum handler
// ---------------------------------------------------------------------------

/// Axum handler for `POST /api/v1/webhooks/{feed_name}`.
///
/// Processing pipeline:
/// 1. Look up feed config in the registry (404 if not found).
/// 2. Verify the feed is of type [`FeedType::Webhook`] (400 otherwise).
/// 3. Deserialise [`WebhookConfig`] from the feed's config blob.
/// 4. Validate HMAC signature if a secret is configured (401 on failure).
/// 5. Extract event type from headers / body.
/// 6. Filter by allowed event types (if configured).
/// 7. Check idempotency via delivery ID (200 on duplicate).
/// 8. Construct [`FeedEvent`] and send through the event channel.
/// 9. Return `202 Accepted`.
#[allow(clippy::too_many_lines)]
pub async fn webhook_handler(
    Path(feed_name): Path<String>,
    State(state): State<Arc<WebhookState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    // 1. Look up feed config.
    let config = match state.registry.get(&feed_name) {
        Some(c) => c,
        None => {
            warn!(feed_name, "webhook received for unknown feed");
            return (
                StatusCode::NOT_FOUND,
                axum::Json(serde_json::json!({ "error": "unknown feed" })),
            );
        }
    };

    // 2. Must be a webhook-type feed.
    if config.feed_type != FeedType::Webhook {
        return (
            StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({ "error": "feed is not a webhook type" })),
        );
    }

    // 3. Deserialise webhook-specific config.
    let wh_config: WebhookConfig = match serde_json::from_value(config.config.clone()) {
        Ok(c) => c,
        Err(e) => {
            warn!(feed_name, error = %e, "invalid webhook config");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                axum::Json(serde_json::json!({ "error": "invalid webhook config" })),
            );
        }
    };

    // 4. HMAC signature verification.
    if let Some(ref secret) = wh_config.secret {
        if !verify_hmac(secret, &body, &headers) {
            warn!(feed_name, "webhook HMAC verification failed");
            return (
                StatusCode::UNAUTHORIZED,
                axum::Json(serde_json::json!({ "error": "invalid signature" })),
            );
        }
    }

    // 5. Extract event type from headers or body.
    let event_type = extract_event_type(&headers, &body);

    // 6. Filter by allowed event types.
    if !wh_config.events.is_empty() && !wh_config.events.contains(&event_type) {
        info!(feed_name, event_type, "webhook event type filtered out");
        return (
            StatusCode::OK,
            axum::Json(serde_json::json!({
                "status": "filtered",
                "event_type": event_type,
            })),
        );
    }

    // 7. Idempotency check via delivery ID.
    let delivery_id = extract_delivery_id(&headers);
    if state.check_idempotency(&delivery_id) {
        info!(feed_name, delivery_id, "duplicate webhook delivery skipped");
        return (
            StatusCode::OK,
            axum::Json(serde_json::json!({
                "status": "duplicate",
                "delivery_id": delivery_id,
            })),
        );
    }

    // 8. Parse body and construct FeedEvent.
    let payload: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => {
            // Fall back to treating body as an opaque string.
            serde_json::Value::String(String::from_utf8_lossy(&body).into_owned())
        }
    };

    let event = FeedEvent {
        id: FeedEventId::deterministic(&format!("{feed_name}:{delivery_id}")),
        source_name: feed_name.clone(),
        event_type: event_type.clone(),
        tags: config.tags.clone(),
        payload,
        received_at: Timestamp::now(),
    };

    // 9. Send to kernel.
    if let Err(e) = state.event_tx.send(event).await {
        warn!(feed_name, error = %e, "failed to send webhook event to kernel");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(serde_json::json!({ "error": "event channel closed" })),
        );
    }

    info!(feed_name, event_type, delivery_id, "webhook event accepted");
    (
        StatusCode::ACCEPTED,
        axum::Json(serde_json::json!({
            "status": "accepted",
            "feed": feed_name,
            "event_type": event_type,
            "delivery_id": delivery_id,
        })),
    )
}

// ---------------------------------------------------------------------------
// HMAC verification
// ---------------------------------------------------------------------------

/// Verify HMAC-SHA256 signature from request headers.
///
/// Supports two conventions:
/// - **GitHub**: `X-Hub-Signature-256: sha256=<hex>`
/// - **Generic**: `X-Webhook-Signature: <hex>`
///
/// Uses constant-time comparison via the [`subtle`] crate to prevent timing
/// attacks.
fn verify_hmac(secret: &str, body: &[u8], headers: &HeaderMap) -> bool {
    // GitHub format: "sha256=<hex>"
    if let Some(sig_header) = headers.get("x-hub-signature-256") {
        let sig_str = match sig_header.to_str() {
            Ok(s) => s,
            Err(_) => return false,
        };
        let hex_sig = match sig_str.strip_prefix("sha256=") {
            Some(h) => h,
            None => return false,
        };
        return verify_hex_hmac(secret, body, hex_sig);
    }

    // Generic format: raw hex digest.
    if let Some(sig_header) = headers.get("x-webhook-signature") {
        let hex_sig = match sig_header.to_str() {
            Ok(s) => s,
            Err(_) => return false,
        };
        return verify_hex_hmac(secret, body, hex_sig);
    }

    // No recognised signature header — reject when a secret is configured.
    false
}

/// Compute HMAC-SHA256 and constant-time compare against the provided hex
/// digest.
fn verify_hex_hmac(secret: &str, body: &[u8], expected_hex: &str) -> bool {
    let expected_bytes = match hex::decode(expected_hex) {
        Ok(b) => b,
        Err(_) => return false,
    };

    let mut mac =
        Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key length");
    mac.update(body);
    let computed = mac.finalize().into_bytes();

    computed.as_slice().ct_eq(&expected_bytes).into()
}

// ---------------------------------------------------------------------------
// Header extraction helpers
// ---------------------------------------------------------------------------

/// Extract event type from well-known headers, falling back to JSON body.
fn extract_event_type(headers: &HeaderMap, body: &[u8]) -> String {
    // GitHub: X-GitHub-Event
    if let Some(val) = headers.get("x-github-event") {
        if let Ok(s) = val.to_str() {
            return s.to_owned();
        }
    }

    // GitLab: X-Gitlab-Event
    if let Some(val) = headers.get("x-gitlab-event") {
        if let Ok(s) = val.to_str() {
            return s.to_owned();
        }
    }

    // Try JSON body `event_type` field.
    if let Ok(parsed) = serde_json::from_slice::<serde_json::Value>(body) {
        if let Some(et) = parsed.get("event_type").and_then(|v| v.as_str()) {
            return et.to_owned();
        }
    }

    "unknown".to_owned()
}

/// Extract a delivery ID for idempotency from well-known headers.
///
/// Falls back to the current timestamp in milliseconds if no header is found.
fn extract_delivery_id(headers: &HeaderMap) -> String {
    // GitHub: X-GitHub-Delivery
    if let Some(val) = headers.get("x-github-delivery") {
        if let Ok(s) = val.to_str() {
            return s.to_owned();
        }
    }

    // Generic: X-Webhook-Id or X-Request-ID
    for name in &["x-webhook-id", "x-request-id"] {
        if let Some(val) = headers.get(*name) {
            if let Ok(s) = val.to_str() {
                return s.to_owned();
            }
        }
    }

    // Fallback: monotonic timestamp (ms precision).
    format!("auto-{}", jiff::Timestamp::now().as_millisecond())
}

// ---------------------------------------------------------------------------
// Route registration
// ---------------------------------------------------------------------------

/// Build an axum [`Router`] with the webhook endpoint.
///
/// Returns a closure compatible with `start_rest_server`'s `route_handlers`
/// parameter. The returned router nests
/// `POST /api/v1/webhooks/:feed_name` under the existing app.
pub fn webhook_routes(state: Arc<WebhookState>) -> impl FnOnce(Router) -> Router {
    move |router: Router| {
        router.route(
            "/api/v1/webhooks/{feed_name}",
            post(webhook_handler).with_state(state),
        )
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    use tokio::sync::mpsc;
    use tower::ServiceExt;

    use super::*;
    use crate::data_feed::{DataFeedConfig, FeedType};

    /// Helper: register a webhook feed in the registry and return the state.
    fn setup_state(secret: Option<&str>) -> (Arc<WebhookState>, mpsc::Receiver<FeedEvent>) {
        let (event_tx, event_rx) = mpsc::channel(16);
        let registry = Arc::new(DataFeedRegistry::new(event_tx.clone()));

        let wh_config = WebhookConfig {
            secret: secret.map(|s| s.to_owned()),
            events: vec![],
        };

        let feed_config = DataFeedConfig {
            name:       "test-hook".to_owned(),
            feed_type:  FeedType::Webhook,
            tags:       vec!["test".to_owned()],
            config:     serde_json::to_value(&wh_config).expect("serialise config"),
            created_at: Timestamp::UNIX_EPOCH,
        };
        registry.register(feed_config).expect("register feed");

        let state = Arc::new(WebhookState::new(registry, event_tx));
        (state, event_rx)
    }

    /// Build a test router wired to the given state.
    fn test_router(state: Arc<WebhookState>) -> Router {
        Router::new().route(
            "/api/v1/webhooks/{feed_name}",
            post(webhook_handler).with_state(state),
        )
    }

    /// Compute GitHub-style HMAC signature header value.
    fn github_signature(secret: &str, body: &[u8]) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("hmac key");
        mac.update(body);
        let result = mac.finalize().into_bytes();
        format!("sha256={}", hex::encode(result))
    }

    // -- Happy path --

    #[tokio::test]
    async fn accepts_valid_webhook_without_secret() {
        let (state, mut rx) = setup_state(None);
        let app = test_router(state);

        let body = serde_json::json!({"action": "opened"});
        let request = Request::builder()
            .method("POST")
            .uri("/api/v1/webhooks/test-hook")
            .header("content-type", "application/json")
            .header("x-github-event", "pull_request")
            .header("x-github-delivery", "delivery-001")
            .body(Body::from(serde_json::to_vec(&body).expect("json")))
            .expect("request");

        let response = app.oneshot(request).await.expect("response");
        assert_eq!(response.status(), StatusCode::ACCEPTED);

        let event = rx.try_recv().expect("should receive event");
        assert_eq!(event.source_name, "test-hook");
        assert_eq!(event.event_type, "pull_request");
    }

    #[tokio::test]
    async fn accepts_valid_webhook_with_github_signature() {
        let secret = "test-secret-123";
        let (state, mut rx) = setup_state(Some(secret));
        let app = test_router(state);

        let body_bytes = br#"{"action":"opened"}"#;
        let sig = github_signature(secret, body_bytes);

        let request = Request::builder()
            .method("POST")
            .uri("/api/v1/webhooks/test-hook")
            .header("content-type", "application/json")
            .header("x-hub-signature-256", sig)
            .header("x-github-delivery", "delivery-002")
            .body(Body::from(body_bytes.to_vec()))
            .expect("request");

        let response = app.oneshot(request).await.expect("response");
        assert_eq!(response.status(), StatusCode::ACCEPTED);

        let event = rx.try_recv().expect("should receive event");
        assert_eq!(event.source_name, "test-hook");
    }

    #[tokio::test]
    async fn accepts_valid_webhook_with_generic_signature() {
        let secret = "generic-secret";
        let (state, mut rx) = setup_state(Some(secret));
        let app = test_router(state);

        let body_bytes = br#"{"type":"test"}"#;
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("hmac key");
        mac.update(body_bytes);
        let sig_hex = hex::encode(mac.finalize().into_bytes());

        let request = Request::builder()
            .method("POST")
            .uri("/api/v1/webhooks/test-hook")
            .header("content-type", "application/json")
            .header("x-webhook-signature", sig_hex)
            .header("x-webhook-id", "gen-001")
            .body(Body::from(body_bytes.to_vec()))
            .expect("request");

        let response = app.oneshot(request).await.expect("response");
        assert_eq!(response.status(), StatusCode::ACCEPTED);

        let event = rx.try_recv().expect("should receive event");
        assert_eq!(event.source_name, "test-hook");
    }

    // -- Rejection cases --

    #[tokio::test]
    async fn rejects_unknown_feed() {
        let (state, _rx) = setup_state(None);
        let app = test_router(state);

        let request = Request::builder()
            .method("POST")
            .uri("/api/v1/webhooks/nonexistent")
            .header("content-type", "application/json")
            .body(Body::from(b"{}".to_vec()))
            .expect("request");

        let response = app.oneshot(request).await.expect("response");
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn rejects_invalid_hmac() {
        let (state, _rx) = setup_state(Some("real-secret"));
        let app = test_router(state);

        let request = Request::builder()
            .method("POST")
            .uri("/api/v1/webhooks/test-hook")
            .header("content-type", "application/json")
            .header("x-hub-signature-256", "sha256=deadbeef")
            .body(Body::from(b"{}".to_vec()))
            .expect("request");

        let response = app.oneshot(request).await.expect("response");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn rejects_missing_signature_when_secret_configured() {
        let (state, _rx) = setup_state(Some("requires-sig"));
        let app = test_router(state);

        let request = Request::builder()
            .method("POST")
            .uri("/api/v1/webhooks/test-hook")
            .header("content-type", "application/json")
            .body(Body::from(b"{}".to_vec()))
            .expect("request");

        let response = app.oneshot(request).await.expect("response");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    // -- Idempotency --

    #[tokio::test]
    async fn deduplicates_retried_deliveries() {
        let (state, _rx) = setup_state(None);
        let app = test_router(state.clone());

        let make_request = || {
            Request::builder()
                .method("POST")
                .uri("/api/v1/webhooks/test-hook")
                .header("content-type", "application/json")
                .header("x-github-delivery", "dedup-001")
                .body(Body::from(b"{}".to_vec()))
                .expect("request")
        };

        // First delivery — accepted.
        let r1 = app.clone().oneshot(make_request()).await.expect("r1");
        assert_eq!(r1.status(), StatusCode::ACCEPTED);

        // Second delivery with same ID — deduplicated.
        let r2 = app.oneshot(make_request()).await.expect("r2");
        assert_eq!(r2.status(), StatusCode::OK);
    }

    // -- HMAC unit tests --

    #[test]
    fn verify_hmac_github_format() {
        let secret = "mysecret";
        let body = b"hello world";
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("key");
        mac.update(body);
        let sig = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));

        let mut headers = HeaderMap::new();
        headers.insert("x-hub-signature-256", sig.parse().expect("header"));

        assert!(verify_hmac(secret, body, &headers));
    }

    #[test]
    fn verify_hmac_rejects_wrong_secret() {
        let body = b"hello world";
        let mut mac = Hmac::<Sha256>::new_from_slice(b"wrong-secret").expect("key");
        mac.update(body);
        let sig = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));

        let mut headers = HeaderMap::new();
        headers.insert("x-hub-signature-256", sig.parse().expect("header"));

        assert!(!verify_hmac("correct-secret", body, &headers));
    }

    #[test]
    fn verify_hmac_no_header_returns_false() {
        let headers = HeaderMap::new();
        assert!(!verify_hmac("secret", b"body", &headers));
    }
}
