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

use std::{
    collections::BTreeMap,
    io::ErrorKind,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use oauth2::{
    AccessToken, EmptyExtraTokenFields, RefreshToken, Scope, TokenResponse, basic::BasicTokenType,
};
use rara_keyring_store::{KeyringStore, KeyringStoreRef};
use rmcp::transport::auth::{AuthorizationManager, OAuthTokenResponse};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tracing::warn;

// ── Constants ───────────────────────────────────────────────────────────

/// Keyring service name for storing OAuth tokens. The "account" is the
/// store key computed from `(server_name, url)`.
const KEYRING_SERVICE: &str = "rara-mcp-oauth";

/// Name of the fallback credentials file inside the config directory.
const CREDENTIALS_FILENAME: &str = ".credentials.json";

// ── OAuthPersistor ──────────────────────────────────────────────────────

/// Persists MCP OAuth tokens across sessions. Holds a reference to the
/// authorization manager so it can snapshot fresh tokens whenever they rotate.
#[derive(Clone)]
pub(crate) struct OAuthPersistor {
    inner: Arc<OAuthPersistorInner>,
}

impl OAuthPersistor {
    pub(crate) fn new<S: AsRef<str>>(
        server_name: S,
        url: S,
        authorization_manager: Arc<Mutex<AuthorizationManager>>,
        store_mode: OAuthCredentialsStoreMode,
        store: KeyringStoreRef,
        initial_credentials: Option<StoredOAuthTokens>,
    ) -> Self {
        Self {
            inner: Arc::new(OAuthPersistorInner {
                server_name: server_name.as_ref().to_owned(),
                url: url.as_ref().to_owned(),
                authorization_manager,
                store_mode,
                store,
                last_credentials: Mutex::new(initial_credentials),
            }),
        }
    }

    /// Snapshot the current credentials from the authorization manager and
    /// persist them if they differ from the last-saved copy. When the server
    /// has revoked credentials (returns `None`), any previously stored tokens
    /// are deleted.
    pub(crate) async fn persist_if_needed(&self) -> Result<()> {
        // Read the latest credentials from the authorization manager.
        let (client_id, maybe_credentials) = {
            let manager = self.inner.authorization_manager.clone();
            let guard = manager.lock().await;
            guard.get_credentials().await
        }?;

        match maybe_credentials {
            Some(credentials) => {
                self.persist_credentials(client_id, credentials).await?;
            }
            None => {
                self.remove_stale_credentials().await;
            }
        }

        Ok(())
    }

    /// Build a `StoredOAuthTokens` from fresh credentials and save it if it
    /// differs from the cached copy.
    async fn persist_credentials(
        &self,
        client_id: String,
        credentials: OAuthTokenResponse,
    ) -> Result<()> {
        let mut last = self.inner.last_credentials.lock().await;

        let new_response = WrappedOAuthTokenResponse(credentials.clone());
        let token_unchanged = last
            .as_ref()
            .is_some_and(|prev| prev.token_response == new_response);

        // Re-use the cached `expires_at` when the token hasn't rotated;
        // otherwise compute it from the fresh `expires_in` field.
        let expires_at = if token_unchanged {
            last.as_ref().and_then(|prev| prev.expires_at)
        } else {
            compute_expires_at_millis(&credentials)
        };

        let stored = StoredOAuthTokens {
            server_name: self.inner.server_name.clone(),
            url: self.inner.url.clone(),
            client_id,
            token_response: new_response,
            expires_at,
        };

        if last.as_ref() != Some(&stored) {
            stored
                .save(self.inner.store_mode, &*self.inner.store)
                .await?;
            *last = Some(stored);
        }

        Ok(())
    }

    /// Remove previously stored credentials when the server has revoked them.
    /// Logs a warning on failure rather than propagating the error.
    async fn remove_stale_credentials(&self) {
        let mut last = self.inner.last_credentials.lock().await;
        if last.take().is_some() {
            if let Err(error) = StoredOAuthTokens::delete(
                &self.inner.server_name,
                &self.inner.url,
                self.inner.store_mode,
                &*self.inner.store,
            )
            .await
            {
                warn!(
                    "failed to remove OAuth tokens for server {}: {error}",
                    self.inner.server_name
                );
            }
        }
    }

    pub(crate) async fn refresh_if_needed(&self) -> Result<()> {
        const REFRESH_SKEW_MILLIS: u64 = 30_000;
        fn token_needs_refresh(expires_at: Option<u64>) -> bool {
            let Some(expires_at) = expires_at else {
                return false;
            };

            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_else(|_| Duration::from_secs(0))
                .as_millis() as u64;

            now.saturating_add(REFRESH_SKEW_MILLIS) >= expires_at
        }

        let expires_at = {
            let guard = self.inner.last_credentials.lock().await;
            guard.as_ref().and_then(|tokens| tokens.expires_at)
        };

        if !token_needs_refresh(expires_at) {
            return Ok(());
        }

        {
            let manager = self.inner.authorization_manager.clone();
            let guard = manager.lock().await;
            guard.refresh_token().await.with_context(|| {
                format!(
                    "failed to refresh OAuth tokens for server {}",
                    self.inner.server_name
                )
            })?;
        }

        self.persist_if_needed().await
    }
}

struct OAuthPersistorInner {
    server_name:           String,
    url:                   String,
    authorization_manager: Arc<Mutex<AuthorizationManager>>,
    store_mode:            OAuthCredentialsStoreMode,
    store:                 KeyringStoreRef,
    last_credentials:      Mutex<Option<StoredOAuthTokens>>,
}

// ── OAuthCredentialsStoreMode ───────────────────────────────────────────

/// Where rara stores and reads MCP OAuth credentials.
#[derive(Debug, Default, Copy, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum OAuthCredentialsStoreMode {
    /// Try the OS keyring first; fall back to a JSON file if the keyring is
    /// unavailable.
    #[default]
    Auto,
    /// Always use `<config_dir>/.credentials.json`. Readable by any process
    /// running as the same OS user.
    File,
    /// Always use the OS keyring. Fail if the keyring is unavailable.
    Keyring,
}

// ── WrappedOAuthTokenResponse ───────────────────────────────────────────

/// Newtype around `OAuthTokenResponse` so we can implement `PartialEq` via
/// JSON comparison (the upstream type does not implement it).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WrappedOAuthTokenResponse(pub OAuthTokenResponse);

impl PartialEq for WrappedOAuthTokenResponse {
    fn eq(&self, other: &Self) -> bool {
        // Round-trip through serde_json::Value. If serialization fails the
        // tokens were never valid, so we return false rather than panicking.
        let Ok(lhs) = serde_json::to_value(&self.0) else {
            return false;
        };
        let Ok(rhs) = serde_json::to_value(&other.0) else {
            return false;
        };
        lhs == rhs
    }
}

// ── StoredOAuthTokens ───────────────────────────────────────────────────

/// A complete set of OAuth tokens stored for one MCP server, together with
/// the metadata needed to match them back to the right server.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StoredOAuthTokens {
    pub server_name:    String,
    pub url:            String,
    pub client_id:      String,
    pub token_response: WrappedOAuthTokenResponse,
    #[serde(default)]
    pub expires_at:     Option<u64>,
}

impl StoredOAuthTokens {
    /// Load previously-stored OAuth tokens for the given MCP server.
    ///
    /// The `store_mode` decides where to look:
    /// - `Auto`    — try keyring first, fall back to credentials file.
    /// - `File`    — only look in `<config_dir>/.credentials.json`.
    /// - `Keyring` — only look in the OS keyring (fail if unavailable).
    #[tracing::instrument(level = "debug")]
    pub(crate) async fn load(
        server_name: &str,
        url: &str,
        store_mode: OAuthCredentialsStoreMode,
        store: &dyn KeyringStore,
    ) -> Result<Option<Self>> {
        match store_mode {
            OAuthCredentialsStoreMode::Auto => {
                // Prefer the credential store; silently fall back to the file
                // store when the credential store is unavailable or returns
                // nothing.
                match Self::load_from_store(server_name, url, store).await {
                    Ok(Some(tokens)) => Ok(Some(tokens)),
                    Ok(None) | Err(_) => Self::load_from_file(server_name, url),
                }
            }
            OAuthCredentialsStoreMode::File => Self::load_from_file(server_name, url),
            OAuthCredentialsStoreMode::Keyring => Self::load_from_store(server_name, url, store)
                .await
                .context("failed to read OAuth tokens from credential store"),
        }
    }

    /// Load tokens from the credential store. The credential is stored as a
    /// JSON blob keyed by `(KEYRING_SERVICE, store_key)`.
    async fn load_from_store(
        server_name: &str,
        url: &str,
        store: &dyn KeyringStore,
    ) -> Result<Option<Self>> {
        let account = Self::store_key(server_name, url)?;

        let Some(json) = store
            .load(KEYRING_SERVICE, &account)
            .await
            .context("credential store load failed")?
        else {
            return Ok(None);
        };

        let mut tokens: Self =
            serde_json::from_str(&json).context("failed to parse OAuth tokens from store")?;
        tokens.refresh_expires_in();
        Ok(Some(tokens))
    }

    /// Load tokens from `<config_dir>/.credentials.json`.
    fn load_from_file(server_name: &str, url: &str) -> Result<Option<Self>> {
        let Some(file) = CredentialsFile::read()? else {
            return Ok(None);
        };
        file.find(server_name, url)
    }

    /// Compute a deterministic store key for a `(server_name, url)` pair.
    ///
    /// Format: `"<server_name>|<sha256_prefix>"` where the SHA-256 input is a
    /// canonical JSON object `{"headers":{},"type":"http","url":"..."}`. The
    /// hash is truncated to 16 hex chars — enough to avoid collisions while
    /// keeping keyring account names readable.
    fn store_key(server_name: &str, url: &str) -> Result<String> {
        use sha2::{Digest, Sha256};

        let payload = serde_json::json!({
            "type": "http",
            "url": url,
            "headers": {}
        });
        let serialized =
            serde_json::to_string(&payload).context("failed to serialize store key payload")?;

        let hash = Sha256::digest(serialized.as_bytes());
        let prefix = &format!("{hash:x}")[..16];
        Ok(format!("{server_name}|{prefix}"))
    }

    /// If `expires_at` is set (milliseconds since epoch), convert it into a
    /// relative `expires_in` duration on the token response so downstream
    /// code can use standard expiry checks.
    fn refresh_expires_in(&mut self) {
        let Some(expires_at_ms) = self.expires_at else {
            return;
        };
        let duration = remaining_seconds(expires_at_ms).map(Duration::from_secs);
        self.token_response.0.set_expires_in(duration.as_ref());
    }

    // ── save ────────────────────────────────────────────────────────────

    /// Persist these tokens according to `store_mode`.
    ///
    /// # Arguments
    ///
    /// * `store_mode` — Where to write: keyring, file, or auto (try keyring
    ///   first, fall back to file).
    #[tracing::instrument(skip(self, store), fields(server = %self.server_name), level = "debug")]
    pub(crate) async fn save(
        &self,
        store_mode: OAuthCredentialsStoreMode,
        store: &dyn KeyringStore,
    ) -> Result<()> {
        match store_mode {
            OAuthCredentialsStoreMode::Auto => match self.save_to_store(store).await {
                Ok(()) => Ok(()),
                Err(_) => self.save_to_file(),
            },
            OAuthCredentialsStoreMode::File => self.save_to_file(),
            OAuthCredentialsStoreMode::Keyring => self
                .save_to_store(store)
                .await
                .context("failed to save OAuth tokens to credential store"),
        }
    }

    /// Serialize and store these tokens in the credential store.
    async fn save_to_store(&self, store: &dyn KeyringStore) -> Result<()> {
        let account = Self::store_key(&self.server_name, &self.url)?;
        let json = serde_json::to_string(self).context("failed to serialize OAuth tokens")?;
        store
            .save(KEYRING_SERVICE, &account, &json)
            .await
            .context("credential store save failed")
    }

    /// Upsert these tokens into the credentials file on disk.
    fn save_to_file(&self) -> Result<()> {
        let key = Self::store_key(&self.server_name, &self.url)?;
        let mut file = CredentialsFile::read_or_default()?;
        file.0.insert(key, FileTokenEntry::from_stored(self));
        file.write()
    }

    // ── delete ──────────────────────────────────────────────────────────

    /// Remove previously-stored tokens for the given server.
    ///
    /// # Arguments
    ///
    /// * `server_name` — Human-readable server identifier.
    /// * `url`         — The MCP server endpoint URL.
    /// * `store_mode`  — Where to delete from: keyring, file, or auto (both).
    #[tracing::instrument(level = "debug")]
    pub(crate) async fn delete(
        server_name: &str,
        url: &str,
        store_mode: OAuthCredentialsStoreMode,
        store: &dyn KeyringStore,
    ) -> Result<()> {
        match store_mode {
            OAuthCredentialsStoreMode::Auto => {
                // Best-effort removal from both stores.
                let _ = Self::delete_from_store(server_name, url, store).await;
                Self::delete_from_file(server_name, url)
            }
            OAuthCredentialsStoreMode::File => Self::delete_from_file(server_name, url),
            OAuthCredentialsStoreMode::Keyring => Self::delete_from_store(server_name, url, store)
                .await
                .context("failed to delete OAuth tokens from credential store"),
        }
    }

    /// Delete tokens from the credential store. Returns `Ok(())` regardless
    /// of whether an entry was actually present.
    async fn delete_from_store(
        server_name: &str,
        url: &str,
        store: &dyn KeyringStore,
    ) -> Result<()> {
        let account = Self::store_key(server_name, url)?;
        store
            .delete(KEYRING_SERVICE, &account)
            .await
            .context("credential store delete failed")?;
        Ok(())
    }

    /// Remove the matching entry from the credentials file on disk.
    fn delete_from_file(server_name: &str, url: &str) -> Result<()> {
        let key = Self::store_key(server_name, url)?;
        let Some(mut file) = CredentialsFile::read()? else {
            return Ok(());
        };
        file.0.remove(&key);
        file.write()
    }
}

// ── CredentialsFile ─────────────────────────────────────────────────────

/// On-disk credentials store: a JSON object mapping arbitrary keys to token
/// entries. Lives at `<config_dir>/.credentials.json`.
#[derive(Debug, Serialize, Deserialize)]
struct CredentialsFile(BTreeMap<String, FileTokenEntry>);

impl CredentialsFile {
    /// Read and parse the credentials file. Returns `Ok(None)` when the file
    /// does not exist (normal on first run).
    fn read() -> Result<Option<Self>> {
        let path = rara_paths::config_dir().join(CREDENTIALS_FILENAME);

        let contents = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
            Err(e) => {
                return Err(e).context(format!(
                    "failed to read credentials file at {}",
                    path.display()
                ));
            }
        };

        let file: Self = serde_json::from_str(&contents).context(format!(
            "failed to parse credentials file at {}",
            path.display()
        ))?;
        Ok(Some(file))
    }

    /// Read the credentials file, or return an empty store if the file does
    /// not exist yet.
    fn read_or_default() -> Result<Self> {
        Ok(Self::read()?.unwrap_or_else(|| Self(BTreeMap::new())))
    }

    /// Serialize and write the credentials file to disk. Creates the parent
    /// directory if it doesn't exist.
    fn write(&self) -> Result<()> {
        let path = rara_paths::config_dir().join(CREDENTIALS_FILENAME);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("failed to create config directory at {}", parent.display())
            })?;
        }
        let json = serde_json::to_string_pretty(self).context("failed to serialize credentials")?;
        std::fs::write(&path, json)
            .with_context(|| format!("failed to write credentials file at {}", path.display()))
    }

    /// Find the first entry whose store key matches the given server, and
    /// convert it into a `StoredOAuthTokens`.
    fn find(self, server_name: &str, url: &str) -> Result<Option<StoredOAuthTokens>> {
        let target_key = StoredOAuthTokens::store_key(server_name, url)?;

        for entry in self.0.into_values() {
            let entry_key = StoredOAuthTokens::store_key(&entry.server_name, &entry.server_url)?;
            if entry_key == target_key {
                return Ok(Some(entry.into()));
            }
        }
        Ok(None)
    }
}

/// One entry in the `.credentials.json` fallback file.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct FileTokenEntry {
    server_name:   String,
    server_url:    String,
    client_id:     String,
    access_token:  String,
    #[serde(default)]
    expires_at:    Option<u64>,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    scopes:        Vec<String>,
}

impl FileTokenEntry {
    /// Build a file entry from an in-memory `StoredOAuthTokens`. This is the
    /// inverse of the `From<FileTokenEntry> for StoredOAuthTokens` conversion.
    fn from_stored(tokens: &StoredOAuthTokens) -> Self {
        let response = &tokens.token_response.0;
        Self {
            server_name:   tokens.server_name.clone(),
            server_url:    tokens.url.clone(),
            client_id:     tokens.client_id.clone(),
            access_token:  response.access_token().secret().to_string(),
            expires_at:    tokens
                .expires_at
                .or_else(|| compute_expires_at_millis(response)),
            refresh_token: response.refresh_token().map(|t| t.secret().to_string()),
            scopes:        response
                .scopes()
                .map(|s| s.iter().map(|s| s.to_string()).collect())
                .unwrap_or_default(),
        }
    }
}

impl From<FileTokenEntry> for StoredOAuthTokens {
    fn from(entry: FileTokenEntry) -> Self {
        let mut response = OAuthTokenResponse::new(
            AccessToken::new(entry.access_token),
            BasicTokenType::Bearer,
            EmptyExtraTokenFields {},
        );

        if let Some(refresh) = entry.refresh_token {
            response.set_refresh_token(Some(RefreshToken::new(refresh)));
        }
        if !entry.scopes.is_empty() {
            response.set_scopes(Some(entry.scopes.into_iter().map(Scope::new).collect()));
        }

        let mut tokens = StoredOAuthTokens {
            server_name:    entry.server_name,
            url:            entry.server_url,
            client_id:      entry.client_id,
            token_response: WrappedOAuthTokenResponse(response),
            expires_at:     entry.expires_at,
        };
        tokens.refresh_expires_in();
        tokens
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────

/// Compute the absolute expiry timestamp (milliseconds since UNIX epoch) from
/// the `expires_in` field of a token response. Returns `None` if no expiry
/// duration is set on the response.
fn compute_expires_at_millis(response: &OAuthTokenResponse) -> Option<u64> {
    let duration = response.expires_in()?;
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    Some(now_ms + duration.as_millis() as u64)
}

/// Compute how many whole seconds remain until `expires_at_ms` (milliseconds
/// since UNIX epoch). Returns `None` if the timestamp is already in the past.
fn remaining_seconds(expires_at_ms: u64) -> Option<u64> {
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    expires_at_ms
        .checked_sub(now_ms)
        .filter(|&diff| diff > 0)
        .map(|diff| diff / 1000)
}
