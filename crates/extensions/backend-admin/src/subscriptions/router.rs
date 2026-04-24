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

//! Subscription REST API.
//!
//! | Method | Path                                    | Description                      |
//! |--------|-----------------------------------------|----------------------------------|
//! | GET    | `/api/v1/subscriptions`                 | list subscriptions (`?owner=`)   |
//! | POST   | `/api/v1/subscriptions`                 | create subscription              |
//! | PATCH  | `/api/v1/subscriptions/{id}`            | update tags / on_receive         |
//! | DELETE | `/api/v1/subscriptions/{id}`            | unsubscribe (admin + audit)      |
//!
//! The `subscriber` session key on create is supplied by the caller — the
//! admin UI lets an operator bind a subscription to any existing session
//! (typically their own) so matching `FeedEvent`s fan out to that session.

use axum::{
    Extension, Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, patch},
};
use rara_kernel::{
    identity::{Principal, Resolved, UserId},
    notification::{NotifyAction, Subscription, SubscriptionRegistryRef},
    session::SessionKey,
};
use serde::{Deserialize, Serialize};
use tracing::info;
use uuid::Uuid;

use crate::kernel::problem::ProblemDetails;

/// Shared state for subscription routes.
///
/// Wraps the kernel's in-memory registry so the HTTP handlers can persist
/// new subscriptions without owning their own store — the JSON file
/// backing the registry remains the single source of truth.
#[derive(Clone)]
pub struct SubscriptionRouterState {
    /// Kernel subscription registry (shared Arc).
    pub registry: SubscriptionRegistryRef,
}

/// Build the `/api/v1/subscriptions/...` router.
pub fn subscription_routes(state: SubscriptionRouterState) -> Router {
    Router::new()
        .route(
            "/api/v1/subscriptions",
            get(list_subscriptions).post(create_subscription),
        )
        .route(
            "/api/v1/subscriptions/{id}",
            patch(update_subscription).delete(delete_subscription),
        )
        .with_state(state)
}

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

/// Wire representation of a [`Subscription`] — mirrors the kernel struct
/// but stringifies the session key for JSON ergonomics.
#[derive(Debug, Serialize)]
struct SubscriptionDto {
    id:         Uuid,
    subscriber: String,
    owner:      String,
    match_tags: Vec<String>,
    on_receive: NotifyAction,
}

impl From<Subscription> for SubscriptionDto {
    fn from(sub: Subscription) -> Self {
        Self {
            id:         sub.id,
            subscriber: sub.subscriber.to_string(),
            owner:      sub.owner.0,
            match_tags: sub.match_tags,
            on_receive: sub.on_receive,
        }
    }
}

/// `GET /api/v1/subscriptions?owner=<user>` — optional owner filter.
#[derive(Debug, Deserialize)]
struct ListQuery {
    owner: Option<String>,
}

/// Body of `POST /api/v1/subscriptions`.
#[derive(Debug, Deserialize)]
struct CreateSubscriptionRequest {
    /// Session key (UUID) that will receive matching notifications.
    subscriber: String,
    /// Owner user ID — defaults to the authenticated caller when omitted.
    owner:      Option<String>,
    match_tags: Vec<String>,
    on_receive: NotifyAction,
}

/// Body of `PATCH /api/v1/subscriptions/{id}`.
#[derive(Debug, Deserialize)]
struct UpdateSubscriptionRequest {
    match_tags: Option<Vec<String>>,
    on_receive: Option<NotifyAction>,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /api/v1/subscriptions` — list subscriptions (optionally filtered).
async fn list_subscriptions(
    State(state): State<SubscriptionRouterState>,
    Query(params): Query<ListQuery>,
) -> Json<Vec<SubscriptionDto>> {
    let owner = params.owner.map(UserId);
    let subs = state.registry.list_all(owner.as_ref()).await;
    Json(subs.into_iter().map(SubscriptionDto::from).collect())
}

/// `POST /api/v1/subscriptions` — register a new subscription.
async fn create_subscription(
    State(state): State<SubscriptionRouterState>,
    Extension(principal): Extension<Principal<Resolved>>,
    Json(body): Json<CreateSubscriptionRequest>,
) -> Result<(StatusCode, Json<SubscriptionDto>), ProblemDetails> {
    if body.match_tags.is_empty() {
        return Err(ProblemDetails::bad_request(
            "match_tags must contain at least one tag",
        ));
    }

    let subscriber = SessionKey::try_from_raw(&body.subscriber)
        .map_err(|e| ProblemDetails::bad_request(format!("invalid subscriber session key: {e}")))?;

    // Default the owner to the authenticated caller. An explicit owner is
    // accepted to support operator workflows (e.g. creating a subscription
    // on behalf of another user), but it is still audited below.
    let owner = body
        .owner
        .map(UserId)
        .unwrap_or_else(|| principal.user_id.clone());

    let id = state
        .registry
        .subscribe(
            subscriber,
            owner.clone(),
            body.match_tags.clone(),
            body.on_receive,
        )
        .await;

    info!(
        actor = %principal.user_id,
        subscription_id = %id,
        owner = %owner,
        tags = ?body.match_tags,
        action = ?body.on_receive,
        "create_subscription"
    );

    let created = Subscription {
        id,
        subscriber,
        owner,
        match_tags: body.match_tags,
        on_receive: body.on_receive,
    };
    Ok((StatusCode::CREATED, Json(SubscriptionDto::from(created))))
}

/// `PATCH /api/v1/subscriptions/{id}` — update tags and/or action.
async fn update_subscription(
    State(state): State<SubscriptionRouterState>,
    Extension(principal): Extension<Principal<Resolved>>,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateSubscriptionRequest>,
) -> Result<Json<SubscriptionDto>, ProblemDetails> {
    if body.match_tags.is_none() && body.on_receive.is_none() {
        return Err(ProblemDetails::bad_request(
            "at least one of match_tags or on_receive must be provided",
        ));
    }

    if let Some(ref tags) = body.match_tags {
        if tags.is_empty() {
            return Err(ProblemDetails::bad_request(
                "match_tags must contain at least one tag",
            ));
        }
    }

    let updated = state
        .registry
        .update(id, body.match_tags, body.on_receive)
        .await
        .ok_or_else(|| {
            ProblemDetails::not_found(
                "Subscription Not Found",
                format!("no subscription with id: {id}"),
            )
        })?;

    info!(
        actor = %principal.user_id,
        subscription_id = %id,
        "update_subscription"
    );

    Ok(Json(SubscriptionDto::from(updated)))
}

/// `DELETE /api/v1/subscriptions/{id}` — admin-only unsubscribe with audit.
async fn delete_subscription(
    State(state): State<SubscriptionRouterState>,
    Extension(principal): Extension<Principal<Resolved>>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ProblemDetails> {
    if !principal.is_admin() {
        return Err(ProblemDetails::forbidden(
            "deleting subscriptions requires admin role",
        ));
    }

    info!(
        actor = %principal.user_id,
        subscription_id = %id,
        "delete_subscription"
    );

    match state.registry.admin_unsubscribe(id).await {
        Some(_) => Ok(StatusCode::NO_CONTENT),
        None => Err(ProblemDetails::not_found(
            "Subscription Not Found",
            format!("no subscription with id: {id}"),
        )),
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
    };
    use rara_kernel::{
        error::Result as KernelResult,
        identity::{KernelUser, Permission, Role, UserStore},
        notification::SubscriptionRegistry,
        security::{ApprovalManager, ApprovalPolicy, SecuritySubsystem},
        session::SessionKey,
    };
    use tempfile::TempDir;
    use tower::ServiceExt;

    use super::*;
    use crate::auth::{AuthState, auth_layer};

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

    fn user_of(role: Role) -> KernelUser {
        KernelUser {
            name: match role {
                Role::Admin | Role::Root => "admin".into(),
                Role::User => "alice".into(),
            },
            role,
            permissions: match role {
                Role::Admin | Role::Root => vec![Permission::All],
                // Non-admin callers still need Spawn to resolve through the
                // security subsystem — matches production user seeding.
                Role::User => vec![Permission::Spawn],
            },
            enabled: true,
        }
    }

    fn auth_state_direct(user: KernelUser) -> AuthState {
        let name = user.name.clone();
        let store: Arc<dyn UserStore> = Arc::new(TestUserStore { user });
        let approval = Arc::new(ApprovalManager::new(ApprovalPolicy::default()));
        let security = Arc::new(SecuritySubsystem::new(store, approval));
        AuthState::for_tests("s3cret", &name, security)
    }

    fn test_registry() -> (SubscriptionRegistryRef, TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("subscriptions.json");
        (Arc::new(SubscriptionRegistry::load(path)), tmp)
    }

    fn app(user: KernelUser) -> (Router, SubscriptionRegistryRef, TempDir) {
        let (registry, tmp) = test_registry();
        let state = SubscriptionRouterState {
            registry: registry.clone(),
        };
        let auth = auth_state_direct(user);
        let router =
            subscription_routes(state).layer(middleware::from_fn_with_state(auth, auth_layer));
        (router, registry, tmp)
    }

    async fn body_bytes(res: axum::response::Response) -> Vec<u8> {
        to_bytes(res.into_body(), 64 * 1024).await.unwrap().to_vec()
    }

    #[tokio::test]
    async fn rejects_missing_auth() {
        let (app, _reg, _tmp) = app(user_of(Role::Admin));
        let res = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/subscriptions")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn create_list_and_admin_delete_round_trip() {
        let (app, registry, _tmp) = app(user_of(Role::Admin));
        let session = SessionKey::new();

        let body = serde_json::json!({
            "subscriber": session.to_string(),
            "match_tags": ["news.aapl"],
            "on_receive": "proactive_turn",
        });
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/subscriptions")
                    .header("Authorization", "Bearer s3cret")
                    .header("Content-Type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::CREATED);
        let created: serde_json::Value = serde_json::from_slice(&body_bytes(res).await).unwrap();
        let id = created["id"].as_str().unwrap().to_owned();
        assert_eq!(created["match_tags"][0], "news.aapl");
        assert_eq!(created["on_receive"], "proactive_turn");

        // Registry reflects the new subscription.
        assert_eq!(registry.list_all(None).await.len(), 1);

        // List endpoint returns it.
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/subscriptions")
                    .header("Authorization", "Bearer s3cret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let listed: serde_json::Value = serde_json::from_slice(&body_bytes(res).await).unwrap();
        assert_eq!(listed.as_array().unwrap().len(), 1);

        // Admin can delete.
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/api/v1/subscriptions/{id}"))
                    .header("Authorization", "Bearer s3cret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NO_CONTENT);
        assert!(registry.list_all(None).await.is_empty());
    }

    #[tokio::test]
    async fn non_admin_cannot_delete() {
        let (app, registry, _tmp) = app(user_of(Role::User));
        // Seed a subscription directly via the registry.
        let id = registry
            .subscribe(
                SessionKey::new(),
                UserId("alice".into()),
                vec!["x".into()],
                NotifyAction::SilentAppend,
            )
            .await;

        let res = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/api/v1/subscriptions/{id}"))
                    .header("Authorization", "Bearer s3cret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
        assert_eq!(registry.list_all(None).await.len(), 1);
    }

    #[tokio::test]
    async fn patch_updates_tags_and_action() {
        let (app, registry, _tmp) = app(user_of(Role::Admin));
        let id = registry
            .subscribe(
                SessionKey::new(),
                UserId("admin".into()),
                vec!["old".into()],
                NotifyAction::SilentAppend,
            )
            .await;

        let body = serde_json::json!({
            "match_tags": ["new", "news.aapl"],
            "on_receive": "proactive_turn",
        });
        let res = app
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri(format!("/api/v1/subscriptions/{id}"))
                    .header("Authorization", "Bearer s3cret")
                    .header("Content-Type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let updated: serde_json::Value = serde_json::from_slice(&body_bytes(res).await).unwrap();
        assert_eq!(updated["match_tags"][1], "news.aapl");
        assert_eq!(updated["on_receive"], "proactive_turn");
    }

    #[tokio::test]
    async fn create_rejects_empty_tags() {
        let (app, _reg, _tmp) = app(user_of(Role::Admin));
        let body = serde_json::json!({
            "subscriber": SessionKey::new().to_string(),
            "match_tags": [],
            "on_receive": "silent_append",
        });
        let res = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/subscriptions")
                    .header("Authorization", "Bearer s3cret")
                    .header("Content-Type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }
}
