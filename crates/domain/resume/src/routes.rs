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

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{delete, get, post, put},
};
use tracing::instrument;
use uuid::Uuid;

use crate::{
    repository::ResumeRepository,
    service::ResumeService,
    types::{CreateResumeRequest, Resume, ResumeError, ResumeFilter, UpdateResumeRequest},
};

/// Register all resume routes on a new router with shared state.
pub fn routes<R: ResumeRepository + 'static>(service: Arc<ResumeService<R>>) -> Router {
    Router::new()
        .route("/api/v1/resumes", post(create_resume::<R>))
        .route("/api/v1/resumes", get(list_resumes::<R>))
        .route("/api/v1/resumes/{id}", get(get_resume::<R>))
        .route("/api/v1/resumes/{id}", put(update_resume::<R>))
        .route("/api/v1/resumes/{id}", delete(delete_resume::<R>))
        .with_state(service)
}

#[instrument(skip(service, req))]
async fn create_resume<R: ResumeRepository + 'static>(
    State(service): State<Arc<ResumeService<R>>>,
    Json(req): Json<CreateResumeRequest>,
) -> Result<(StatusCode, Json<Resume>), ResumeError> {
    let resume = service.create(req).await?;
    Ok((StatusCode::CREATED, Json(resume)))
}

#[instrument(skip(service))]
async fn list_resumes<R: ResumeRepository + 'static>(
    State(service): State<Arc<ResumeService<R>>>,
    Query(filter): Query<ResumeFilter>,
) -> Result<Json<Vec<Resume>>, ResumeError> {
    let resumes = service.list(filter).await?;
    Ok(Json(resumes))
}

#[instrument(skip(service))]
async fn get_resume<R: ResumeRepository + 'static>(
    State(service): State<Arc<ResumeService<R>>>,
    Path(id): Path<Uuid>,
) -> Result<Json<Resume>, ResumeError> {
    let resume = service.get(id).await?.ok_or(ResumeError::NotFound { id })?;
    Ok(Json(resume))
}

#[instrument(skip(service, req))]
async fn update_resume<R: ResumeRepository + 'static>(
    State(service): State<Arc<ResumeService<R>>>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateResumeRequest>,
) -> Result<Json<Resume>, ResumeError> {
    let resume = service.update(id, req).await?;
    Ok(Json(resume))
}

#[instrument(skip(service))]
async fn delete_resume<R: ResumeRepository + 'static>(
    State(service): State<Arc<ResumeService<R>>>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ResumeError> {
    service.delete(id).await?;
    Ok(StatusCode::NO_CONTENT)
}
