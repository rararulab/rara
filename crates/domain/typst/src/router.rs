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

use axum::{
    Json, Router,
    body::Body,
    extract::{Path, State},
    http::{StatusCode, header},
    response::IntoResponse,
    routing::{delete, get, post, put},
};
use tracing::instrument;
use uuid::Uuid;

use crate::{
    error::TypstError,
    fs::FileEntry,
    service::TypstService,
    types::{
        CompileRequest, ImportGitRequest, RegisterProjectRequest, RenderResult, TypstProject,
        UpdateFileRequest,
    },
};

/// Build an axum [`Router`] with all Typst endpoints.
pub fn routes(service: TypstService) -> Router {
    Router::new()
        // Projects
        .route("/api/v1/typst/projects", post(register_project))
        .route("/api/v1/typst/projects", get(list_projects))
        .route("/api/v1/typst/projects/{id}", get(get_project))
        .route("/api/v1/typst/projects/{id}", delete(delete_project))
        // Git import & sync
        .route(
            "/api/v1/typst/projects/import-git",
            post(import_from_git),
        )
        .route(
            "/api/v1/typst/projects/{id}/git-sync",
            post(sync_git),
        )
        // Files (local filesystem)
        .route("/api/v1/typst/projects/{id}/files", get(list_files))
        .route(
            "/api/v1/typst/projects/{id}/files/{*path}",
            get(read_file),
        )
        .route(
            "/api/v1/typst/projects/{id}/files/{*path}",
            put(write_file),
        )
        // Compile
        .route(
            "/api/v1/typst/projects/{id}/compile",
            post(compile_project),
        )
        // Renders
        .route(
            "/api/v1/typst/projects/{id}/renders",
            get(list_renders),
        )
        .route("/api/v1/typst/renders/{id}/pdf", get(get_render_pdf))
        .with_state(service)
}

// ---------------------------------------------------------------------------
// Project handlers
// ---------------------------------------------------------------------------

#[instrument(skip(service, req))]
async fn register_project(
    State(service): State<TypstService>,
    Json(req): Json<RegisterProjectRequest>,
) -> Result<(StatusCode, Json<TypstProject>), TypstError> {
    let project = service.register_project(req).await?;
    Ok((StatusCode::CREATED, Json(project)))
}

#[instrument(skip(service))]
async fn list_projects(
    State(service): State<TypstService>,
) -> Result<Json<Vec<TypstProject>>, TypstError> {
    let projects = service.list_projects().await?;
    Ok(Json(projects))
}

#[instrument(skip(service))]
async fn get_project(
    State(service): State<TypstService>,
    Path(id): Path<Uuid>,
) -> Result<Json<TypstProject>, TypstError> {
    let project = service.get_project(id).await?;
    Ok(Json(project))
}

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

#[instrument(skip(service, req))]
async fn import_from_git(
    State(service): State<TypstService>,
    Json(req): Json<ImportGitRequest>,
) -> Result<(StatusCode, Json<TypstProject>), TypstError> {
    let project = service.import_from_git(req).await?;
    Ok((StatusCode::CREATED, Json(project)))
}

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

#[instrument(skip(service))]
async fn list_files(
    State(service): State<TypstService>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<FileEntry>>, TypstError> {
    let project = service.get_project(id).await?;
    let entries = service.list_files(&project)?;
    Ok(Json(entries))
}

#[instrument(skip(service))]
async fn read_file(
    State(service): State<TypstService>,
    Path((id, path)): Path<(Uuid, String)>,
) -> Result<Json<FileContent>, TypstError> {
    let project = service.get_project(id).await?;
    let content = service.read_file(&project, &path)?;
    Ok(Json(FileContent { path, content }))
}

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
#[derive(serde::Serialize)]
struct FileContent {
    path:    String,
    content: String,
}

// ---------------------------------------------------------------------------
// Compile handlers
// ---------------------------------------------------------------------------

#[instrument(skip(service))]
async fn compile_project(
    State(service): State<TypstService>,
    Path(id): Path<Uuid>,
    Json(req): Json<CompileRequest>,
) -> Result<(StatusCode, Json<RenderResult>), TypstError> {
    let render = service.compile(id, req.main_file).await?;
    Ok((StatusCode::OK, Json(render)))
}

#[instrument(skip(service))]
async fn list_renders(
    State(service): State<TypstService>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<RenderResult>>, TypstError> {
    let renders = service.list_renders(id).await?;
    Ok(Json(renders))
}

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
