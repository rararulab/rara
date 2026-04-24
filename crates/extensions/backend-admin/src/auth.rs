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

//! Bearer-token authentication middleware and the `/whoami` endpoint.
//!
//! Every admin HTTP route is expected to run under [`auth_layer`], which
//! resolves the caller into a kernel [`Principal<Resolved>`] and inserts it
//! into request extensions. Downstream handlers extract the principal with
//! `Extension<Principal<Resolved>>` and use `is_admin()` / `role()` /
//! `has_permission()` for fine-grained checks.
//!
//! Startup is responsible for guaranteeing the configured `owner_user_id`
//! resolves (see `rara_app::validate_owner_auth`); a resolve failure at
//! request time is therefore treated as a 500, not a 401, because it
//! indicates operator misconfiguration rather than a bad caller.

use std::sync::Arc;

use axum::{
    Json,
    extract::{Extension, Request, State},
    http::{HeaderMap, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use rara_kernel::{
    handle::KernelHandle,
    identity::{Principal, Resolved},
    security::SecurityRef,
};
use serde::Serialize;
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::kernel::problem::ProblemDetails;

/// Shared state for the auth middleware and `/whoami` route.
///
/// Cloned by axum for every request; keep the contents cheap (`Arc<str>`
/// for the secret, owned `String` for the resolved username, plus the
/// already-`Clone` kernel handles).
#[derive(Clone)]
pub struct AuthState {
    owner_token:   Arc<str>,
    owner_user_id: String,
    security:      SecurityRef,
}

impl AuthState {
    /// Build an [`AuthState`] from startup config and a running kernel
    /// handle. The caller guarantees `owner_user_id` has already been
    /// validated by `rara_app::validate_owner_auth`.
    pub fn new(owner_token: String, owner_user_id: String, handle: &KernelHandle) -> Self {
        Self {
            owner_token: Arc::from(owner_token),
            owner_user_id,
            security: handle.security().clone(),
        }
    }
}

/// Axum middleware enforcing `Authorization: Bearer <owner_token>`.
///
/// On success it attaches an `Extension<Principal<Resolved>>` to the request
/// so downstream handlers can perform authorization checks without another
/// lookup. On failure it returns an RFC 9457 problem+json document.
pub async fn auth_layer(
    State(state): State<AuthState>,
    headers: HeaderMap,
    mut request: Request,
    next: Next,
) -> Response {
    let Some(provided) = bearer_from_headers(&headers) else {
        return ProblemDetails::unauthorized("missing or malformed Authorization header")
            .into_response();
    };

    if !rara_kernel::auth::verify_owner_token(&state.owner_token, provided) {
        return ProblemDetails::unauthorized("invalid owner token").into_response();
    }

    let principal = match state
        .security
        .resolve_principal(&Principal::lookup(state.owner_user_id.clone()))
        .await
    {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(
                error = %e,
                owner_user_id = %state.owner_user_id,
                "owner_user_id failed to resolve; check startup validation"
            );
            return ProblemDetails::internal(format!("owner principal unavailable: {e}"))
                .into_response();
        }
    };

    request.extensions_mut().insert(principal);
    next.run(request).await
}

/// Extract a Bearer token from the `Authorization` header.
///
/// Accepts case-insensitive `Bearer` / `bearer` schemes per RFC 6750 §2.1.
/// Returns `None` for missing, non-UTF-8, or malformed values.
fn bearer_from_headers(headers: &HeaderMap) -> Option<&str> {
    let raw = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let token = raw
        .strip_prefix("Bearer ")
        .or_else(|| raw.strip_prefix("bearer "))?;
    (!token.is_empty()).then_some(token)
}

// ---------------------------------------------------------------------------
// /whoami
// ---------------------------------------------------------------------------

/// Response body for `GET /api/v1/whoami`.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct WhoamiResponse {
    /// Resolved kernel username.
    pub user_id:  String,
    /// Principal role: `Root` | `Admin` | `User`.
    pub role:     String,
    /// Whether the caller has admin-or-higher privileges.
    pub is_admin: bool,
}

/// Build routes that require authentication but also expose the caller's
/// resolved identity.
pub fn routes() -> OpenApiRouter { OpenApiRouter::new().routes(routes!(whoami)) }

/// Return the resolved principal for the authenticated caller.
#[utoipa::path(
    get,
    path = "/api/v1/whoami",
    tag = "auth",
    responses(
        (status = 200, description = "Authenticated principal", body = WhoamiResponse),
        (status = 401, description = "Missing or invalid bearer token", body = ()),
    ),
    security(("bearer_auth" = []))
)]
async fn whoami(Extension(principal): Extension<Principal<Resolved>>) -> Json<WhoamiResponse> {
    Json(WhoamiResponse {
        user_id:  principal.user_id.0.clone(),
        role:     format!("{:?}", principal.role()),
        is_admin: principal.is_admin(),
    })
}

// ---------------------------------------------------------------------------
// 401 helper on ProblemDetails
// ---------------------------------------------------------------------------

impl ProblemDetails {
    /// Build a 401 `problem+json` response.
    pub fn unauthorized(detail: impl Into<String>) -> Self {
        Self {
            problem_type: "https://rara.dev/problems/unauthorized".to_string(),
            title:        "Unauthorized".to_string(),
            status:       StatusCode::UNAUTHORIZED.as_u16(),
            detail:       Some(detail.into()),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use axum::{
        Router,
        body::{Body, to_bytes},
        http::{Request, StatusCode},
        middleware,
        routing::get,
    };
    use rara_kernel::{
        error::Result as KernelResult,
        identity::{KernelUser, Permission, Principal, Resolved, Role, UserStore},
        security::{ApprovalManager, ApprovalPolicy, SecuritySubsystem},
    };
    use tower::ServiceExt;

    use super::{AuthState, auth_layer};

    struct TestUserStore {
        user: KernelUser,
    }

    #[async_trait]
    impl UserStore for TestUserStore {
        async fn get_by_name(&self, name: &str) -> KernelResult<Option<KernelUser>> {
            Ok((name == self.user.name).then(|| self.user.clone()))
        }

        async fn list(&self) -> KernelResult<Vec<KernelUser>> { Ok(vec![self.user.clone()]) }
    }

    fn admin_user() -> KernelUser {
        KernelUser {
            name:        "admin".into(),
            role:        Role::Admin,
            permissions: vec![Permission::All],
            enabled:     true,
        }
    }

    fn auth_state(token: &str, owner_user_id: &str) -> AuthState {
        let store: Arc<dyn UserStore> = Arc::new(TestUserStore { user: admin_user() });
        let approval = Arc::new(ApprovalManager::new(ApprovalPolicy::default()));
        let security = Arc::new(SecuritySubsystem::new(store, approval));
        AuthState {
            owner_token: Arc::from(token),
            owner_user_id: owner_user_id.to_owned(),
            security,
        }
    }

    /// Protected test handler — echoes the resolved principal's user_id.
    async fn echo_principal(
        axum::extract::Extension(p): axum::extract::Extension<Principal<Resolved>>,
    ) -> String {
        p.user_id.0
    }

    fn app(state: AuthState) -> Router {
        Router::new()
            .route("/protected", get(echo_principal))
            .layer(middleware::from_fn_with_state(state, auth_layer))
    }

    async fn body_str(res: axum::response::Response) -> String {
        let bytes = to_bytes(res.into_body(), 4096).await.unwrap();
        String::from_utf8(bytes.to_vec()).unwrap_or_default()
    }

    #[tokio::test]
    async fn accepts_valid_bearer_and_injects_principal() {
        let app = app(auth_state("s3cret", "admin"));
        let res = app
            .oneshot(
                Request::builder()
                    .uri("/protected")
                    .header("Authorization", "Bearer s3cret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(body_str(res).await, "admin");
    }

    #[tokio::test]
    async fn rejects_missing_authorization_header() {
        let app = app(auth_state("s3cret", "admin"));
        let res = app
            .oneshot(
                Request::builder()
                    .uri("/protected")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn rejects_wrong_bearer_token() {
        let app = app(auth_state("s3cret", "admin"));
        let res = app
            .oneshot(
                Request::builder()
                    .uri("/protected")
                    .header("Authorization", "Bearer nope")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn rejects_malformed_authorization_header() {
        let app = app(auth_state("s3cret", "admin"));
        let res = app
            .oneshot(
                Request::builder()
                    .uri("/protected")
                    .header("Authorization", "Basic s3cret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn whoami_round_trip_returns_principal_fields() {
        // Integration test: layer the production `routes()` under the real
        // middleware and hit `/api/v1/whoami` end-to-end.
        let state = auth_state("s3cret", "admin");
        let (whoami_router, _api) = super::routes().split_for_parts();
        let app: Router = Router::new()
            .merge(whoami_router)
            .layer(middleware::from_fn_with_state(state, auth_layer));
        let res = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/whoami")
                    .header("Authorization", "Bearer s3cret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = body_str(res).await;
        let json: serde_json::Value = serde_json::from_str(&body).expect("json");
        assert_eq!(json["user_id"], "admin");
        assert_eq!(json["role"], "Admin");
        assert_eq!(json["is_admin"], true);
    }

    #[tokio::test]
    async fn fails_closed_when_owner_user_unresolvable() {
        // Misconfiguration: owner_user_id references a user who no longer
        // exists in the store — the middleware must return 500, not 401.
        let app = app(auth_state("s3cret", "ghost"));
        let res = app
            .oneshot(
                Request::builder()
                    .uri("/protected")
                    .header("Authorization", "Bearer s3cret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }
}
