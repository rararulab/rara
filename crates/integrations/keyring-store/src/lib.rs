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

use std::{fmt::Debug, sync::Arc};

use async_trait::async_trait;
use snafu::{ResultExt, Snafu};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Snafu)]
pub enum Error {
    /// Failed to interact with the OS keyring.
    #[snafu(display("keyring error: {source}"))]
    Keyring {
        source:   keyring::Error,
        #[snafu(implicit)]
        location: snafu::Location,
    },

    /// Database error from the PostgreSQL credential store.
    #[snafu(display("database error: {source}"), visibility(pub))]
    Pg {
        source:   sqlx::Error,
        #[snafu(implicit)]
        location: snafu::Location,
    },
}

pub type KeyringStoreRef = Arc<dyn KeyringStore>;

/// Credential store backed by the OS keyring (macOS Keychain, Linux Secret
/// Service, etc.).
///
/// Each credential is addressed by a `(service, account)` pair — the same
/// key space used by the underlying `keyring` crate.
#[async_trait]
pub trait KeyringStore: Debug + Send + Sync {
    /// Load a stored credential. Returns `None` when no entry exists.
    async fn load(&self, service: &str, account: &str) -> Result<Option<String>>;

    /// Save (or overwrite) a credential.
    async fn save(&self, service: &str, account: &str, value: &str) -> Result<()>;

    /// Delete a credential. Returns `true` if an entry was removed, `false`
    /// if it did not exist.
    async fn delete(&self, service: &str, account: &str) -> Result<bool>;
}

/// Default implementation that delegates directly to the OS keyring via the
/// `keyring` crate. Each method is instrumented with `tracing` at debug level
/// so callers get structured logs for free.
#[derive(Debug)]
pub struct DefaultKeyringStore;

/// Convert a `keyring::Error` into our `Error`, filtering out `NoEntry`.
///
/// Returns `Ok(None)` for `NoEntry` (the entry simply doesn't exist) and
/// `Err` for everything else.
fn filter_no_entry(err: keyring::Error) -> Result<Option<String>> {
    match err {
        keyring::Error::NoEntry => Ok(None),
        other => Err(other).context(KeyringSnafu),
    }
}

#[async_trait]
impl KeyringStore for DefaultKeyringStore {
    #[tracing::instrument(skip(self), level = "debug")]
    async fn load(&self, service: &str, account: &str) -> Result<Option<String>> {
        let entry = keyring::Entry::new(service, account).context(KeyringSnafu)?;
        match entry.get_password() {
            Ok(password) => Ok(Some(password)),
            // NoEntry is not an error — the caller asked "is there a value?"
            // and the answer is simply "no".
            Err(err) => filter_no_entry(err),
        }
    }

    #[tracing::instrument(skip(self, value), fields(value_len = value.len()), level = "debug")]
    async fn save(&self, service: &str, account: &str, value: &str) -> Result<()> {
        let entry = keyring::Entry::new(service, account).context(KeyringSnafu)?;
        entry.set_password(value).context(KeyringSnafu)
    }

    #[tracing::instrument(skip(self), level = "debug")]
    async fn delete(&self, service: &str, account: &str) -> Result<bool> {
        let entry = keyring::Entry::new(service, account).context(KeyringSnafu)?;
        match entry.delete_credential() {
            Ok(()) => Ok(true),
            // Same as load — a missing entry just means "nothing to delete".
            Err(keyring::Error::NoEntry) => Ok(false),
            Err(other) => Err(other).context(KeyringSnafu),
        }
    }
}
