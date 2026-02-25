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

//! Unified error types for the memory layer.

use snafu::prelude::*;

#[derive(Debug, Snafu)]
#[snafu(visibility(pub(crate)))]
pub enum MemoryError {
    #[snafu(display("mem0 error: {message}"))]
    Mem0 { message: String },

    #[snafu(display("memos error: {message}"))]
    Memos { message: String },

    #[snafu(display("hindsight error: {message}"))]
    Hindsight { message: String },

    #[snafu(display("HTTP request failed: {source}"))]
    Http { source: reqwest::Error },

    #[snafu(display("{message}"))]
    Other { message: String },
}

pub type MemoryResult<T> = Result<T, MemoryError>;
