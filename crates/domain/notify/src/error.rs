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

//! Error types for the notification domain.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NotifyError {
    NotFound { id: Uuid },
    SendFailed { channel: String, message: String },
    RepositoryError { message: String },
    ValidationError { message: String },
    RetryExhausted { id: Uuid, attempts: i32 },
}

impl std::fmt::Display for NotifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound { id } => write!(f, "notification not found: {id}"),
            Self::SendFailed { channel, message } => {
                write!(f, "send failed on {channel}: {message}")
            }
            Self::RepositoryError { message } => write!(f, "repository error: {message}"),
            Self::ValidationError { message } => write!(f, "validation error: {message}"),
            Self::RetryExhausted { id, attempts } => {
                write!(f, "retry exhausted for {id} after {attempts} attempts")
            }
        }
    }
}

impl std::error::Error for NotifyError {}
