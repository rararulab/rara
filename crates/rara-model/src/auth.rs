use serde::{Deserialize, Serialize};
use sqlx::FromRow;

pub const AUTH_PROVIDER_GITHUB: &str = "github";
pub const AUTH_PROVIDER_GOOGLE: &str = "google";

/// Persistent login credentials and account state for a kernel user.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct AuthUserRecord {
    pub user_id: String,
    pub email: String,
    pub password_hash: Option<String>,
    pub email_verified_at: Option<String>,
    pub failed_login_attempts: i64,
    pub locked_until: Option<String>,
    pub last_login_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// External OAuth identity linked to a local kernel user.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct AuthOAuthAccountRecord {
    pub id: String,
    pub user_id: String,
    pub provider: String,
    pub provider_user_id: String,
    pub provider_email: Option<String>,
    pub provider_login: Option<String>,
    pub last_login_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}
