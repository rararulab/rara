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

pub mod config;
pub mod db;
pub mod diesel_pool;
pub mod error;
pub mod kv;

pub use config::DatabaseConfig;
pub use db::DBStore;
pub use diesel_pool::{
    DieselPoolConfig, DieselPoolInitError, DieselPoolRunError, DieselSqliteConnection,
    DieselSqlitePool, DieselSqlitePools, build_sqlite_pools,
};
pub use error::{Error, Result};
pub use kv::KVStore;
