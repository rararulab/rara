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

//! `rara-vault` — HashiCorp Vault KV v2 client with AppRole authentication.
//!
//! This crate provides [`VaultClient`] for reading and writing secrets
//! stored in a Vault KV v2 backend, authenticating via the AppRole method.
//!
//! # Quick start
//!
//! ```rust,no_run
//! use rara_vault::{VaultClient, VaultConfig};
//!
//! # async fn example() -> Result<(), rara_vault::VaultError> {
//! let config: VaultConfig = todo!("load from YAML");
//! let client = VaultClient::new(config)?;
//! client.login().await?;
//!
//! // Pull all secrets as flat key-value pairs
//! let pairs = client.pull_all().await?;
//! for (key, value) in &pairs {
//!     println!("{key} = {value}");
//! }
//! # Ok(())
//! # }
//! ```

pub mod client;
pub mod config;
pub mod error;

pub use client::VaultClient;
pub use config::{VaultAuthConfig, VaultConfig};
pub use error::VaultError;
