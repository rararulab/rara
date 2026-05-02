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

//! BDD binding for `specs/issue-2043-session-status.spec.md` scenario
//! "invalid status query parameter returns 400 with the allowed list".
//!
//! The full chat router needs a tape store, FTS pool, settings provider,
//! and a model lister to construct — overhead that buys nothing for a
//! query-string validation check. The test wires the production
//! `ListSessionsQuery` extractor into a one-handler router and invokes
//! the same `parse_status_filter` the HTTP layer calls, so a future
//! drift in the parser surfaces here.
//!
//! `Package: rara-app`, `Filter: list_sessions_rejects_unknown_status`.

use axum::{
    Router,
    body::Body,
    extract::Query,
    http::{Request, StatusCode},
    response::IntoResponse,
    routing::get,
};
use rara_backend_admin::chat::{ListSessionsQuery, parse_status_filter};
use tower::ServiceExt;

async fn handler(Query(q): Query<ListSessionsQuery>) -> impl IntoResponse {
    match parse_status_filter(q.status.as_deref()) {
        Ok(_) => StatusCode::OK.into_response(),
        Err(e) => e.into_response(),
    }
}

#[tokio::test]
async fn list_sessions_rejects_unknown_status() {
    let app = Router::new().route("/api/v1/chat/sessions", get(handler));

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/chat/sessions?status=banana")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "?status=banana must reject with 400"
    );

    let body_bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("body");
    let body_text = std::str::from_utf8(&body_bytes).expect("utf8");

    // The error body must enumerate the accepted values so a
    // mistyping client sees the fix without grepping the source.
    for allowed in ["active", "archived", "all"] {
        assert!(
            body_text.contains(allowed),
            "error body must list `{allowed}` as an allowed value, got: {body_text}"
        );
    }
}

#[tokio::test]
async fn list_sessions_accepts_known_status() {
    // Negative-control: the parser must NOT reject the three valid
    // strings — otherwise the 400-on-banana assertion is vacuous.
    let app = Router::new().route("/api/v1/chat/sessions", get(handler));

    for value in ["active", "archived", "all"] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/api/v1/chat/sessions?status={value}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "?status={value} must be accepted"
        );
    }
}
