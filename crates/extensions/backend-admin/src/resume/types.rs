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

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use snafu::Snafu;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Domain model
// ---------------------------------------------------------------------------

/// A resume project backed by a GitHub repository.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ResumeProject {
    pub id:             Uuid,
    pub name:           String,
    pub git_url:        String,
    pub local_path:     String,
    #[schema(value_type = Option<String>)]
    pub last_synced_at: Option<Timestamp>,
    #[schema(value_type = String)]
    pub created_at:     Timestamp,
    #[schema(value_type = String)]
    pub updated_at:     Timestamp,
}

// ---------------------------------------------------------------------------
// Requests
// ---------------------------------------------------------------------------

/// Request to set up a new resume project.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct SetupResumeProjectRequest {
    pub name:    String,
    pub git_url: String,
}

/// Request to update a resume project.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateResumeProjectRequest {
    pub name: Option<String>,
}

// ---------------------------------------------------------------------------
// DB row
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ResumeProjectRow {
    pub id:             Uuid,
    pub name:           String,
    pub git_url:        String,
    pub local_path:     String,
    pub last_synced_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at:     chrono::DateTime<chrono::Utc>,
    pub updated_at:     chrono::DateTime<chrono::Utc>,
}

impl From<ResumeProjectRow> for ResumeProject {
    fn from(row: ResumeProjectRow) -> Self {
        Self {
            id:             row.id,
            name:           row.name,
            git_url:        row.git_url,
            local_path:     row.local_path,
            last_synced_at: row
                .last_synced_at
                .map(|t| rara_domain_shared::convert::chrono_to_timestamp(t)),
            created_at:     rara_domain_shared::convert::chrono_to_timestamp(row.created_at),
            updated_at:     rara_domain_shared::convert::chrono_to_timestamp(row.updated_at),
        }
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Snafu)]
pub enum ResumeError {
    #[snafu(display("resume project not found"))]
    NotFound,

    #[snafu(display("resume project already exists"))]
    AlreadyExists,

    #[snafu(display("invalid git URL: {url}"))]
    InvalidGitUrl { url: String },

    #[snafu(display("git operation failed: {message}"))]
    GitFailed { message: String },

    #[snafu(display("repository error: {source}"))]
    Repository { source: sqlx::Error },
}

impl axum::response::IntoResponse for ResumeError {
    fn into_response(self) -> axum::response::Response {
        let (status, msg) = match &self {
            Self::NotFound => (axum::http::StatusCode::NOT_FOUND, self.to_string()),
            Self::AlreadyExists => (axum::http::StatusCode::CONFLICT, self.to_string()),
            Self::InvalidGitUrl { .. } => (axum::http::StatusCode::BAD_REQUEST, self.to_string()),
            Self::GitFailed { .. } => (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                self.to_string(),
            ),
            Self::Repository { .. } => (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                self.to_string(),
            ),
        };
        (status, axum::Json(serde_json::json!({ "error": msg }))).into_response()
    }
}
