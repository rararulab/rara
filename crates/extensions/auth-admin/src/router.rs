use axum::{Json, http::StatusCode};
use utoipa_axum::router::OpenApiRouter;

#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct SshKeyResponse {
    pub public_key: String,
}

pub fn routes() -> OpenApiRouter {
    OpenApiRouter::new().route("/api/v1/auth/ssh-key", axum::routing::get(get_ssh_key))
}

#[utoipa::path(
    get,
    path = "/api/v1/auth/ssh-key",
    tag = "auth-admin",
    responses((status = 200, description = "SSH public key", body = SshKeyResponse))
)]
async fn get_ssh_key() -> Result<Json<SshKeyResponse>, (StatusCode, String)> {
    let ssh_dir = rara_paths::data_dir().join("ssh");
    let public_key = rara_git::get_public_key(&ssh_dir)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(SshKeyResponse { public_key }))
}
