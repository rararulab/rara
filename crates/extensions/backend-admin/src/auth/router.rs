use axum::{Json, extract::State};
use tracing::instrument;
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::auth::{
    error::AuthError,
    service::{AuthService, LoginRequest, LoginResponse},
};

pub fn routes(service: AuthService) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(login))
        .with_state(service)
}

#[utoipa::path(
    post,
    path = "/api/v1/auth/login",
    tag = "auth",
    request_body = LoginRequest,
    responses(
        (status = 200, description = "Successful login", body = LoginResponse),
        (status = 400, description = "Invalid request body"),
        (status = 401, description = "Invalid credentials"),
        (status = 403, description = "Email not verified"),
        (status = 429, description = "Account locked"),
    )
)]
#[instrument(skip(service, request))]
async fn login(
    State(service): State<AuthService>,
    Json(request): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, AuthError> {
    Ok(Json(service.login(request).await?))
}
