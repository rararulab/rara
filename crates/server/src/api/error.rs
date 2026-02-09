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

//! Unified API error type that maps domain errors to HTTP status codes.

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use job_domain_application::error::ApplicationError;
use job_domain_interview::error::InterviewError;
use job_domain_notify::error::NotifyError;
use job_domain_resume::types::ResumeError;
use job_domain_scheduler::error::SchedulerError;

/// A structured API error returned as a JSON body with the appropriate
/// HTTP status code.
pub struct ApiError {
    /// The HTTP status code to return.
    pub status:  StatusCode,
    /// A human-readable error message.
    pub message: String,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = serde_json::json!({
            "error": {
                "status": self.status.as_u16(),
                "message": self.message,
            }
        });
        (self.status, axum::Json(body)).into_response()
    }
}

impl From<ApplicationError> for ApiError {
    fn from(err: ApplicationError) -> Self {
        let status = match &err {
            ApplicationError::NotFound { .. } => StatusCode::NOT_FOUND,
            ApplicationError::ValidationError { .. } => StatusCode::BAD_REQUEST,
            ApplicationError::InvalidTransition { .. } => StatusCode::CONFLICT,
            ApplicationError::DuplicateApplication { .. } => StatusCode::CONFLICT,
            ApplicationError::RepositoryError { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        };
        Self {
            status,
            message: err.to_string(),
        }
    }
}

impl From<InterviewError> for ApiError {
    fn from(err: InterviewError) -> Self {
        let status = match &err {
            InterviewError::NotFound { .. } => StatusCode::NOT_FOUND,
            InterviewError::ValidationError { .. } => StatusCode::BAD_REQUEST,
            InterviewError::InvalidStatusTransition { .. } => StatusCode::CONFLICT,
            InterviewError::RepositoryError { .. } => StatusCode::INTERNAL_SERVER_ERROR,
            InterviewError::PrepGenerationFailed { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        };
        Self {
            status,
            message: err.to_string(),
        }
    }
}

impl From<ResumeError> for ApiError {
    fn from(err: ResumeError) -> Self {
        let status = match &err {
            ResumeError::NotFound { .. } => StatusCode::NOT_FOUND,
            ResumeError::InvalidContent { .. } => StatusCode::BAD_REQUEST,
            ResumeError::ParentNotFound { .. } => StatusCode::BAD_REQUEST,
            ResumeError::DuplicateContent { .. } => StatusCode::CONFLICT,
            ResumeError::Storage { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        };
        Self {
            status,
            message: err.to_string(),
        }
    }
}

impl From<NotifyError> for ApiError {
    fn from(err: NotifyError) -> Self {
        let status = match &err {
            NotifyError::NotFound { .. } => StatusCode::NOT_FOUND,
            NotifyError::ValidationError { .. } => StatusCode::BAD_REQUEST,
            NotifyError::RetryExhausted { .. } => StatusCode::CONFLICT,
            NotifyError::SendFailed { .. } => StatusCode::BAD_GATEWAY,
            NotifyError::RepositoryError { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        };
        Self {
            status,
            message: err.to_string(),
        }
    }
}

impl From<SchedulerError> for ApiError {
    fn from(err: SchedulerError) -> Self {
        let status = match &err {
            SchedulerError::NotFound { .. } => StatusCode::NOT_FOUND,
            SchedulerError::NotFoundByName { .. } => StatusCode::NOT_FOUND,
            SchedulerError::InvalidCronExpression { .. } => StatusCode::BAD_REQUEST,
            SchedulerError::TaskDisabled { .. } => StatusCode::BAD_REQUEST,
            SchedulerError::TaskExecutionFailed { .. } => StatusCode::INTERNAL_SERVER_ERROR,
            SchedulerError::RepositoryError { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        };
        Self {
            status,
            message: err.to_string(),
        }
    }
}
