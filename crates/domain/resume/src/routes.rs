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

//! HTTP API routes for resume management.

use axum::{
    Json,
    extract::{Multipart, Path, Query, State},
    http::{StatusCode, header},
    response::IntoResponse,
    routing::{delete, get, post, put},
};
use opendal::Operator;
use tracing::instrument;
use utoipa_axum::router::OpenApiRouter;
use uuid::Uuid;

use crate::{
    repository::ResumeRepository,
    service::ResumeService,
    types::{CreateResumeRequest, Resume, ResumeError, ResumeFilter, UpdateResumeRequest},
};

/// Shared state for resume routes that need object storage access.
struct ResumeRouteState<R: ResumeRepository> {
    service:      ResumeService<R>,
    object_store: Operator,
}

impl<R: ResumeRepository> Clone for ResumeRouteState<R> {
    fn clone(&self) -> Self {
        Self {
            service:      self.service.clone(),
            object_store: self.object_store.clone(),
        }
    }
}

/// Register all resume routes on a new router with shared state.
pub fn routes<R: ResumeRepository + 'static>(
    service: ResumeService<R>,
    object_store: Operator,
) -> OpenApiRouter {
    let state = ResumeRouteState {
        service,
        object_store,
    };
    OpenApiRouter::new()
        .route("/api/v1/resumes", post(create_resume::<R>))
        .route("/api/v1/resumes", get(list_resumes::<R>))
        .route("/api/v1/resumes/upload", post(upload_pdf::<R>))
        .route("/api/v1/resumes/{id}", get(get_resume::<R>))
        .route("/api/v1/resumes/{id}", put(update_resume::<R>))
        .route("/api/v1/resumes/{id}", delete(delete_resume::<R>))
        .route("/api/v1/resumes/{id}/pdf", get(download_pdf::<R>))
        .with_state(state)
}

#[utoipa::path(
    post,
    path = "/api/v1/resumes",
    tag = "resumes",
    request_body = CreateResumeRequest,
    responses(
        (status = 201, description = "Resume created", body = Resume),
    )
)]
#[instrument(skip(state, req))]
async fn create_resume<R: ResumeRepository + 'static>(
    State(state): State<ResumeRouteState<R>>,
    Json(req): Json<CreateResumeRequest>,
) -> Result<(StatusCode, Json<Resume>), ResumeError> {
    let resume = state.service.create(req).await?;
    Ok((StatusCode::CREATED, Json(resume)))
}

#[utoipa::path(
    get,
    path = "/api/v1/resumes",
    tag = "resumes",
    responses(
        (status = 200, description = "List of resumes", body = Vec<Resume>),
    )
)]
#[instrument(skip(state))]
async fn list_resumes<R: ResumeRepository + 'static>(
    State(state): State<ResumeRouteState<R>>,
    Query(filter): Query<ResumeFilter>,
) -> Result<Json<Vec<Resume>>, ResumeError> {
    let resumes = state.service.list(filter).await?;
    Ok(Json(resumes))
}

#[utoipa::path(
    get,
    path = "/api/v1/resumes/{id}",
    tag = "resumes",
    params(("id" = Uuid, Path, description = "Resume ID")),
    responses(
        (status = 200, description = "Resume found", body = Resume),
        (status = 404, description = "Resume not found"),
    )
)]
#[instrument(skip(state))]
async fn get_resume<R: ResumeRepository + 'static>(
    State(state): State<ResumeRouteState<R>>,
    Path(id): Path<Uuid>,
) -> Result<Json<Resume>, ResumeError> {
    let resume = state
        .service
        .get(id)
        .await?
        .ok_or(ResumeError::NotFound { id })?;
    Ok(Json(resume))
}

#[utoipa::path(
    put,
    path = "/api/v1/resumes/{id}",
    tag = "resumes",
    params(("id" = Uuid, Path, description = "Resume ID")),
    request_body = UpdateResumeRequest,
    responses(
        (status = 200, description = "Resume updated", body = Resume),
    )
)]
#[instrument(skip(state, req))]
async fn update_resume<R: ResumeRepository + 'static>(
    State(state): State<ResumeRouteState<R>>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateResumeRequest>,
) -> Result<Json<Resume>, ResumeError> {
    let resume = state.service.update(id, req).await?;
    Ok(Json(resume))
}

#[utoipa::path(
    delete,
    path = "/api/v1/resumes/{id}",
    tag = "resumes",
    params(("id" = Uuid, Path, description = "Resume ID")),
    responses(
        (status = 204, description = "Resume deleted"),
    )
)]
#[instrument(skip(state))]
async fn delete_resume<R: ResumeRepository + 'static>(
    State(state): State<ResumeRouteState<R>>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ResumeError> {
    state.service.delete(id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Upload a PDF file as a new resume.
///
/// Expects `multipart/form-data` with fields:
/// - `title` (text): resume title
/// - `tags` (text, optional): comma-separated tags
/// - `file` (file): the PDF file
#[utoipa::path(
    post,
    path = "/api/v1/resumes/upload",
    tag = "resumes",
    description = "Upload a PDF resume via multipart form. Fields: title (text), tags (text, comma-separated), file (PDF)",
    responses(
        (status = 201, description = "Resume created from uploaded PDF", body = Resume),
    )
)]
#[instrument(skip(state, multipart))]
async fn upload_pdf<R: ResumeRepository + 'static>(
    State(state): State<ResumeRouteState<R>>,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<Resume>), ResumeError> {
    let mut title: Option<String> = None;
    let mut tags: Vec<String> = Vec::new();
    let mut file_data: Option<bytes::Bytes> = None;

    while let Some(field) =
        multipart
            .next_field()
            .await
            .map_err(|e| ResumeError::InvalidContent {
                reason: format!("failed to read multipart field: {e}"),
            })?
    {
        let name = field.name().unwrap_or("").to_owned();
        match name.as_str() {
            "title" => {
                title = Some(
                    field
                        .text()
                        .await
                        .map_err(|e| ResumeError::InvalidContent {
                            reason: format!("failed to read title field: {e}"),
                        })?,
                );
            }
            "tags" => {
                let raw = field
                    .text()
                    .await
                    .map_err(|e| ResumeError::InvalidContent {
                        reason: format!("failed to read tags field: {e}"),
                    })?;
                tags = raw
                    .split(',')
                    .map(|s| s.trim().to_owned())
                    .filter(|s| !s.is_empty())
                    .collect();
            }
            "file" => {
                // Validate content type if provided.
                if let Some(ct) = field.content_type() {
                    if ct != "application/pdf" {
                        return Err(ResumeError::InvalidFileType {
                            content_type: ct.to_owned(),
                        });
                    }
                }
                file_data = Some(
                    field
                        .bytes()
                        .await
                        .map_err(|e| ResumeError::InvalidContent {
                            reason: format!("failed to read file data: {e}"),
                        })?,
                );
            }
            _ => {
                // Ignore unknown fields.
            }
        }
    }

    let title = title.ok_or_else(|| ResumeError::InvalidContent {
        reason: "missing required field: title".to_owned(),
    })?;

    let pdf_data = file_data.ok_or_else(|| ResumeError::InvalidContent {
        reason: "missing required field: file".to_owned(),
    })?;

    let resume = state
        .service
        .upload_pdf(title, tags, pdf_data, &state.object_store)
        .await?;

    Ok((StatusCode::CREATED, Json(resume)))
}

/// Download the PDF associated with a resume.
#[utoipa::path(
    get,
    path = "/api/v1/resumes/{id}/pdf",
    tag = "resumes",
    params(("id" = Uuid, Path, description = "Resume ID")),
    responses(
        (status = 200, description = "Resume PDF", content_type = "application/pdf"),
    )
)]
#[instrument(skip(state))]
async fn download_pdf<R: ResumeRepository + 'static>(
    State(state): State<ResumeRouteState<R>>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ResumeError> {
    let data = state.service.get_pdf(id, &state.object_store).await?;

    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/pdf"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"resume.pdf\"",
            ),
        ],
        data,
    ))
}
