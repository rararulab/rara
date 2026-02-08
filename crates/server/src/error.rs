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

use std::any::Any;

use axum::{Json, response::IntoResponse};
use rsketch_error::{ErrorExt, StackError, StatusCode};
use serde::Serialize;
use snafu::Snafu;
use strum::EnumProperty;

#[derive(Debug, Serialize)]
pub struct ErrorBody {
    pub code:    StatusCode,
    pub message: String,
}

#[derive(Debug, Snafu, strum_macros::EnumProperty)]
#[snafu(visibility(pub))]
pub enum ApiError {
    #[snafu(display("Invalid argument: {reason}"))]
    #[strum(props(status_code = "invalid_argument"))]
    InvalidArgument { reason: String },

    #[snafu(display("Not found: {resource}"))]
    #[strum(props(status_code = "not_found"))]
    NotFound { resource: String },

    #[snafu(display("Unauthorized"))]
    #[strum(props(status_code = "unauthorized"))]
    Unauthorized,

    #[snafu(display("Forbidden"))]
    #[strum(props(status_code = "forbidden"))]
    Forbidden,

    #[snafu(display("Conflict: {reason}"))]
    #[strum(props(status_code = "conflict"))]
    Conflict { reason: String },

    #[snafu(display("Internal error"))]
    #[strum(props(status_code = "internal"))]
    Internal,
}

impl ErrorExt for ApiError {
    fn status_code(&self) -> StatusCode {
        self.get_str("status_code")
            .and_then(|value| value.parse().ok())
            .unwrap_or(StatusCode::Unknown)
    }

    fn as_any(&self) -> &dyn Any { self as _ }
}

impl StackError for ApiError {
    fn debug_fmt(&self, layer: usize, buf: &mut Vec<String>) {
        buf.push(format!("{layer}: {self}"));
    }

    fn next(&self) -> Option<&dyn StackError> { None }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let body = Json(ErrorBody {
            code:    self.status_code(),
            message: self.output_msg(),
        });
        (self.status_code().http_status(), body).into_response()
    }
}

impl From<ApiError> for tonic::Status {
    fn from(error: ApiError) -> Self {
        Self::new(error.status_code().tonic_code(), error.output_msg())
    }
}

pub type ApiResult<T> = std::result::Result<T, ApiError>;
