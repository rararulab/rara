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

use std::{any::Any, error::Error as StdError, sync::Arc};

use http::StatusCode as HttpStatusCode;
use serde::Serialize;
use snafu::Snafu;
use strum::EnumProperty;
use tonic::Code as TonicCode;

#[derive(
    Clone,
    Copy,
    Debug,
    Eq,
    PartialEq,
    Serialize,
    strum_macros::EnumProperty,
    strum_macros::EnumString,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum StatusCode {
    #[strum(props(http_status = "400", tonic_code = "3"))]
    InvalidArgument,
    #[strum(props(http_status = "404", tonic_code = "5"))]
    NotFound,
    #[strum(props(http_status = "401", tonic_code = "16"))]
    Unauthorized,
    #[strum(props(http_status = "403", tonic_code = "7"))]
    Forbidden,
    #[strum(props(http_status = "409", tonic_code = "6"))]
    Conflict,
    #[strum(props(http_status = "500", tonic_code = "13"))]
    Internal,
    #[strum(props(http_status = "500", tonic_code = "13"))]
    Unknown,
}

impl StatusCode {
    #[must_use]
    pub fn http_status(self) -> HttpStatusCode {
        self.get_str("http_status")
            .and_then(|value| value.parse::<u16>().ok())
            .and_then(|value| HttpStatusCode::from_u16(value).ok())
            .unwrap_or(HttpStatusCode::INTERNAL_SERVER_ERROR)
    }

    #[must_use]
    pub fn tonic_code(self) -> TonicCode {
        let value = self
            .get_str("tonic_code")
            .and_then(|value| value.parse::<i32>().ok())
            .unwrap_or(TonicCode::Internal as i32);
        TonicCode::from_i32(value)
    }
}

pub trait StackError: StdError {
    fn debug_fmt(&self, layer: usize, buf: &mut Vec<String>);

    fn next(&self) -> Option<&dyn StackError>;

    fn last(&self) -> &dyn StackError
    where
        Self: Sized,
    {
        let Some(mut result) = self.next() else {
            return self;
        };
        while let Some(err) = result.next() {
            result = err;
        }
        result
    }

    fn transparent(&self) -> bool { false }
}

pub trait ErrorExt: StackError {
    fn status_code(&self) -> StatusCode { StatusCode::Unknown }

    fn as_any(&self) -> &dyn Any;

    fn output_msg(&self) -> String
    where
        Self: Sized,
    {
        match self.status_code() {
            StatusCode::Unknown | StatusCode::Internal => {
                format!("Internal error: {}", self.status_code() as u32)
            }
            _ => {
                let error = self.last();
                error.source().map_or_else(
                    || format!("{error}"),
                    |external_error| {
                        let mut root = external_error;
                        while let Some(source) = root.source() {
                            root = source;
                        }
                        if error.transparent() {
                            format!("{root}")
                        } else {
                            format!("{error}: {root}")
                        }
                    },
                )
            }
        }
    }

    fn root_cause(&self) -> Option<&dyn StdError>
    where
        Self: Sized,
    {
        let error = self.last();
        let mut source = error.source()?;
        while let Some(next) = source.source() {
            source = next;
        }
        Some(source)
    }
}

impl<T: ?Sized + StackError> StackError for Arc<T> {
    fn debug_fmt(&self, layer: usize, buf: &mut Vec<String>) {
        self.as_ref().debug_fmt(layer, buf);
    }

    fn next(&self) -> Option<&dyn StackError> { self.as_ref().next() }
}

impl<T: StackError> StackError for Box<T> {
    fn debug_fmt(&self, layer: usize, buf: &mut Vec<String>) {
        self.as_ref().debug_fmt(layer, buf);
    }

    fn next(&self) -> Option<&dyn StackError> { self.as_ref().next() }
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Snafu, Debug)]
#[snafu(visibility(pub))]
pub enum Error {
    #[snafu(transparent)]
    Network {
        source: NetworkError,
        #[snafu(implicit)]
        loc:    snafu::Location,
    },
}

#[derive(Snafu, Debug)]
#[snafu(visibility(pub))]
pub enum NetworkError {
    #[snafu(display("Failed to connect to {addr}"))]
    ConnectionError {
        addr:   String,
        #[snafu(source)]
        source: std::io::Error,
        #[snafu(implicit)]
        loc:    snafu::Location,
    },

    #[snafu(display("Failed to parse address {addr}"))]
    ParseAddressError {
        addr:   String,
        #[snafu(source)]
        source: std::net::AddrParseError,
        #[snafu(implicit)]
        loc:    snafu::Location,
    },
}
