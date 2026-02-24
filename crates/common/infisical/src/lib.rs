pub mod config;
pub mod error;

use snafu::ResultExt;

/// Load secrets from Infisical and inject them as environment variables.
///
/// Secrets from Infisical will **not** override already-set env vars.
/// Returns the count of newly injected variables.
///
/// # Safety concern
///
/// Uses `std::env::set_var` which is unsafe in Rust 2024 edition.
/// This function must be called early at startup, before spawning threads
/// that may read environment variables concurrently.
#[allow(unsafe_code)]
pub async fn load_secrets_to_env(
    config: &config::InfisicalConfig,
) -> Result<usize, error::InfisicalError> {
    let mut client = infisical::Client::builder()
        .base_url(&config.base_url)
        .build()
        .await
        .context(error::ClientBuildSnafu)?;

    let auth = infisical::AuthMethod::new_universal_auth(
        &config.client_id,
        &config.client_secret,
    );
    client.login(auth).await.context(error::AuthSnafu)?;

    let request = infisical::secrets::ListSecretsRequest::builder(
        &config.project_id,
        &config.environment,
    )
    .path(&config.secret_path)
    .recursive(true)
    .build();

    let secrets = client
        .secrets()
        .list(request)
        .await
        .context(error::ListSecretsSnafu)?;

    let mut count = 0usize;
    for secret in &secrets {
        if std::env::var(&secret.secret_key).is_err() {
            // SAFETY: we are single-threaded at startup before any other
            // threads read env vars.
            unsafe {
                std::env::set_var(&secret.secret_key, &secret.secret_value);
            }
            tracing::debug!(key = %secret.secret_key, "injected secret from Infisical");
            count += 1;
        } else {
            tracing::trace!(
                key = %secret.secret_key,
                "skipped (already set in environment)",
            );
        }
    }

    Ok(count)
}
