use axum::{Json, extract::State, http::StatusCode, routing::post};
use utoipa_axum::router::OpenApiRouter;

use super::{
    repository::ResumeRepository,
    service::ResumeService,
    types::{ResumeError, ResumeProject, SetupResumeProjectRequest, UpdateResumeProjectRequest},
};

struct RouteState<R: ResumeRepository> {
    service: ResumeService<R>,
}

impl<R: ResumeRepository> Clone for RouteState<R> {
    fn clone(&self) -> Self {
        Self {
            service: self.service.clone(),
        }
    }
}

pub fn routes<R: ResumeRepository + 'static>(service: ResumeService<R>) -> OpenApiRouter {
    let state = RouteState { service };
    OpenApiRouter::new()
        .route(
            "/api/v1/resume-project",
            post(setup_project::<R>)
                .get(get_project::<R>)
                .put(update_project::<R>)
                .delete(delete_project::<R>),
        )
        .route("/api/v1/resume-project/sync", post(sync_project::<R>))
        .with_state(state)
}

#[utoipa::path(
    post,
    path = "/api/v1/resume-project",
    tag = "resume",
    request_body = SetupResumeProjectRequest,
    responses(
        (status = 201, description = "Project created", body = ResumeProject),
    )
)]
async fn setup_project<R: ResumeRepository + 'static>(
    State(state): State<RouteState<R>>,
    Json(req): Json<SetupResumeProjectRequest>,
) -> Result<(StatusCode, Json<ResumeProject>), ResumeError> {
    let project = state.service.setup(req).await?;
    Ok((StatusCode::CREATED, Json(project)))
}

#[utoipa::path(
    get,
    path = "/api/v1/resume-project",
    tag = "resume",
    responses(
        (status = 200, description = "Current project", body = Option<ResumeProject>),
    )
)]
async fn get_project<R: ResumeRepository + 'static>(
    State(state): State<RouteState<R>>,
) -> Result<Json<Option<ResumeProject>>, ResumeError> {
    let project = state.service.get().await?;
    Ok(Json(project))
}

#[utoipa::path(
    put,
    path = "/api/v1/resume-project",
    tag = "resume",
    request_body = UpdateResumeProjectRequest,
    responses(
        (status = 200, description = "Updated", body = ResumeProject),
    )
)]
async fn update_project<R: ResumeRepository + 'static>(
    State(state): State<RouteState<R>>,
    Json(req): Json<UpdateResumeProjectRequest>,
) -> Result<Json<ResumeProject>, ResumeError> {
    let name = req.name.as_deref().unwrap_or("");
    if name.is_empty() {
        return Err(ResumeError::InvalidGitUrl {
            url: "name cannot be empty".into(),
        });
    }
    let project = state.service.update_name(name).await?;
    Ok(Json(project))
}

#[utoipa::path(
    delete,
    path = "/api/v1/resume-project",
    tag = "resume",
    responses(
        (status = 204, description = "Deleted"),
    )
)]
async fn delete_project<R: ResumeRepository + 'static>(
    State(state): State<RouteState<R>>,
) -> Result<StatusCode, ResumeError> {
    state.service.delete().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/api/v1/resume-project/sync",
    tag = "resume",
    responses(
        (status = 200, description = "Synced", body = ResumeProject),
    )
)]
async fn sync_project<R: ResumeRepository + 'static>(
    State(state): State<RouteState<R>>,
) -> Result<Json<ResumeProject>, ResumeError> {
    let project = state.service.sync().await?;
    Ok(Json(project))
}
