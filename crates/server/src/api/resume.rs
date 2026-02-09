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
use job_domain_resume::{
    repository::ResumeRepository,
    types::{CreateResumeRequest, Resume, ResumeFilter, UpdateResumeRequest},
};
use uuid::Uuid;

use crate::{api::error::ApiError, state::AppState};

/// Register all resume routes on a new router with shared state.
pub fn resume_routes<R: ResumeRepository + 'static>(state: Arc<AppState<R>>) -> Router {
    Router::new()
        .route("/api/v1/resumes", post(create_resume::<R>))
        .route("/api/v1/resumes", get(list_resumes::<R>))
        .route("/api/v1/resumes/{id}", get(get_resume::<R>))
        .route("/api/v1/resumes/{id}", put(update_resume::<R>))
        .route("/api/v1/resumes/{id}", delete(delete_resume::<R>))
        .with_state(state)
}

/// POST /api/v1/resumes
async fn create_resume<R: ResumeRepository + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Json(req): Json<CreateResumeRequest>,
) -> Result<(StatusCode, Json<Resume>), ApiError> {
    let resume = state.resume_service.create(req).await?;
    Ok((StatusCode::CREATED, Json(resume)))
}

/// GET /api/v1/resumes
async fn list_resumes<R: ResumeRepository + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Query(filter): Query<ResumeFilter>,
) -> Result<Json<Vec<Resume>>, ApiError> {
    let resumes = state.resume_service.list(filter).await?;
    Ok(Json(resumes))
}

/// GET /api/v1/resumes/:id
async fn get_resume<R: ResumeRepository + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Path(id): Path<Uuid>,
) -> Result<Json<Resume>, ApiError> {
    let resume = state
        .resume_service
        .get(id)
        .await?
        .ok_or_else(|| ApiError {
            status:  StatusCode::NOT_FOUND,
            message: format!("resume not found: {id}"),
        })?;
    Ok(Json(resume))
}

/// PUT /api/v1/resumes/:id
async fn update_resume<R: ResumeRepository + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateResumeRequest>,
) -> Result<Json<Resume>, ApiError> {
    let resume = state.resume_service.update(id, req).await?;
    Ok(Json(resume))
}

/// DELETE /api/v1/resumes/:id
async fn delete_resume<R: ResumeRepository + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    state.resume_service.delete(id).await?;
    Ok(StatusCode::NO_CONTENT)
}
