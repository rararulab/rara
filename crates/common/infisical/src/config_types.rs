#[derive(Debug, Clone, serde::Deserialize)]
pub struct InfisicalConfig {
    /// Infisical instance base URL.
    pub base_url: String,
    /// Universal Auth client ID.
    pub client_id: String,
    /// Universal Auth client secret.
    pub client_secret: String,
    /// Infisical project ID.
    pub project_id: String,
    /// Environment slug (e.g. "dev", "staging", "prod").
    pub environment: String,
    /// Secret path inside the project (e.g. "/").
    pub secret_path: String,
}

impl InfisicalConfig {
    /// Try to load from environment variables.
    ///
    /// Returns `None` if `INFISICAL_CLIENT_ID` is not set (opt-in).
    /// Uses sensible defaults for `base_url`, `environment`, and
    /// `secret_path` when the corresponding env vars are absent.
    #[must_use]
    pub fn from_env() -> Option<Self> {
        let client_id = std::env::var("INFISICAL_CLIENT_ID").ok()?;
        let client_secret = std::env::var("INFISICAL_CLIENT_SECRET").ok()?;
        let project_id = std::env::var("INFISICAL_PROJECT_ID").ok()?;

        let base_url = std::env::var("INFISICAL_BASE_URL")
            .unwrap_or_else(|_| "https://app.infisical.com".to_owned());
        let environment =
            std::env::var("INFISICAL_ENVIRONMENT").unwrap_or_else(|_| "dev".to_owned());
        let secret_path =
            std::env::var("INFISICAL_SECRET_PATH").unwrap_or_else(|_| "/".to_owned());

        Some(Self {
            base_url,
            client_id,
            client_secret,
            project_id,
            environment,
            secret_path,
        })
    }
}

#[cfg(test)]
#[allow(unsafe_code)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    /// Env-var mutations are process-global, so we serialize tests that
    /// touch `INFISICAL_*` variables.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// Remove all `INFISICAL_*` env vars used by the config.
    fn clear_infisical_env() {
        for key in [
            "INFISICAL_CLIENT_ID",
            "INFISICAL_CLIENT_SECRET",
            "INFISICAL_PROJECT_ID",
            "INFISICAL_BASE_URL",
            "INFISICAL_ENVIRONMENT",
            "INFISICAL_SECRET_PATH",
        ] {
            unsafe { std::env::remove_var(key) };
        }
    }

    #[test]
    fn from_env_returns_none_when_vars_missing() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_infisical_env();
        assert!(InfisicalConfig::from_env().is_none());
    }

    #[test]
    fn from_env_returns_some_with_defaults() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_infisical_env();

        unsafe {
            std::env::set_var("INFISICAL_CLIENT_ID", "test-id");
            std::env::set_var("INFISICAL_CLIENT_SECRET", "test-secret");
            std::env::set_var("INFISICAL_PROJECT_ID", "proj-123");
        }

        let cfg = InfisicalConfig::from_env().expect("should return Some");

        assert_eq!(cfg.client_id, "test-id");
        assert_eq!(cfg.client_secret, "test-secret");
        assert_eq!(cfg.project_id, "proj-123");

        // Defaults
        assert_eq!(cfg.base_url, "https://app.infisical.com");
        assert_eq!(cfg.environment, "dev");
        assert_eq!(cfg.secret_path, "/");

        clear_infisical_env();
    }

    #[test]
    fn from_env_respects_custom_values() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_infisical_env();

        unsafe {
            std::env::set_var("INFISICAL_CLIENT_ID", "cid");
            std::env::set_var("INFISICAL_CLIENT_SECRET", "csecret");
            std::env::set_var("INFISICAL_PROJECT_ID", "pid");
            std::env::set_var("INFISICAL_BASE_URL", "https://custom.infisical.io");
            std::env::set_var("INFISICAL_ENVIRONMENT", "prod");
            std::env::set_var("INFISICAL_SECRET_PATH", "/backend");
        }

        let cfg = InfisicalConfig::from_env().expect("should return Some");

        assert_eq!(cfg.base_url, "https://custom.infisical.io");
        assert_eq!(cfg.environment, "prod");
        assert_eq!(cfg.secret_path, "/backend");

        clear_infisical_env();
    }
}
