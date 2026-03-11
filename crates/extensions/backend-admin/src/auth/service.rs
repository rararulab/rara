use std::time::Duration;

use bcrypt::verify;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use jsonwebtoken::{EncodingKey, Header, encode};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

use crate::auth::error::AuthError;

#[derive(Debug, Clone)]
pub struct AuthService {
    pool: SqlitePool,
    config: AuthConfig,
}

impl AuthService {
    #[must_use]
    pub fn new(pool: SqlitePool, config: AuthConfig) -> Self {
        Self { pool, config }
    }

    pub async fn login(&self, request: LoginRequest) -> Result<LoginResponse, AuthError> {
        let email = request.email.trim().to_lowercase();
        let password = request.password;

        if email.is_empty() {
            return Err(AuthError::InvalidRequest {
                message: "email is required".to_owned(),
            });
        }
        if password.is_empty() {
            return Err(AuthError::InvalidRequest {
                message: "password is required".to_owned(),
            });
        }

        let Some(secret) = self.config.jwt.secret.as_deref() else {
            return Err(AuthError::NotConfigured);
        };

        let record = sqlx::query_as::<_, AuthLoginRecord>(
            r#"
            SELECT
                kernel_users.id,
                kernel_users.name,
                kernel_users.role,
                kernel_users.enabled,
                auth_users.email,
                auth_users.password_hash,
                auth_users.email_verified_at,
                auth_users.failed_login_attempts,
                auth_users.locked_until
            FROM auth_users
            INNER JOIN kernel_users ON kernel_users.id = auth_users.user_id
            WHERE auth_users.email = ?1
            "#,
        )
        .bind(&email)
        .fetch_optional(&self.pool)
        .await
        .map_err(internal_error)?;

        let Some(record) = record else {
            return Err(AuthError::InvalidCredentials);
        };

        if !record.enabled {
            return Err(AuthError::InvalidCredentials);
        }

        if is_locked(record.locked_until.as_deref())? {
            return Err(AuthError::AccountLocked);
        }

        if self.config.password.require_email_verification && record.email_verified_at.is_none() {
            return Err(AuthError::EmailNotVerified);
        }

        let Some(password_hash) = record.password_hash.as_deref() else {
            return Err(AuthError::InvalidCredentials);
        };

        let password_valid = verify(&password, password_hash).map_err(internal_error)?;
        if !password_valid {
            self.record_failed_attempt(&record.id, record.failed_login_attempts)
                .await?;
            return if record.failed_login_attempts + 1
                >= i64::from(self.config.password.max_attempts)
            {
                Err(AuthError::AccountLocked)
            } else {
                Err(AuthError::InvalidCredentials)
            };
        }

        let now = Utc::now();
        sqlx::query(
            r#"
            UPDATE auth_users
            SET failed_login_attempts = 0,
                locked_until = NULL,
                last_login_at = ?2
            WHERE user_id = ?1
            "#,
        )
        .bind(&record.id)
        .bind(format_sqlite_timestamp(now))
        .execute(&self.pool)
        .await
        .map_err(internal_error)?;

        let token = issue_jwt(&record, &self.config, secret, now)?;

        Ok(LoginResponse {
            token,
            user: AuthenticatedUser {
                id: record.id,
                name: record.name,
                email: record.email,
                role: record.role,
            },
        })
    }

    async fn record_failed_attempt(
        &self,
        user_id: &str,
        previous_attempts: i64,
    ) -> Result<(), AuthError> {
        let attempts = previous_attempts + 1;
        let locked_until = if attempts >= i64::from(self.config.password.max_attempts) {
            Some(format_sqlite_timestamp(
                Utc::now() + duration_to_chrono(self.config.password.lockout_window)?,
            ))
        } else {
            None
        };

        sqlx::query(
            r#"
            UPDATE auth_users
            SET failed_login_attempts = ?2,
                locked_until = ?3
            WHERE user_id = ?1
            "#,
        )
        .bind(user_id)
        .bind(attempts)
        .bind(locked_until)
        .execute(&self.pool)
        .await
        .map_err(internal_error)?;

        Ok(())
    }
}

#[derive(Debug, Deserialize, Serialize, utoipa::ToSchema)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct LoginResponse {
    pub token: String,
    pub user: AuthenticatedUser,
}

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct AuthenticatedUser {
    pub id: String,
    pub name: String,
    pub email: String,
    pub role: i64,
}

#[derive(Debug, Clone)]
pub struct AuthConfig {
    pub jwt: JwtConfig,
    pub password: PasswordAuthConfig,
}

#[derive(Debug, Clone)]
pub struct JwtConfig {
    pub issuer: String,
    pub audience: String,
    pub secret: Option<String>,
    pub access_token_ttl: Duration,
}

#[derive(Debug, Clone)]
pub struct PasswordAuthConfig {
    pub max_attempts: u32,
    pub require_email_verification: bool,
    pub lockout_window: Duration,
}

#[derive(Debug, Serialize, Deserialize)]
struct JwtClaims {
    sub: String,
    name: String,
    email: String,
    role: i64,
    iss: String,
    aud: String,
    iat: i64,
    exp: i64,
}

#[derive(Debug, FromRow)]
struct AuthLoginRecord {
    id: String,
    name: String,
    role: i64,
    enabled: bool,
    email: String,
    password_hash: Option<String>,
    email_verified_at: Option<String>,
    failed_login_attempts: i64,
    locked_until: Option<String>,
}

fn issue_jwt(
    record: &AuthLoginRecord,
    config: &AuthConfig,
    secret: &str,
    now: DateTime<Utc>,
) -> Result<String, AuthError> {
    let ttl = duration_to_chrono(config.jwt.access_token_ttl)?;
    let claims = JwtClaims {
        sub: record.id.clone(),
        name: record.name.clone(),
        email: record.email.clone(),
        role: record.role,
        iss: config.jwt.issuer.clone(),
        aud: config.jwt.audience.clone(),
        iat: now.timestamp(),
        exp: (now + ttl).timestamp(),
    };
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(internal_error)
}

fn is_locked(locked_until: Option<&str>) -> Result<bool, AuthError> {
    let Some(locked_until) = locked_until else {
        return Ok(false);
    };
    Ok(parse_sqlite_timestamp(locked_until)? > Utc::now())
}

fn parse_sqlite_timestamp(value: &str) -> Result<DateTime<Utc>, AuthError> {
    let naive =
        chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S").map_err(|_| {
            AuthError::Internal {
                message: format!("invalid timestamp stored in auth_users: {value}"),
            }
        })?;
    Ok(DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc))
}

fn format_sqlite_timestamp(value: DateTime<Utc>) -> String {
    value.format("%Y-%m-%d %H:%M:%S").to_string()
}

fn duration_to_chrono(duration: Duration) -> Result<ChronoDuration, AuthError> {
    ChronoDuration::from_std(duration).map_err(|err| AuthError::Internal {
        message: format!("invalid auth duration: {err}"),
    })
}

fn internal_error<E: std::fmt::Display>(error: E) -> AuthError {
    AuthError::Internal {
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use axum::{
        Router,
        body::Body,
        http::{Request, StatusCode},
    };
    use bcrypt::hash;
    use serde_json::{Value, json};
    use sqlx::{Row, sqlite::SqlitePoolOptions};
    use tower::ServiceExt;

    use super::*;

    #[tokio::test]
    async fn login_succeeds_with_verified_email() {
        let pool = test_pool().await;
        insert_user(
            &pool,
            "user-1",
            "Alice",
            "alice@example.com",
            "correct-horse",
            Some("2026-03-12 00:00:00"),
            0,
            None,
        )
        .await;
        let app = test_app(pool.clone()).await;

        let response = app
            .oneshot(
                Request::post("/api/v1/auth/login")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "email": "alice@example.com",
                            "password": "correct-horse"
                        })
                        .to_string(),
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        let body = body_json(response).await;
        assert!(
            body["token"]
                .as_str()
                .is_some_and(|token| !token.is_empty())
        );
        assert_eq!(body["user"]["email"], "alice@example.com");

        let row = sqlx::query(
            "SELECT failed_login_attempts, locked_until, last_login_at FROM auth_users WHERE user_id = ?1",
        )
        .bind("user-1")
        .fetch_one(&pool)
        .await
        .expect("auth state");
        assert_eq!(row.get::<i64, _>("failed_login_attempts"), 0);
        assert!(row.get::<Option<String>, _>("locked_until").is_none());
        assert!(row.get::<Option<String>, _>("last_login_at").is_some());
    }

    #[tokio::test]
    async fn login_with_wrong_password_returns_401_and_increments_counter() {
        let pool = test_pool().await;
        insert_user(
            &pool,
            "user-2",
            "Bob",
            "bob@example.com",
            "s3cret",
            Some("2026-03-12 00:00:00"),
            1,
            None,
        )
        .await;
        let app = test_app(pool.clone()).await;

        let response = app
            .oneshot(
                Request::post("/api/v1/auth/login")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "email": "bob@example.com",
                            "password": "wrong"
                        })
                        .to_string(),
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let body = body_json(response).await;
        assert_eq!(body["error"]["message"], "Invalid credentials");

        let row = sqlx::query("SELECT failed_login_attempts FROM auth_users WHERE user_id = ?1")
            .bind("user-2")
            .fetch_one(&pool)
            .await
            .expect("auth state");
        assert_eq!(row.get::<i64, _>("failed_login_attempts"), 2);
    }

    #[tokio::test]
    async fn locked_account_returns_429() {
        let pool = test_pool().await;
        insert_user(
            &pool,
            "user-3",
            "Carol",
            "carol@example.com",
            "open-sesame",
            Some("2026-03-12 00:00:00"),
            4,
            None,
        )
        .await;
        let app = test_app(pool.clone()).await;

        let first_response = app
            .clone()
            .oneshot(
                Request::post("/api/v1/auth/login")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "email": "carol@example.com",
                            "password": "wrong"
                        })
                        .to_string(),
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(first_response.status(), StatusCode::TOO_MANY_REQUESTS);
        let first_body = body_json(first_response).await;
        assert_eq!(
            first_body["error"]["message"],
            "Account locked, please try again in 15 minutes"
        );

        let second_response = app
            .oneshot(
                Request::post("/api/v1/auth/login")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "email": "carol@example.com",
                            "password": "open-sesame"
                        })
                        .to_string(),
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(second_response.status(), StatusCode::TOO_MANY_REQUESTS);
        let row = sqlx::query(
            "SELECT failed_login_attempts, locked_until FROM auth_users WHERE user_id = ?1",
        )
        .bind("user-3")
        .fetch_one(&pool)
        .await
        .expect("auth state");
        assert_eq!(row.get::<i64, _>("failed_login_attempts"), 5);
        assert!(row.get::<Option<String>, _>("locked_until").is_some());
    }

    async fn test_app(pool: SqlitePool) -> Router {
        let auth = AuthConfig {
            jwt: JwtConfig {
                issuer: "rara".to_owned(),
                audience: "rara-web".to_owned(),
                secret: Some("test-secret".to_owned()),
                access_token_ttl: Duration::from_secs(3600),
            },
            password: PasswordAuthConfig {
                max_attempts: 5,
                require_email_verification: true,
                lockout_window: Duration::from_secs(15 * 60),
            },
        };
        let service = AuthService::new(pool, auth);
        let (router, _) = crate::auth::routes(service).split_for_parts();
        router
    }

    async fn test_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("sqlite pool");
        sqlx::migrate!("../../rara-model/migrations")
            .run(&pool)
            .await
            .expect("migrations");
        pool
    }

    async fn insert_user(
        pool: &SqlitePool,
        user_id: &str,
        name: &str,
        email: &str,
        password: &str,
        email_verified_at: Option<&str>,
        failed_login_attempts: i64,
        locked_until: Option<&str>,
    ) {
        sqlx::query(
            r#"
            INSERT INTO kernel_users (id, name, role, permissions, enabled)
            VALUES (?1, ?2, 2, '[]', 1)
            "#,
        )
        .bind(user_id)
        .bind(name)
        .execute(pool)
        .await
        .expect("insert kernel user");

        sqlx::query(
            r#"
            INSERT INTO auth_users (
                user_id,
                email,
                password_hash,
                email_verified_at,
                failed_login_attempts,
                locked_until
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#,
        )
        .bind(user_id)
        .bind(email)
        .bind(hash(password, 4).expect("password hash"))
        .bind(email_verified_at)
        .bind(failed_login_attempts)
        .bind(locked_until)
        .execute(pool)
        .await
        .expect("insert auth user");
    }

    async fn body_json(response: axum::response::Response) -> Value {
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        serde_json::from_slice(&body).expect("json body")
    }
}
