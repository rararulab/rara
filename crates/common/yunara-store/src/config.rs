// Copyright 2025 Crrow
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

use std::time::Duration;

use serde::Deserialize;
use smart_default::SmartDefault;

/// Database configuration
#[derive(Debug, Clone, SmartDefault, bon::Builder, Deserialize)]
#[serde(default)]
#[builder(on(String, into), on(Duration, into))]
pub struct DatabaseConfig {
    /// PostgreSQL database URL, e.g. `postgres://user:pass@host:5432/dbname`
    #[default(_code = "\"postgres://postgres:postgres@localhost:5432/job\".to_string()")]
    #[builder(default = "postgres://postgres:postgres@localhost:5432/job", getter)]
    pub database_url: String,

    /// Maximum number of connections in the pool
    #[default = 10]
    #[builder(default = 10, getter)]
    pub max_connections: u32,

    /// Minimum number of idle connections
    #[default = 1]
    #[builder(default = 1, getter)]
    pub min_connections: u32,

    /// Connection timeout (default: 30 seconds)
    #[default(_code = "Duration::from_secs(30)")]
    #[builder(default = Duration::from_secs(30), getter)]
    #[serde(with = "humantime_serde")]
    pub connect_timeout: Duration,

    /// Maximum lifetime of a connection (default: 30 minutes)
    #[default(_code = "Some(Duration::from_secs(1800))")]
    #[builder(getter)]
    #[serde(with = "humantime_serde::option")]
    pub max_lifetime: Option<Duration>,

    /// Idle timeout for connections (default: 10 minutes)
    #[default(_code = "Some(Duration::from_secs(600))")]
    #[builder(getter)]
    #[serde(with = "humantime_serde::option")]
    pub idle_timeout: Option<Duration>,
}
