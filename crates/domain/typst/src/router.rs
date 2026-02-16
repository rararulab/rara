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

//! HTTP API routes for the Typst domain.
//!
//! ## Route table
//!
//! | Method   | Path                                          | Description              |
//! |----------|-----------------------------------------------|--------------------------|
//! | `POST`   | `/api/v1/typst/projects`                      | Register a local project |
//! | `GET`    | `/api/v1/typst/projects`                      | List projects            |
//! | `GET`    | `/api/v1/typst/projects/{id}`                 | Get project details      |
//! | `DELETE` | `/api/v1/typst/projects/{id}`                 | Delete a project         |
//! | `POST`   | `/api/v1/typst/projects/import-git`           | Import from Git URL      |
//! | `POST`   | `/api/v1/typst/projects/{id}/git-sync`        | Sync Git remote updates  |
//! | `GET`    | `/api/v1/typst/projects/{id}/files`           | List project file tree   |
//! | `GET`    | `/api/v1/typst/projects/{id}/files/{path}`    | Read file content        |
//! | `PUT`    | `/api/v1/typst/projects/{id}/files/{path}`    | Write file content       |
//! | `POST`   | `/api/v1/typst/projects/{id}/compile`         | Compile project to PDF   |
//! | `GET`    | `/api/v1/typst/projects/{id}/renders`         | List render history      |
//! | `GET`    | `/api/v1/typst/renders/{id}/pdf`              | Download rendered PDF    |
//! | `GET`    | `/api/v1/typst/projects/{id}/recipes`         | List just recipes        |
//! | `POST`   | `/api/v1/typst/projects/{id}/run`             | Run recipe or command    |

use axum::{
    Json, Router,
    body::Body,
    extract::{Path, State},
    http::{StatusCode, header},
    response::IntoResponse,
    routing::get,
};
use serde::Deserialize;
use tracing::instrument;
use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::{
    error::TypstError,
    fs::FileEntry,
    runner::{JustRecipe, RunOutput},
    service::TypstService,
    types::{
        CompileRequest, ImportGitRequest, RegisterProjectRequest, RenderResult, TypstProject,
        UpdateFileRequest,
    },
};

/// Build an axum [`Router`] with all Typst endpoints.
pub fn routes(service: TypstService) -> OpenApiRouter {
    project_routes(service.clone())
        .merge(file_routes(service.clone()))
        .merge(compile_routes(service.clone()))
        .merge(runner_routes(service))
}

/// Build Typst OpenAPI documentation without constructing OpenApiRouter.
pub fn openapi_doc() -> utoipa::openapi::OpenApi {
    #[derive(OpenApi)]
    #[openapi(
        paths(
            register_project,
            list_projects,
            get_project,
            delete_project,
            import_from_git,
            sync_git,
            list_files,
            read_file,
            write_file,
            compile_project,
            list_renders,
            get_render_pdf,
            list_recipes,
            run_project_command
        )
    )]
    struct TypstApiDoc;

    TypstApiDoc::openapi()
}

/// Build an axum [`Router`] for Typst endpoints without OpenAPI metadata.
pub fn plain_routes(service: TypstService) -> Router {
    Router::new()
        .route(
            "/api/v1/typst/projects",
            get(list_projects).post(register_project),
        )
        .route(
            "/api/v1/typst/projects/{id}",
            get(get_project).delete(delete_project),
        )
        .route(
            "/api/v1/typst/projects/import-git",
            axum::routing::post(import_from_git),
        )
        .route(
            "/api/v1/typst/projects/{id}/git-sync",
            axum::routing::post(sync_git),
        )
        .route("/api/v1/typst/projects/{id}/files", get(list_files))
        .route(
            "/api/v1/typst/projects/{id}/files/{*path}",
            get(read_file).put(write_file),
        )
        .route(
            "/api/v1/typst/projects/{id}/compile",
            axum::routing::post(compile_project),
        )
        .route("/api/v1/typst/projects/{id}/renders", get(list_renders))
        .route("/api/v1/typst/renders/{id}/pdf", get(get_render_pdf))
        .route("/api/v1/typst/projects/{id}/recipes", get(list_recipes))
        .route(
            "/api/v1/typst/projects/{id}/run",
            axum::routing::post(run_project_command),
        )
        .with_state(service)
}

fn project_routes(service: TypstService) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(register_project, list_projects))
        .routes(routes!(get_project, delete_project))
        .routes(routes!(import_from_git))
        .routes(routes!(sync_git))
        .with_state(service)
}

fn file_routes(service: TypstService) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_files))
        .route(
            "/api/v1/typst/projects/{id}/files/{*path}",
            get(read_file).put(write_file),
        )
        .with_state(service)
}

fn compile_routes(service: TypstService) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(compile_project))
        .routes(routes!(list_renders))
        .routes(routes!(get_render_pdf))
        .with_state(service)
}

fn runner_routes(service: TypstService) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_recipes))
        .routes(routes!(run_project_command))
        .with_state(service)
}

// ---------------------------------------------------------------------------
// Project handlers
// ---------------------------------------------------------------------------

#[utoipa::path(
    post,
    path = "/api/v1/typst/projects",
    tag = "typst",
    request_body = RegisterProjectRequest,
    responses(
        (status = 201, description = "Project registered", body = TypstProject),
    )
)]
#[instrument(skip(service, req))]
async fn register_project(
    State(service): State<TypstService>,
    Json(req): Json<RegisterProjectRequest>,
) -> Result<(StatusCode, Json<TypstProject>), TypstError> {
    let project = service.register_project(req).await?;
    Ok((StatusCode::CREATED, Json(project)))
}

#[utoipa::path(
    get,
    path = "/api/v1/typst/projects",
    tag = "typst",
    responses(
        (status = 200, description = "List of projects", body = Vec<TypstProject>),
    )
)]
#[instrument(skip(service))]
async fn list_projects(
    State(service): State<TypstService>,
) -> Result<Json<Vec<TypstProject>>, TypstError> {
    let projects = service.list_projects().await?;
    Ok(Json(projects))
}

#[utoipa::path(
    get,
    path = "/api/v1/typst/projects/{id}",
    tag = "typst",
    params(("id" = Uuid, Path, description = "Project ID")),
    responses(
        (status = 200, description = "Project found", body = TypstProject),
    )
)]
#[instrument(skip(service))]
async fn get_project(
    State(service): State<TypstService>,
    Path(id): Path<Uuid>,
) -> Result<Json<TypstProject>, TypstError> {
    let project = service.get_project(id).await?;
    Ok(Json(project))
}

#[utoipa::path(
    delete,
    path = "/api/v1/typst/projects/{id}",
    tag = "typst",
    params(("id" = Uuid, Path, description = "Project ID")),
    responses(
        (status = 204, description = "Project deleted"),
    )
)]
#[instrument(skip(service))]
async fn delete_project(
    State(service): State<TypstService>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, TypstError> {
    service.delete_project(id).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Git import handlers
// ---------------------------------------------------------------------------

#[utoipa::path(
    post,
    path = "/api/v1/typst/projects/import-git",
    tag = "typst",
    request_body = ImportGitRequest,
    responses(
        (status = 201, description = "Project imported from Git", body = TypstProject),
    )
)]
#[instrument(skip(service, req))]
async fn import_from_git(
    State(service): State<TypstService>,
    Json(req): Json<ImportGitRequest>,
) -> Result<(StatusCode, Json<TypstProject>), TypstError> {
    let project = service.import_from_git(req).await?;
    Ok((StatusCode::CREATED, Json(project)))
}

#[utoipa::path(
    post,
    path = "/api/v1/typst/projects/{id}/git-sync",
    tag = "typst",
    params(("id" = Uuid, Path, description = "Project ID")),
    responses(
        (status = 200, description = "Git sync completed", body = TypstProject),
    )
)]
#[instrument(skip(service))]
async fn sync_git(
    State(service): State<TypstService>,
    Path(id): Path<Uuid>,
) -> Result<Json<TypstProject>, TypstError> {
    let project = service.sync_git(id).await?;
    Ok(Json(project))
}

// ---------------------------------------------------------------------------
// File handlers (local filesystem)
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/api/v1/typst/projects/{id}/files",
    tag = "typst",
    params(("id" = Uuid, Path, description = "Project ID")),
    responses(
        (status = 200, description = "File tree", body = Vec<FileEntry>),
    )
)]
#[instrument(skip(service))]
async fn list_files(
    State(service): State<TypstService>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<FileEntry>>, TypstError> {
    let project = service.get_project(id).await?;
    let entries = service.list_files(&project)?;
    Ok(Json(entries))
}

#[utoipa::path(
    get,
    path = "/api/v1/typst/projects/{id}/files/{path}",
    tag = "typst",
    params(
        ("id" = Uuid, Path, description = "Project ID"),
        ("path" = String, Path, description = "File path relative to project root"),
    ),
    responses(
        (status = 200, description = "File content", body = FileContent),
    )
)]
#[instrument(skip(service))]
async fn read_file(
    State(service): State<TypstService>,
    Path((id, path)): Path<(Uuid, String)>,
) -> Result<Json<FileContent>, TypstError> {
    let project = service.get_project(id).await?;
    let content = service.read_file(&project, &path)?;
    Ok(Json(FileContent { path, content }))
}

#[utoipa::path(
    put,
    path = "/api/v1/typst/projects/{id}/files/{path}",
    tag = "typst",
    params(
        ("id" = Uuid, Path, description = "Project ID"),
        ("path" = String, Path, description = "File path relative to project root"),
    ),
    request_body = UpdateFileRequest,
    responses(
        (status = 200, description = "File written", body = FileContent),
    )
)]
#[instrument(skip(service, req))]
async fn write_file(
    State(service): State<TypstService>,
    Path((id, path)): Path<(Uuid, String)>,
    Json(req): Json<UpdateFileRequest>,
) -> Result<Json<FileContent>, TypstError> {
    let project = service.get_project(id).await?;
    service.write_file(&project, &path, &req.content)?;
    Ok(Json(FileContent {
        path,
        content: req.content,
    }))
}

/// JSON response body for file content endpoints.
#[derive(serde::Serialize, utoipa::ToSchema)]
struct FileContent {
    path: String,
    content: String,
}

// ---------------------------------------------------------------------------
// Compile handlers
// ---------------------------------------------------------------------------

#[utoipa::path(
    post,
    path = "/api/v1/typst/projects/{id}/compile",
    tag = "typst",
    params(("id" = Uuid, Path, description = "Project ID")),
    request_body = CompileRequest,
    responses(
        (status = 200, description = "Compilation result", body = RenderResult),
    )
)]
#[instrument(skip(service))]
async fn compile_project(
    State(service): State<TypstService>,
    Path(id): Path<Uuid>,
    Json(req): Json<CompileRequest>,
) -> Result<(StatusCode, Json<RenderResult>), TypstError> {
    let render = service.compile(id, req.main_file).await?;
    Ok((StatusCode::OK, Json(render)))
}

#[utoipa::path(
    get,
    path = "/api/v1/typst/projects/{id}/renders",
    tag = "typst",
    params(("id" = Uuid, Path, description = "Project ID")),
    responses(
        (status = 200, description = "Render history", body = Vec<RenderResult>),
    )
)]
#[instrument(skip(service))]
async fn list_renders(
    State(service): State<TypstService>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<RenderResult>>, TypstError> {
    let renders = service.list_renders(id).await?;
    Ok(Json(renders))
}

#[utoipa::path(
    get,
    path = "/api/v1/typst/renders/{id}/pdf",
    tag = "typst",
    params(("id" = Uuid, Path, description = "Render ID")),
    responses(
        (status = 200, description = "Rendered PDF", content_type = "application/pdf"),
    )
)]
#[instrument(skip(service))]
async fn get_render_pdf(
    State(service): State<TypstService>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, TypstError> {
    let (pdf_bytes, _object_key) = service.get_render_pdf(id).await?;

    let headers = [
        (header::CONTENT_TYPE, "application/pdf"),
        (
            header::CONTENT_DISPOSITION,
            "inline; filename=\"render.pdf\"",
        ),
    ];

    Ok((headers, Body::from(pdf_bytes)))
}

// ---------------------------------------------------------------------------
// Runner handlers (just recipes / shell commands)
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/api/v1/typst/projects/{id}/recipes",
    tag = "typst",
    params(("id" = Uuid, Path, description = "Project ID")),
    responses(
        (status = 200, description = "Available just recipes", body = Vec<JustRecipe>),
    )
)]
#[instrument(skip(service))]
async fn list_recipes(
    State(service): State<TypstService>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<JustRecipe>>, TypstError> {
    let recipes = service.list_recipes(id).await?;
    Ok(Json(recipes))
}

/// Request body for `POST /api/v1/typst/projects/{id}/run`.
///
/// Exactly one of `recipe` or `command` must be provided.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
struct RunRequest {
    recipe: Option<String>,
    command: Option<String>,
}

#[utoipa::path(
    post,
    path = "/api/v1/typst/projects/{id}/run",
    tag = "typst",
    params(("id" = Uuid, Path, description = "Project ID")),
    request_body = RunRequest,
    responses(
        (status = 200, description = "Command output", body = RunOutput),
    )
)]
#[instrument(skip(service, req))]
async fn run_project_command(
    State(service): State<TypstService>,
    Path(id): Path<Uuid>,
    Json(req): Json<RunRequest>,
) -> Result<Json<RunOutput>, TypstError> {
    let output = match (req.recipe, req.command) {
        (Some(recipe), None) => service.run_recipe(id, &recipe).await?,
        (None, Some(command)) => service.run_command(id, &command).await?,
        _ => {
            return Err(TypstError::InvalidRequest {
                message: "exactly one of 'recipe' or 'command' must be provided".to_owned(),
            });
        }
    };
    Ok(Json(output))
}
