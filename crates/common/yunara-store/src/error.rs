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

use snafu::Snafu;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Snafu, Debug)]
#[snafu(visibility(pub))]
pub enum Error {
    #[snafu(transparent)]
    Sqlx {
        source: sqlx::Error,
        #[snafu(implicit)]
        loc:    snafu::Location,
    },

    #[snafu(transparent)]
    Migration {
        source: sqlx::migrate::MigrateError,
        #[snafu(implicit)]
        loc:    snafu::Location,
    },

    #[snafu(display("Failed to encode/decode value"))]
    Codec {
        source: serde_json::Error,
        #[snafu(implicit)]
        loc:    snafu::Location,
    },

    #[snafu(display("Invalid time configuration: {message}"))]
    InvalidTimeConfig { message: String },

    #[snafu(display("Failed to build diesel-async connection pool"))]
    BuildDieselPool {
        source: diesel_async::pooled_connection::PoolError,
        #[snafu(implicit)]
        loc:    snafu::Location,
    },

    #[snafu(display("Failed to acquire diesel-async connection from pool"))]
    DieselPoolRun {
        source: bb8::RunError<diesel_async::pooled_connection::PoolError>,
        #[snafu(implicit)]
        loc:    snafu::Location,
    },

    #[snafu(display("Diesel query failed"))]
    Diesel {
        source: diesel::result::Error,
        #[snafu(implicit)]
        loc:    snafu::Location,
    },
}
