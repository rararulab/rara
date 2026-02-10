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

//! Tower middleware that deduplicates concurrent identical HTTP requests.
//!
//! Uses [`moka::future::Cache::get_with`] which provides built-in singleflight
//! semantics: when multiple callers request the same key concurrently, only
//! one executes the upstream call and the rest wait and share the result.

use std::{
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};

use axum::body::Body;
use bytes::Bytes;
use http::{HeaderMap, StatusCode};
use moka::future::Cache;
use tower::{Layer, Service};

use crate::request_key::request_key;

/// Snapshot of an HTTP response that can be cheaply cloned and cached.
#[derive(Clone, Debug)]
struct CachedResponse {
    status:  StatusCode,
    headers: HeaderMap,
    body:    Bytes,
}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for [`DedupLayer`].
#[derive(Clone, Debug)]
pub struct DedupLayerConfig {
    /// How long a cached response stays valid (default 30 s).
    pub ttl:          Duration,
    /// Maximum number of cached entries (default 1000).
    pub max_capacity: u64,
}

impl Default for DedupLayerConfig {
    fn default() -> Self {
        Self {
            ttl:          Duration::from_secs(30),
            max_capacity: 1000,
        }
    }
}

// ---------------------------------------------------------------------------
// Layer
// ---------------------------------------------------------------------------

/// Tower [`Layer`] that wraps a service with request deduplication.
///
/// Apply this to an axum [`Router`](axum::Router) via `.layer(DedupLayer::new(cfg))`.
/// Only the first request for a given key will reach the inner service;
/// concurrent duplicates block and receive a clone of the same response.
#[derive(Clone)]
pub struct DedupLayer {
    cache: Cache<String, Arc<CachedResponse>>,
}

impl DedupLayer {
    /// Create a new `DedupLayer` with the given configuration.
    #[must_use]
    pub fn new(config: DedupLayerConfig) -> Self {
        let cache = Cache::builder()
            .max_capacity(config.max_capacity)
            .time_to_live(config.ttl)
            .build();
        Self { cache }
    }
}

impl<S> Layer<S> for DedupLayer {
    type Service = DedupService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        DedupService {
            inner,
            cache: self.cache.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// Service
// ---------------------------------------------------------------------------

/// The [`Service`] created by [`DedupLayer`].
#[derive(Clone)]
pub struct DedupService<S> {
    inner: S,
    cache: Cache<String, Arc<CachedResponse>>,
}

impl<S> Service<http::Request<Body>> for DedupService<S>
where
    S: Service<http::Request<Body>, Response = http::Response<Body>> + Clone + Send + 'static,
    S::Future: Send,
    S::Error: Send + std::fmt::Debug + 'static,
{
    type Error = S::Error;
    type Future = futures::future::BoxFuture<'static, Result<Self::Response, Self::Error>>;
    type Response = http::Response<Body>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: http::Request<Body>) -> Self::Future {
        // Clone the inner service so the future is 'static.
        let mut inner = self.inner.clone();
        // Swap so `self.inner` keeps the "ready" instance.
        std::mem::swap(&mut self.inner, &mut inner);

        let cache = self.cache.clone();

        Box::pin(async move {
            // Buffer the body so we can compute the key and still forward it.
            let (parts, body) = req.into_parts();
            let body_bytes = axum::body::to_bytes(body, 10 * 1024 * 1024)
                .await
                .unwrap_or_default();

            let key = request_key(
                &parts.method,
                parts.uri.path(),
                parts.uri.query(),
                &body_bytes,
            );

            // `get_with` provides singleflight: only one caller executes the
            // closure; the rest block and receive a clone of the `Arc`.
            let cached = cache
                .get_with(key, async {
                    // Reconstruct the request for the inner service.
                    let req = http::Request::from_parts(parts, Body::from(body_bytes));
                    match inner.call(req).await {
                        Ok(resp) => {
                            let (resp_parts, resp_body) = resp.into_parts();
                            let resp_bytes = axum::body::to_bytes(resp_body, 10 * 1024 * 1024)
                                .await
                                .unwrap_or_default();
                            Arc::new(CachedResponse {
                                status:  resp_parts.status,
                                headers: resp_parts.headers,
                                body:    resp_bytes,
                            })
                        }
                        Err(err) => {
                            // On error we still need to return *something* to cache.
                            // Store a 502 so the next retry can try again after TTL.
                            tracing::warn!(?err, "dedup upstream error, caching 502");
                            Arc::new(CachedResponse {
                                status:  StatusCode::BAD_GATEWAY,
                                headers: HeaderMap::new(),
                                body:    Bytes::from_static(b"upstream error"),
                            })
                        }
                    }
                })
                .await;

            // Reconstruct an HTTP response from the cached snapshot.
            let mut builder = http::Response::builder().status(cached.status);
            if let Some(headers) = builder.headers_mut() {
                headers.extend(
                    cached
                        .headers
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone())),
                );
            }
            let resp = builder
                .body(Body::from(cached.body.clone()))
                .expect("response builder should not fail");
            Ok(resp)
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;

    /// A trivial service that counts how many times it has been called.
    #[derive(Clone)]
    struct CountingService {
        counter: Arc<AtomicU32>,
    }

    impl Service<http::Request<Body>> for CountingService {
        type Error = std::convert::Infallible;
        type Future = futures::future::BoxFuture<'static, Result<Self::Response, Self::Error>>;
        type Response = http::Response<Body>;

        fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn call(&mut self, _req: http::Request<Body>) -> Self::Future {
            let counter = self.counter.clone();
            Box::pin(async move {
                counter.fetch_add(1, Ordering::SeqCst);
                // Simulate a slow upstream.
                tokio::time::sleep(Duration::from_millis(200)).await;
                let resp = http::Response::builder()
                    .status(StatusCode::OK)
                    .header("x-custom", "value")
                    .body(Body::from("ok"))
                    .unwrap();
                Ok(resp)
            })
        }
    }

    fn make_req(path: &str, body: &[u8]) -> http::Request<Body> {
        http::Request::builder()
            .method(http::Method::POST)
            .uri(path)
            .body(Body::from(Bytes::copy_from_slice(body)))
            .unwrap()
    }

    #[tokio::test]
    async fn concurrent_same_key_calls_upstream_once() {
        let counter = Arc::new(AtomicU32::new(0));
        let svc = CountingService {
            counter: counter.clone(),
        };
        let layer = DedupLayer::new(DedupLayerConfig {
            ttl:          Duration::from_secs(5),
            max_capacity: 100,
        });
        let dedup_svc = layer.layer(svc);

        // Spawn 5 concurrent requests with the same key.
        let mut handles = Vec::new();
        for _ in 0..5 {
            let mut svc = dedup_svc.clone();
            handles.push(tokio::spawn(async move {
                let req = make_req("/api/v1/jobs/discover", b"{\"q\":\"rust\"}");
                svc.call(req).await
            }));
        }

        let mut responses = Vec::new();
        for h in handles {
            responses.push(h.await.unwrap().unwrap());
        }

        // The inner service must have been called exactly once.
        assert_eq!(counter.load(Ordering::SeqCst), 1);
        // All responses must be 200 OK.
        for resp in &responses {
            assert_eq!(resp.status(), StatusCode::OK);
        }
    }

    #[tokio::test]
    async fn different_keys_call_upstream_separately() {
        let counter = Arc::new(AtomicU32::new(0));
        let svc = CountingService {
            counter: counter.clone(),
        };
        let layer = DedupLayer::new(DedupLayerConfig {
            ttl:          Duration::from_secs(5),
            max_capacity: 100,
        });
        let mut dedup_svc = layer.layer(svc);

        // Two sequential requests with different bodies.
        let req_a = make_req("/api/v1/jobs/discover", b"{\"q\":\"rust\"}");
        let _ = dedup_svc.call(req_a).await.unwrap();
        let req_b = make_req("/api/v1/jobs/discover", b"{\"q\":\"python\"}");
        let _ = dedup_svc.call(req_b).await.unwrap();

        assert_eq!(counter.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn cached_response_preserves_headers_and_body() {
        let counter = Arc::new(AtomicU32::new(0));
        let svc = CountingService {
            counter: counter.clone(),
        };
        let layer = DedupLayer::new(DedupLayerConfig {
            ttl:          Duration::from_secs(5),
            max_capacity: 100,
        });
        let mut dedup_svc = layer.layer(svc);

        // First call populates the cache.
        let req = make_req("/test", b"body");
        let resp = dedup_svc.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers().get("x-custom").unwrap(), "value");

        let body_bytes = axum::body::to_bytes(resp.into_body(), 1024)
            .await
            .unwrap();
        assert_eq!(&body_bytes[..], b"ok");

        // Second call (same key) returns cached response.
        let req = make_req("/test", b"body");
        let resp = dedup_svc.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers().get("x-custom").unwrap(), "value");

        // Only called once.
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }
}
