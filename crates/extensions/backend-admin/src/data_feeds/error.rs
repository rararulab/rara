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

//! Error types for the data-feed domain service.
//!
//! [`DataFeedSvcError`] is a domain-layer snafu enum. The service layer does
//! not sit at an application boundary (that is the HTTP router), so per
//! `docs/guides/rust-style.md` it must not expose `anyhow::Error`. Router
//! handlers map each variant to an RFC 9457 [`ProblemDetails`] response via
//! the conversions in this module.
//!
//! [`ProblemDetails`]: crate::kernel::problem::ProblemDetails

use diesel_async::pooled_connection::PoolError as DieselConnPoolError;
use snafu::Snafu;
use yunara_store::diesel_pool::DieselPoolRunError;

use crate::kernel::problem::ProblemDetails;

/// Result alias for the data-feed service.
pub type Result<T> = std::result::Result<T, DataFeedSvcError>;

/// Errors that can occur during data-feed persistence operations.
#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum DataFeedSvcError {
    /// Failed to acquire a connection from the diesel-async pool.
    #[snafu(display("data_feeds pool acquire failed: {source}"))]
    PoolAcquire {
        source: DieselPoolRunError<DieselConnPoolError>,
    },

    /// A diesel query against the data-feed tables failed.
    #[snafu(display("data_feeds query failed: {source}"))]
    Query { source: diesel::result::Error },

    /// Failed to serialise a feed-config field (tags / transport / auth) to
    /// JSON before persistence.
    #[snafu(display("failed to encode feed config field as JSON: {source}"))]
    EncodeJson { source: serde_json::Error },

    /// Failed to deserialise a persisted feed-config / event row back into
    /// domain types (JSON columns, enum strings, or timestamps).
    #[snafu(display("failed to decode persisted feed data: {message}"))]
    DecodeRow { message: String },
}

/// Map a service error to a `ProblemDetails` HTTP response.
///
/// All variants currently collapse to 500 Internal Server Error — the
/// service does not distinguish not-found at this layer (the router
/// performs a follow-up `get_feed` check and returns 404 itself).
impl From<DataFeedSvcError> for ProblemDetails {
    fn from(err: DataFeedSvcError) -> Self { ProblemDetails::internal(err.to_string()) }
}
