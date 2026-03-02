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

/// Errors from the memory abstraction layer.
#[derive(Debug, Snafu)]
pub enum MemoryError {
    /// State layer backend error.
    #[snafu(display("state memory error: {message}"))]
    State { message: String },

    /// Knowledge layer backend error.
    #[snafu(display("knowledge memory error: {message}"))]
    Knowledge { message: String },

    /// Learning layer backend error.
    #[snafu(display("learning memory error: {message}"))]
    Learning { message: String },

    /// Target record does not exist.
    #[snafu(display("not found: {id}"))]
    NotFound { id: uuid::Uuid },

    /// Insufficient scope permissions.
    #[snafu(display("scope denied: {message}"))]
    ScopeDenied { message: String },
}

/// Convenience alias used by all memory trait methods.
pub type Result<T> = std::result::Result<T, MemoryError>;
